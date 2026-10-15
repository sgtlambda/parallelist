<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api, type BranchImpact, type RemovalImpact } from '../api'
import Icon from './Icon.vue'
import { WORKSPACE_LABELS, bytes, relativeTime, shortPath } from '../format'
import { projectId, state } from '../store'
import { commitsLost, impactBadges, impactSummary } from '../status'
import { useEscape } from '../useEscape'

const props = defineProps<{
  open: boolean
  /** Directories to remove. */
  paths: string[]
  /** Branches to delete: leftovers with nothing on disk. */
  branches: string[]
  title: string
}>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(() => props.open && !submitting.value, () => emit('close'))

const impacts = ref<RemovalImpact[]>([])
const branchImpacts = ref<BranchImpact[]>([])
/** Filled in after the list is already on screen; `du` is slow. */
const sizes = ref<Record<string, number>>({})
const measuring = ref(false)
const selected = ref<Set<string>>(new Set())
const selectedBranches = ref<Set<string>>(new Set())
const deleteBranches = ref(true)
const loading = ref(false)
/** Only while the tasks are being queued, which is a moment. */
const submitting = ref(false)

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) return
    const { paths, branches } = props
    selected.value = new Set()
    selectedBranches.value = new Set()
    impacts.value = []
    branchImpacts.value = []
    const id = projectId.value
    if (!id || paths.length + branches.length === 0) return
    sizes.value = {}
    loading.value = true
    try {
      const [workspaces, refs] = await Promise.all([
        paths.length ? api.removalImpacts(id, paths) : Promise.resolve([]),
        branches.length ? api.branchImpacts(id, branches) : Promise.resolve([]),
      ])
      impacts.value = workspaces
      branchImpacts.value = refs
      // One item named on its own is what the user came here for. In a
      // sweep, only the ones that cost nothing are ticked: anything still
      // holding work — or that couldn't be checked — is listed to be
      // noticed, not deleted by default.
      const singled = branches.length + paths.length === 1
      selected.value = new Set(
        workspaces
          .filter((i) => singled || impactBadges(i, deleteBranches.value).length === 0)
          .map((i) => i.path),
      )
      selectedBranches.value = new Set(
        refs.filter((r) => singled || r.commitCount === 0).map((r) => r.branch),
      )
    } finally {
      loading.value = false
    }

    if (paths.length === 0) return
    // Sizes arrive separately so the list is usable immediately.
    measuring.value = true
    try {
      const measured = await api.workspaceSizes(paths)
      // The dialog may have been closed and reopened meanwhile.
      if (props.open) sizes.value = measured
    } finally {
      measuring.value = false
    }
  },
)

function toggle(path: string) {
  const next = new Set(selected.value)
  next.has(path) ? next.delete(path) : next.add(path)
  selected.value = next
}

function toggleBranch(branch: string) {
  const next = new Set(selectedBranches.value)
  next.has(branch) ? next.delete(branch) : next.add(branch)
  selectedBranches.value = next
}

const chosen = computed(() => impacts.value.filter((i) => selected.value.has(i.path)))
const chosenBranches = computed(() =>
  branchImpacts.value.filter((b) => selectedBranches.value.has(b.branch)),
)
const chosenCount = computed(() => chosen.value.length + chosenBranches.value.length)

const totals = computed(() => ({
  size: chosen.value.reduce((sum, i) => sum + (sizes.value[i.path] ?? 0), 0),
  modified: chosen.value.reduce((sum, i) => sum + (i.status?.modified ?? 0), 0),
  unpushed: chosen.value.reduce((sum, i) => sum + commitsLost(i, deleteBranches.value), 0),
  stashes: chosen.value.reduce((sum, i) => sum + i.stashes, 0),
  stranded: chosen.value.reduce((sum, i) => sum + i.dependents.length, 0),
  unknown: chosen.value.filter((i) => i.error).length,
  commits: chosenBranches.value.reduce((sum, b) => sum + b.commitCount, 0),
}))

/** The banner's sentence, naming only what is actually at stake. */
const destroys = computed(() => {
  const t = totals.value
  const parts = [
    t.modified && `${t.modified} uncommitted change(s)`,
    t.unpushed && `${t.unpushed} unpushed commit(s)`,
    t.stashes && `${t.stashes} stash(es)`,
  ].filter(Boolean)
  if (!parts.length) return ''
  const last = parts.pop()
  return `This will destroy ${parts.length ? `${parts.join(', ')} and ${last}` : last}.`
})

/**
 * Queue a task per workspace and per branch, then get out of the way. They
 * run one at a time in the background; each reports in the task box, a
 * failure stays there until dismissed, and the list refreshes as each lands.
 */
