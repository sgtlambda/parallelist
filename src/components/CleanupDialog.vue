<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api, type RemovalImpact } from '../api'
import Icon from './Icon.vue'
import { WORKSPACE_LABELS, bytes, shortPath } from '../format'
import { refresh, projectId, staleWorkspacePaths } from '../store'
import { useEscape } from '../useEscape'

const props = defineProps<{
  open: boolean
  /** Explicit targets. When null, every stale workspace is offered. */
  paths: string[] | null
}>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(() => props.open && !working.value, () => emit('close'))

const impacts = ref<RemovalImpact[]>([])
/** Filled in after the list is already on screen; `du` is slow. */
const sizes = ref<Record<string, number>>({})
const measuring = ref(false)
const selected = ref<Set<string>>(new Set())
const deleteBranches = ref(true)
const loading = ref(false)
const working = ref(false)
const failures = ref<{ path: string; error: string }[]>([])

const staleCandidates = staleWorkspacePaths

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) return
    failures.value = []
    const targets = props.paths ?? staleCandidates.value
    selected.value = new Set(targets)
    impacts.value = []
    const id = projectId.value
    if (!id || targets.length === 0) return
    sizes.value = {}
    loading.value = true
    try {
      impacts.value = await api.removalImpacts(id, targets)
    } finally {
      loading.value = false
    }

    // Sizes arrive separately so the list is usable immediately.
    measuring.value = true
    try {
      const measured = await api.workspaceSizes(targets)
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

const chosen = computed(() => impacts.value.filter((i) => selected.value.has(i.path)))

const totals = computed(() => ({
  size: chosen.value.reduce((sum, i) => sum + (sizes.value[i.path] ?? 0), 0),
  modified: chosen.value.reduce((sum, i) => sum + i.modified, 0),
  unpushed: chosen.value.reduce((sum, i) => sum + i.unpushed, 0),
  stashes: chosen.value.reduce((sum, i) => sum + i.stashes, 0),
}))

async function remove() {
  const id = projectId.value
  if (!id || chosen.value.length === 0) return
  working.value = true
  failures.value = []
  try {
    const results = await api.removeWorkspaces(
      id,
      chosen.value.map((i) => ({
        path: i.path,
        kind: i.kind,
        // Only offered for worktrees: a clone's branch lives in its own repo
        // and disappears with it.
        deleteBranch: deleteBranches.value && i.kind === 'worktree' ? i.branch : null,
      })),
    )
    failures.value = results
      .filter((r) => r.error)
      .map((r) => ({ path: r.path, error: r.error as string }))
    await refresh()
    if (failures.value.length === 0) emit('close')
  } finally {
    working.value = false
  }
}
</script>

<template>
  <div v-if="open" class="scrim" @click.self="!working && emit('close')">
    <div class="modal wide">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>{{ paths ? 'Remove workspace' : 'Cleanup' }}</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">
          {{ chosen.length }} selected
          <template v-if="!measuring"> · {{ bytes(totals.size) }}</template>
        </span>
      </div>

      <div class="modal-body">
        <p v-if="loading" class="muted">Measuring…</p>
        <p v-else-if="!impacts.length" class="empty">
          Nothing to clean up. Merged branches, branches deleted on origin and
          leftover directories all show up here.
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
                <span v-if="impact.modified" class="badge warn">{{ impact.modified }} changed</span>
                <span v-if="impact.unpushed" class="badge warn">{{ impact.unpushed }} unpushed</span>
                <span v-if="impact.stashes" class="badge warn">{{ impact.stashes }} stashed</span>
                <span
                  v-if="!impact.modified && !impact.unpushed && !impact.stashes"
                  class="faint"
                >
                  {{ impact.untracked ? `nothing (${impact.untracked} untracked)` : 'nothing' }}
                </span>
              </td>
            </tr>
          </tbody>
        </table>

        <label v-if="impacts.length" class="check">
          <input v-model="deleteBranches" type="checkbox" />
          <span>Also delete the local branch of removed worktrees</span>
        </label>

        <div v-if="totals.modified || totals.unpushed || totals.stashes" class="banner">
          This will destroy {{ totals.modified }} uncommitted change(s),
          {{ totals.unpushed }} unpushed commit(s) and {{ totals.stashes }} stash(es).
        </div>

        <div v-for="failure in failures" :key="failure.path" class="banner error">
          <code>{{ shortPath(failure.path) }}</code> — {{ failure.error }}
        </div>
      </div>

      <div class="modal-footer">
        <span v-if="working" class="spin" />
        <span class="spacer" />
        <button class="btn" :disabled="working" @click="emit('close')">Cancel</button>
        <button
          class="btn primary danger"
          :disabled="working || !chosen.length"
          @click="remove"
        >
          Remove {{ chosen.length || '' }}
        </button>
      </div>
    </div>
  </div>
</template>
