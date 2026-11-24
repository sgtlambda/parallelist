<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { api, type OtherBuild, type TerminalInfo } from '../api'
import TerminalMark from './TerminalMark.vue'
import { ago, basename, splitWorkspaceName } from '../format'
import { state } from '../store'
import { closeTerminal } from '../terminal'
import {
  activeAt,
  byTerminalRank,
  running,
  statuses,
  terminalRank,
  waitingOn,
} from '../terminalState'
import { useEscape } from '../useEscape'

/**
 * Every running terminal, across all projects, to end any number of them at
 * once: the ones left behind by branches long finished, or all of them
 * before a quit. Grouped by project in the sidebar's order, and within one
 * in the order the branch list ranks them, waiting first. Other builds'
 * terminals follow, one group per build, since a build of a worktree whose
 * branch is merged is never launched again to end its own.
 */
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(() => props.open, () => emit('close'))

/** Keys of the ticked rows: a path for this build's terminals, and
 *  `build \n id` for another build's (`*` for the whole of an older one). */
const selected = ref<Set<string>>(new Set())
/** What runs in each terminal's foreground, asked for while open. */
const programs = ref<Record<string, string>>({})
const others = ref<OtherBuild[]>([])

type Waiting = 'blocked' | 'ready' | null

interface Entry {
  key: string
  path: string
  branch: string
  /** What runs in its foreground; only known for this build's. */
  program: string | null
  /** What its mark says, in words. */
  doing: string
  /** What it waits on, for the colour of those words. */
  waiting: Waiting
  at: number
  /** This build's, with a live mark. */
  own: boolean
  /** `<project>-<rest>` read as the rest, then the project, as the queue
   *  shows it. */
  split: { branch: string; project: string } | null
  /** An older build's whole relay, which has no terminals to list. */
  whole: boolean
  /** Another build's state as reported, for its mark; empty when unknown. */
  info: { status?: string | null; ready?: boolean }
}

interface Group {
  key: string
  name: string
  entries: Entry[]
}

const SEP = '\n'

function doingOf(info: Pick<TerminalInfo, 'status' | 'ready'>): string {
  if (info.status === 'blocked') return 'Waiting for an answer'
  if (info.ready) return 'Finished, not replied to'
  if (info.status === 'working') return 'Working'
  return ''
}

const waitingOf = (info: Pick<TerminalInfo, 'status' | 'ready'>): Waiting =>
  info.status === 'blocked' ? 'blocked' : info.ready ? 'ready' : null

/** Its status as the backend would report it, from what the marks read. */
function infoOf(path: string): Pick<TerminalInfo, 'status' | 'ready'> {
  const waiting = waitingOn(path)
  return {
    status: waiting === 'blocked' ? 'blocked' : (statuses.get(path)?.status ?? null),
    ready: waiting === 'ready',
  }
}

const projectOf = (id: string | null) => state.config?.projects.find((p) => p.id === id)

function splitName(path: string, projectId: string | null, branch: string) {
  const project = projectOf(projectId)
  return project && path ? splitWorkspaceName(path, project.path, branch) : null
}

const groups = computed<Group[]>(() => {
  const projects = state.config?.projects ?? []
  const byId = new Map<string, Group>(projects.map((p) => [p.id, { key: p.id, name: p.name, entries: [] }]))
  for (const [path, home] of running) {
    let group = byId.get(home.projectId)
    if (!group) {
      // A project removed from the sidebar while its shells ran on.
      group = { key: home.projectId, name: home.projectId, entries: [] }
      byId.set(home.projectId, group)
    }
    const info = infoOf(path)
    group.entries.push({
      key: path,
      path,
      branch: home.branch,
      program: programs.value[path] ?? null,
      doing: doingOf(info),
      waiting: waitingOf(info),
      at: activeAt.get(path) ?? 0,
      own: true,
      split: splitName(path, home.projectId, home.branch),
      whole: false,
      info: {},
    })
  }
  const listed = [...byId.values()].filter((g) => g.entries.length)
  for (const g of listed) {
    g.entries.sort(
      (a, b) =>
        byTerminalRank(terminalRank([a.path]), terminalRank([b.path])) ||
        basename(a.path).localeCompare(basename(b.path)),
    )
  }

  for (const other of others.value) {
    const entries: Entry[] = other.terminals
      ? other.terminals.map((t) => ({
          key: `${other.build}${SEP}${t.id}`,
          path: t.path,
          branch: [projectOf(t.projectId)?.name ?? t.projectId, t.branch].filter(Boolean).join(' · '),
          program: null,
          doing: doingOf(t),
          waiting: waitingOf(t),
          at: t.activeAt,
          own: false,
          split: splitName(t.path, t.projectId, t.branch),
          whole: false,
          info: { status: t.status, ready: t.ready },
        }))
      : [
          {
            key: `${other.build}${SEP}*`,
            path: '',
            branch: '',
            program: null,
            doing: '',
            waiting: null,
            at: 0,
            own: false,
            split: null,
            whole: true,
            info: {},
          },
        ]
    if (entries.length) listed.push({ key: `build${SEP}${other.build}`, name: `${other.build} build`, entries })
  }
  return listed
})

