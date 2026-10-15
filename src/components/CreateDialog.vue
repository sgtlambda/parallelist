<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api, type CreateRequest } from '../api'
import { shortPath } from '../format'
import { isActive, normalizePath, tasks } from '../tasks'
import { project, projectId, selectedRow, state } from '../store'
import { useEscape } from '../useEscape'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const branch = ref('')
const path = ref('')
const mode = ref<'worktree' | 'clone'>('worktree')
const steps = ref<string[]>([])
const conflict = ref<{ branch: string; path: string } | null>(null)

useEscape(
  () => props.open,
  () => emit('close'),
)

const setupSteps = computed(() => project.value?.setupSteps ?? [])
const base = computed(() => state.overview?.baseBranch ?? 'main')

/** A branch we've never seen has to be created, not just placed in a
 *  workspace. */
const isNewBranch = computed(
  () =>
    branch.value.trim().length > 0 &&
    !(state.overview?.branches ?? []).some((b) => b.name === branch.value.trim()),
)

/** A task already creating this exact directory. The backend refuses the
 *  second one too; this says so before the button is pressed. */
const busy = computed(() =>
  tasks.value.some(
    (t) =>
      t.kind === 'create' &&
      isActive(t) &&
      normalizePath(t.path ?? '') === normalizePath(path.value.trim()),
  ),
)

const canSubmit = computed(
  () => branch.value.trim().length > 0 && path.value.trim().length > 0 && !busy.value,
)

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) return
    branch.value = state.selectedBranch ?? ''
    steps.value = setupSteps.value.filter((s) => s.defaultEnabled).map((s) => s.id)
    await loadPlan()
  },
)

// Retyping the branch changes both the suggested directory and whether a
// worktree is even possible, so the plan is refreshed as you type.
watch(branch, () => void loadPlan())

let planTicket = 0
async function loadPlan() {
  const id = projectId.value
  const name = branch.value.trim()
  if (!id || !name) return
  const ticket = ++planTicket
  try {
    const plan = await api.planWorkspace(id, name, selectedRow.value?.pr?.number ?? null)
    if (ticket !== planTicket) return
    path.value = plan.suggestedPath
    conflict.value = plan.conflict
    mode.value = plan.conflict ? 'clone' : 'worktree'
  } catch {
    /* a bad branch name simply leaves the previous suggestion in place */
  }
}

function toggleStep(id: string) {
  steps.value = steps.value.includes(id)
    ? steps.value.filter((s) => s !== id)
    : [...steps.value, id]
}

/** Start the task and get out of the way. Its progress shows in the task box
 *  and on the pending card in the detail pane, so the branch is selected to
 *  put that card on screen. */
async function submit() {
  const id = projectId.value
  if (!id || !canSubmit.value) return

  const request: CreateRequest = {
    projectId: id,
    branch: branch.value.trim(),
    path: path.value.trim(),
    mode: mode.value,
    newBranch: isNewBranch.value,
    from: isNewBranch.value ? `origin/${base.value}` : null,
    steps: steps.value,
  }

  try {
    await api.createWorkspace(request, selectedRow.value?.pr?.url ?? null)
  } catch (e) {
    state.error = String(e)
    return
  }
  state.selectedBranch = request.branch
  emit('close')
}
</script>

<template>
  <div v-if="open" class="scrim" @click.self="emit('close')">
    <form class="modal" @submit.prevent="submit">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>New workspace</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">{{ project?.name }}</span>
      </div>

      <div class="modal-body">
        <div class="field">
          <label for="branch">Branch</label>
          <input id="branch" v-model="branch" />
          <span v-if="isNewBranch" class="hint">
            Doesn't exist yet — it will be created from <code>origin/{{ base }}</code> with no
            upstream set, so your first push goes to its own branch.
          </span>
        </div>

        <div class="field">
          <label>Kind</label>
          <span class="hint">
            A workspace is a directory holding this branch. It is either a worktree of the
            main workspace or a clone of its own — everywhere else in Switcheroo the two are
            treated the same.
          </span>
          <div v-if="conflict" class="banner">
            <strong>{{ conflict.branch }}</strong> already has a worktree at
            <code>{{ shortPath(conflict.path) }}</code
            >. Git allows a branch in only one worktree per clone, so a second workspace for it
            has to be a clone.
          </div>

          <label class="choice" :class="{ selected: mode === 'worktree', disabled: !!conflict }">
            <input v-model="mode" type="radio" value="worktree" :disabled="!!conflict" />
            <span>
              <strong>Worktree</strong>
              <div class="hint">
                Shares the main workspace's object store. Cheap and instant.
              </div>
            </span>
          </label>

          <label class="choice" :class="{ selected: mode === 'clone' }">
            <input v-model="mode" type="radio" value="clone" />
            <span>
              <strong>Clone</strong>
              <div class="hint">
                An independent repository, so the same branch can sit in two workspaces at
                once. Copied locally with hardlinked objects, then pointed back at origin.
              </div>
            </span>
          </label>
        </div>

        <div class="field">
          <label for="path">Directory</label>
          <input id="path" v-model="path" class="mono" />
          <span v-if="busy" class="hint">This directory is already being created.</span>
        </div>

        <div v-if="setupSteps.length" class="field">
          <label>Setup</label>
          <label v-for="step in setupSteps" :key="step.id" class="check">
            <input
              type="checkbox"
              :checked="steps.includes(step.id)"
              @change="toggleStep(step.id)"
            />
            <span>{{ step.label }}</span>
          </label>
          <span class="hint">
            Steps run in the background. The workspace card and the task box show what
            they are on; clicking either shows the full log.
          </span>
        </div>
      </div>

      <div class="modal-footer">
        <span class="spacer" />
        <button type="button" class="btn" @click="emit('close')">Cancel</button>
        <button type="submit" class="btn primary" :disabled="!canSubmit">Create</button>
      </div>
    </form>
  </div>
</template>
