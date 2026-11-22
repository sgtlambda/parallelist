<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { api } from '../api'
import TerminalMark from './TerminalMark.vue'
import { ago, basename } from '../format'
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
 * in the order the branch list ranks them, waiting first.
 */
const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(() => props.open, () => emit('close'))

const selected = ref<Set<string>>(new Set())
/** What runs in each terminal's foreground, asked for while open. */
const programs = ref<Record<string, string>>({})

interface Entry {
  path: string
  branch: string
  program: string | null
  /** What its mark says, in words. */
  doing: string
  at: number
}

interface Group {
  id: string
  name: string
  entries: Entry[]
}

function doing(path: string): string {
  const waiting = waitingOn(path)
  if (waiting === 'blocked') return 'Waiting for an answer'
  if (waiting === 'ready') return 'Finished, not replied to'
  if (statuses.get(path)?.status === 'working') return 'Working'
  return ''
}

const groups = computed<Group[]>(() => {
  const projects = state.config?.projects ?? []
  const byId = new Map<string, Group>(
    projects.map((p) => [p.id, { id: p.id, name: p.name, entries: [] }]),
  )
  for (const [path, home] of running) {
    let group = byId.get(home.projectId)
    if (!group) {
      // A project removed from the sidebar while its shells ran on.
      group = { id: home.projectId, name: home.projectId, entries: [] }
      byId.set(home.projectId, group)
    }
    group.entries.push({
      path,
      branch: home.branch,
      program: programs.value[path] ?? null,
      doing: doing(path),
      at: activeAt.get(path) ?? 0,
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
  return listed
})

const all = computed(() => groups.value.flatMap((g) => g.entries.map((e) => e.path)))
/** Selected and still running: a shell that exits while the dialog is open
 *  leaves its row, and the count, on its own. */
const chosen = computed(() => all.value.filter((p) => selected.value.has(p)))

function toggle(path: string) {
  const next = new Set(selected.value)
  next.has(path) ? next.delete(path) : next.add(path)
  selected.value = next
}

/** Tick all of these, or untick them if they all are. */
function toggleMany(paths: string[]) {
  const next = new Set(selected.value)
  const every = paths.every((p) => next.has(p))
  for (const p of paths) every ? next.delete(p) : next.add(p)
  selected.value = next
}

function checkState(paths: string[]) {
  const n = paths.filter((p) => selected.value.has(p)).length
  return { checked: n > 0 && n === paths.length, indeterminate: n > 0 && n < paths.length }
}

let poll = 0

async function loadPrograms() {
  try {
    const next = await api.terminalPrograms()
    if (props.open) programs.value = next
  } catch {
    /* the column stays as it was */
  }
}

watch(
  () => props.open,
  (isOpen) => {
    window.clearInterval(poll)
    if (!isOpen) return
    selected.value = new Set()
    programs.value = {}
    void loadPrograms()
    // A program started or ended meanwhile; one `ps` for all of them.
    poll = window.setInterval(loadPrograms, 3000)
  },
  { immediate: true },
)

onUnmounted(() => window.clearInterval(poll))

/** What ending the selection would interrupt, beyond idle shells. */
const atStake = computed(() => {
  const waiting = chosen.value.filter((p) => waitingOn(p)).length
  const working = chosen.value.filter(
    (p) => !waitingOn(p) && statuses.get(p)?.status === 'working',
  ).length
  const other = chosen.value.filter(
    (p) => !waitingOn(p) && statuses.get(p)?.status !== 'working' && programs.value[p],
  ).length
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

function end() {
  for (const path of chosen.value) closeTerminal(path)
  emit('close')
}

const plural = (n: number) => (n === 1 ? '1 terminal' : `${n} terminals`)
</script>

<template>
  <div v-if="open" class="scrim" @click.self="emit('close')">
    <div class="modal wide terminals" role="dialog" aria-labelledby="terminals-title" @keydown="onKey">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span id="terminals-title">Terminals</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">
          {{ plural(all.length) }} running<template v-if="chosen.length">
            · {{ chosen.length }} selected</template
          >
        </span>
      </div>

      <div class="modal-body">
        <p v-if="!all.length" class="empty">No terminals are running.</p>

        <table v-else class="list">
          <thead>
            <tr>
              <th style="width: 24px">
                <input
                  type="checkbox"
                  title="Select all (⌘A)"
                  v-bind="checkState(all)"
                  @change="toggleMany(all)"
                />
              </th>
              <th>Workspace</th>
              <th>Branch</th>
              <th>Running</th>
              <th>Status</th>
              <th style="text-align: right">Active</th>
            </tr>
          </thead>
          <tbody v-for="group in groups" :key="group.id">
            <tr class="group" @click="toggleMany(group.entries.map((e) => e.path))">
              <td>
                <input
                  type="checkbox"
                  :title="`Select every terminal in ${group.name}`"
                  v-bind="checkState(group.entries.map((e) => e.path))"
                  @click.stop
                  @change="toggleMany(group.entries.map((e) => e.path))"
                />
              </td>
              <td colspan="5">
                <span class="with-mark">
                  <span class="group-name">{{ group.name }}</span>
                  <span class="faint">{{ group.entries.length }}</span>
                  <TerminalMark :project-id="group.id" />
                </span>
              </td>
            </tr>
            <tr
              v-for="entry in group.entries"
              :key="entry.path"
              class="entry"
              :class="{ chosen: selected.has(entry.path) }"
              @click="toggle(entry.path)"
            >
              <td>
                <input
                  type="checkbox"
                  :checked="selected.has(entry.path)"
                  @click.stop
                  @change="toggle(entry.path)"
                />
              </td>
              <td class="mono truncate" style="max-width: 200px" :title="entry.path">
                {{ basename(entry.path) }}
              </td>
              <td class="truncate" style="max-width: 180px" :title="entry.branch">
                {{ entry.branch }}
              </td>
              <td class="mono truncate program" style="max-width: 180px" :title="entry.program ?? ''">
                <span v-if="entry.program">{{ entry.program }}</span>
                <span v-else class="faint">shell</span>
              </td>
              <td class="status">
                <span class="with-mark">
                  <TerminalMark :paths="[entry.path]" />
                  <span v-if="entry.doing">{{ entry.doing }}</span>
                </span>
              </td>
              <td class="faint" style="text-align: right; white-space: nowrap">
                {{ entry.at ? ago(entry.at / 1000) : '' }}
              </td>
            </tr>
          </tbody>
        </table>

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
          End {{ chosen.length ? plural(chosen.length) : '' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.terminals tr.group td {
  padding-top: 12px;
  border-bottom: none;
  cursor: default;
}

.terminals tbody:first-of-type tr.group td {
  padding-top: 4px;
}

.group-name {
  font-weight: 600;
}

.with-mark {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.terminals tr.entry {
  cursor: default;
}

.terminals tr.entry:hover td {
  background: var(--bg-hover);
}

.terminals tr.entry.chosen td {
  background: var(--bg-active);
}

.program {
  color: var(--text-dim);
}

.status {
  white-space: nowrap;
}

</style>