const keysOf = (group: Group) => group.entries.map((e) => e.key)

const entries = computed(() => groups.value.flatMap((g) => g.entries))
const all = computed(() => entries.value.map((e) => e.key))
/** Selected and still running: a shell that exits while the dialog is open
 *  leaves its row, and the count, on its own. */
const chosen = computed(() => entries.value.filter((e) => selected.value.has(e.key)))

/** The counts are of terminals listed one by one, this build's and other
 *  builds'. An older version's relay can't say how many it holds, so it
 *  is named instead of counted. */
const listedCount = computed(() => entries.value.filter((e) => !e.whole).length)
const chosenCount = computed(() => chosen.value.filter((e) => !e.whole).length)
const chosenWhole = computed(() => chosen.value.filter((e) => e.whole).length)

const endLabel = computed(() => {
  const older = chosenWhole.value === 1 ? "an older version's terminals" : "older versions' terminals"
  const parts = [chosenCount.value && plural(chosenCount.value), chosenWhole.value && older].filter(Boolean)
  return parts.length ? `End ${parts.join(' and ')}` : 'End'
})

function toggle(key: string) {
  const next = new Set(selected.value)
  next.has(key) ? next.delete(key) : next.add(key)
  selected.value = next
}

/** Tick all of these, or untick them if they all are. */
function toggleMany(keys: string[]) {
  const next = new Set(selected.value)
  const every = keys.every((k) => next.has(k))
  for (const k of keys) every ? next.delete(k) : next.add(k)
  selected.value = next
}

function checkState(keys: string[]) {
  const n = keys.filter((k) => selected.value.has(k)).length
  return { checked: n > 0 && n === keys.length, indeterminate: n > 0 && n < keys.length }
}

let poll = 0

async function load() {
  const [nextPrograms, nextOthers] = await Promise.all([
    api.terminalPrograms().catch(() => null),
    api.otherBuilds().catch(() => null),
  ])
  if (!props.open) return
  if (nextPrograms) programs.value = nextPrograms
  if (nextOthers) others.value = nextOthers
}

watch(
  () => props.open,
  (isOpen) => {
    window.clearInterval(poll)
    if (!isOpen) return
    selected.value = new Set()
    programs.value = {}
    others.value = []
    void load()
    // A program started or ended meanwhile; one `ps` for all of them.
    poll = window.setInterval(load, 3000)
  },
  { immediate: true },
)

onUnmounted(() => window.clearInterval(poll))

/** What ending the selection would interrupt, beyond idle shells. */
const atStake = computed(() => {
  const waiting = chosen.value.filter((e) => e.waiting).length
  const working = chosen.value.filter((e) => e.doing === 'Working').length
  const other = chosen.value.filter((e) => !e.doing && e.program).length
  const parts = [
    waiting && `${waiting} ${waiting === 1 ? 'is' : 'are'} waiting for an answer`,
    working && `${working} ${working === 1 ? 'has' : 'have'} a program working`,
    other && `${other} ${other === 1 ? 'is' : 'are'} running a program`,
  ].filter((part): part is string => !!part)
  const last = parts.pop()
  return !last ? '' : parts.length ? `${parts.join(', ')} and ${last}` : last
})

function onKey(event: KeyboardEvent) {
  if (event.metaKey && event.key === 'a') {
    event.preventDefault()
    toggleMany(all.value)
  }
}

async function end() {
  const byBuild = new Map<string, string[]>()
  for (const { key } of chosen.value) {
    const at = key.indexOf(SEP)
    if (at < 0) {
      closeTerminal(key)
      continue
    }
    const build = key.slice(0, at)
    const id = key.slice(at + 1)
    const ids = byBuild.get(build) ?? []
    // `*`: an older build's relay, ended as a whole.
    if (id !== '*') ids.push(id)
    byBuild.set(build, ids)
  }
  emit('close')
  for (const [build, ids] of byBuild) {
    try {
      await api.endOtherTerminals(build, ids)
    } catch (e) {
      state.error = String(e)
    }
  }
}

const plural = (n: number) => (n === 1 ? '1 terminal' : `${n} terminals`)
</script>