async function remove() {
  const id = projectId.value
  if (!id || chosenCount.value === 0) return
  submitting.value = true
  try {
    if (chosen.value.length) {
      await api.removeWorkspaces(
        id,
        chosen.value.map((i) => ({
          path: i.path,
          kind: i.kind,
          // Only offered for worktrees: a clone's branch lives in its own repo
          // and disappears with it.
          deleteBranch: deleteBranches.value && i.kind === 'worktree' ? i.branch : null,
        })),
      )
    }
    if (chosenBranches.value.length) {
      await api.deleteBranches(
        id,
        chosenBranches.value.map((b) => b.branch),
      )
    }
    emit('close')
  } catch (e) {
    state.error = String(e)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <div v-if="open" class="scrim" @click.self="!submitting && emit('close')">
    <form class="modal wide" @submit.prevent="remove">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>{{ title }}</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">
          {{ chosenCount }} selected
          <template v-if="!measuring && chosen.length"> · {{ bytes(totals.size) }}</template>
        </span>
      </div>

      <div class="modal-body">
        <p v-if="loading" class="muted">Measuring…</p>
        <p v-else-if="!impacts.length && !branchImpacts.length" class="empty">
          Nothing to clean up.
        </p>

        <table v-else class="list">
          <thead>
            <tr>
              <th style="width: 24px"></th>
              <th>Directory</th>
              <th>Kind</th>
              <th>Branch</th>
              <th style="text-align: right">Size</th>
              <th>Would lose</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="impact in impacts" :key="impact.path">
              <td>
                <input
                  type="checkbox"
                  :checked="selected.has(impact.path)"
                  @change="toggle(impact.path)"
                />
              </td>
              <td class="mono truncate selectable" style="max-width: 300px" :title="impact.path">
                {{ shortPath(impact.path) }}
              </td>
              <td>
                <span class="badge kind" :title="WORKSPACE_LABELS[impact.kind]">
                  <Icon :name="impact.kind" />
                </span>
              </td>
              <td class="truncate" style="max-width: 160px">{{ impact.branch ?? '—' }}</td>
              <td style="text-align: right; font-variant-numeric: tabular-nums">
                <span v-if="sizes[impact.path] !== undefined">{{ bytes(sizes[impact.path]) }}</span>
                <span v-else class="faint">measuring…</span>
              </td>
              <td>
                <span
                  v-for="badge in impactBadges(impact, deleteBranches)"
                  :key="badge.key"
                  class="badge"
                  :class="{ warn: badge.warn }"
                  :title="badge.title"
                >
                  {{ badge.text }}
                </span>
                <span v-if="!impactBadges(impact, deleteBranches).length" class="faint">
                  {{ impactSummary(impact, deleteBranches) }}
                </span>
              </td>
            </tr>

            <!-- Branches with nothing on disk. No directory, no size: the only
                 thing at stake is the commits no other ref holds, so those are
                 spelled out under the row rather than left as a number. -->
            <template v-for="impact in branchImpacts" :key="impact.branch">
              <tr :class="{ 'no-rule': impact.commits.length }">
                <td>
                  <input
                    type="checkbox"
                    :checked="selectedBranches.has(impact.branch)"
                    @change="toggleBranch(impact.branch)"
                  />
                </td>
                <td class="faint">—</td>
                <td><span class="badge kind" title="Local branch with no workspace on disk">branch</span></td>
                <td class="truncate" style="max-width: 160px" :title="impact.branch">
                  {{ impact.branch }}
                </td>
                <td class="faint" style="text-align: right">—</td>
                <td>
                  <span v-if="impact.commitCount" class="badge warn">
                    {{ impact.commitCount }} commit{{ impact.commitCount === 1 ? '' : 's' }}
                  </span>
                  <span v-else class="faint">nothing — every commit is on origin</span>
                </td>
              </tr>
              <tr v-if="impact.commits.length" class="detail-row">
                <td></td>
                <td colspan="5">
                  <div v-for="commit in impact.commits" :key="commit.sha" class="commit">
                    <code class="faint">{{ commit.sha.slice(0, 8) }}</code>
                    <span class="truncate">{{ commit.subject }}</span>
                    <span class="age faint">{{ relativeTime(commit.committedAt) }}</span>
                  </div>
                  <div v-if="impact.commitCount > impact.commits.length" class="faint">
                    + {{ impact.commitCount - impact.commits.length }} more
                  </div>
                </td>
              </tr>
            </template>
          </tbody>
        </table>

        <label v-if="impacts.length" class="check">
          <input v-model="deleteBranches" type="checkbox" />
          <span>Also delete the local branch of removed worktrees</span>
        </label>

        <div v-if="destroys" class="banner">{{ destroys }}</div>

        <div v-if="totals.stranded" class="banner">
          {{ totals.stranded }} worktree(s) hang off a clone being removed and
          will stop working without it.
        </div>

        <div v-if="totals.unknown" class="banner">
          {{ totals.unknown }} workspace(s) couldn't be fully checked, so they may
          hold work this dialog can't see.
        </div>

        <div v-if="totals.commits" class="banner">
          {{ totals.commits }} commit(s) exist on no other branch and nowhere on
          origin. Deleting these branches is the last thing holding them.
        </div>

      </div>

      <div class="modal-footer">
        <span class="faint" style="font-size: 11.5px">
          Runs in the background, one at a time.
        </span>
        <span class="spacer" />
        <button type="button" class="btn" :disabled="submitting" @click="emit('close')">
          Cancel
        </button>
        <button
          type="submit"
          class="btn primary danger"
          :disabled="submitting || loading || !chosenCount"
        >
          {{ chosen.length ? 'Remove' : 'Delete' }} {{ chosenCount || '' }}
        </button>
      </div>
    </form>
  </div>
</template>