<template>
  <div v-if="open" class="scrim" @click.self="emit('close')">
    <div class="modal terminals" role="dialog" aria-labelledby="terminals-title" @keydown="onKey">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span id="terminals-title">Terminals</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">
          {{ plural(listedCount) }} running<template v-if="chosenCount">
            · {{ chosenCount }} selected</template
          >
        </span>
      </div>

      <div class="modal-body">
        <p v-if="!all.length" class="empty">No terminals are running.</p>

        <template v-else>
          <label class="select-all">
            <input type="checkbox" v-bind="checkState(all)" @change="toggleMany(all)" />
            <span>Select all</span>
            <kbd>⌘A</kbd>
          </label>

          <section v-for="group in groups" :key="group.key" class="section">
            <label class="section-head">
              <input type="checkbox" v-bind="checkState(keysOf(group))" @change="toggleMany(keysOf(group))" />
              <span>{{ group.name }}</span>
              <span v-if="!group.entries[0].whole" class="count">{{ group.entries.length }}</span>
            </label>
            <div
              v-for="entry in group.entries"
              :key="entry.key"
              class="item"
              :class="{ chosen: selected.has(entry.key) }"
              @click="toggle(entry.key)"
            >
              <input
                class="tick"
                type="checkbox"
                :checked="selected.has(entry.key)"
                @click.stop
                @change="toggle(entry.key)"
              />
              <span class="mark-slot" :class="{ unusable: entry.whole }">
                <TerminalMark v-if="entry.own" :paths="[entry.path]" />
                <TerminalMark v-else :info="entry.info" />
              </span>
              <span v-if="entry.whole" class="name truncate">Terminals from an older version</span>
              <span v-else class="name" :title="entry.path">
                <template v-if="entry.split">
                  <span class="truncate name-main">{{ entry.split.branch }}</span>
                  <span class="truncate name-project">{{ entry.split.project }}</span>
                </template>
                <span v-else class="truncate name-main">{{ basename(entry.path) }}</span>
              </span>
              <span class="age faint">{{ entry.at ? ago(entry.at / 1000) : '' }}</span>
              <span class="meta truncate">
                <template v-if="entry.whole">Can only be ended together</template>
                <template v-else>
                  <span>{{ entry.branch }}</span>
                  <template v-if="entry.program || entry.own">
                    <span class="sep">·</span>
                    <span v-if="entry.program" class="mono">{{ entry.program }}</span>
                    <span v-else>shell</span>
                  </template>
                  <template v-if="entry.doing">
                    <span class="sep">·</span>
                    <span :class="['doing', entry.waiting]">{{ entry.doing }}</span>
                  </template>
                </template>
              </span>
            </div>
          </section>
        </template>

        <div v-if="atStake" class="banner">Of the selected terminals, {{ atStake }}.</div>
      </div>

      <div class="modal-footer">
        <span class="faint" style="font-size: 11.5px">
          Ending a terminal hangs up on its shell and everything running in it.
        </span>
        <span class="spacer" />
        <button type="button" class="btn" @click="emit('close')">Cancel</button>
        <button
          type="button"
          class="btn primary danger"
          :disabled="!chosen.length"
          @click="end"
        >
          {{ endLabel }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.terminals input[type='checkbox'] {
  width: 14px;
  height: 14px;
  margin: 0;
  padding: 0;
  vertical-align: middle;
}

.select-all,
.section-head {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: default;
}

.select-all {
  padding: 0 8px 8px;
  color: var(--text-dim);
  font-size: 12px;
}

.select-all kbd {
  margin-left: auto;
  font: inherit;
  font-size: 11px;
  color: var(--text-faint);
}

.section + .section {
  margin-top: 8px;
}

.section-head {
  padding: 6px 8px 4px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-dim);
}

.count {
  color: var(--text-faint);
  font-weight: 400;
}

.item {
  display: grid;
  grid-template-columns: 16px 16px minmax(0, 1fr) auto;
  column-gap: 10px;
  row-gap: 1px;
  align-items: center;
  padding: 7px 8px;
  border-radius: 6px;
  cursor: default;
}

.item:hover {
  background: var(--bg-hover);
}

.item.chosen {
  background: var(--bg-active);
}

/* Selected rows that meet read as one block, rounded only at its ends. */
.item.chosen + .item.chosen {
  border-top-left-radius: 0;
  border-top-right-radius: 0;
}

.item.chosen:has(+ .item.chosen) {
  border-bottom-left-radius: 0;
  border-bottom-right-radius: 0;
}

.item .tick,
.mark-slot {
  grid-row: span 2;
}

.mark-slot {
  display: flex;
  justify-content: center;
}

/* An older version's terminals can't be opened from any window, only
   ended, and their state is unknown: greyed, unlike a live mark. */
.mark-slot.unusable :deep(.terminal-mark) {
  color: var(--text-faint);
}

.name {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
}

/* The part of `<project>-<rest>` that names the branch leads, and the
   project gives way first, as on a queue card. */
.name-main {
  flex: 0 1 auto;
  font-weight: 600;
}

.name-project {
  flex: 0 1000 auto;
  color: var(--text-dim);
}

.age {
  font-size: 12px;
  white-space: nowrap;
  text-align: right;
}

.meta {
  grid-column: 3 / span 2;
  font-size: 12px;
  color: var(--text-dim);
}

.meta .mono {
  font-size: 11px;
}

.sep {
  color: var(--text-faint);
  margin: 0 5px;
}

.doing.ready {
  color: var(--accent);
}

.doing.blocked {
  color: var(--orange);
}
</style>
