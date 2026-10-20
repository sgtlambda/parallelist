<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api, type CreateRequest, type OpenTarget } from '../api'
import { shortPath } from '../format'
import { isActive, normalizePath, tasks } from '../tasks'
import { openTargetsFor, project, projectId, rows, state } from '../store'
import { useEscape } from '../useEscape'

/** `initialBranch` fills the branch field: the selected branch from its
 *  workspace cards, empty from "Add branch". */
const props = defineProps<{ open: boolean; initialBranch: string }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const branch = ref('')
const branchInput = ref<HTMLInputElement | null>(null)
const path = ref('')
const mode = ref<'worktree' | 'clone'>('worktree')
const steps = ref<string[]>([])
const runTargets = ref<string[]>([])
const conflict = ref<{ branch: string; path: string } | null>(null)

useEscape(
  () => props.open,
  () => emit('close'),
)

/** The row for the branch as typed, not the selected one: the field can name
 *  any branch, or one that doesn't exist yet and so has no PR. */
const row = computed(() => rows.value.find((r) => r.branch.name === branch.value.trim()) ?? null)

const setupSteps = computed(() => project.value?.setupSteps ?? [])

/** "Open in" targets to run once the workspace is set up. One that needs a PR
 *  only when the branch as typed has one. */
const targets = computed(() =>
  openTargetsFor(projectId.value).filter((t) => !t.requiresPr || row.value?.pr),
)
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
    branch.value = props.initialBranch
    // An empty branch has no plan to load, so nothing would replace the last
    // opening's directory and conflict.
    path.value = ''
    conflict.value = null
    mode.value = 'worktree'
    steps.value = setupSteps.value.filter((s) => s.defaultEnabled).map((s) => s.id)
    runTargets.value = []
    for (const t of openTargetsFor(projectId.value)) {
      if (t.runOnCreate && !runTargets.value.some((id) => clashes(t, id))) {
        runTargets.value.push(t.id)
      }
    }
    // Focus is what lets Enter submit; prefilled or not, the branch field
    // is where it belongs.
    void nextTick(() => branchInput.value?.focus())
    await loadPlan()
  },
)

// Retyping the branch changes both the suggested directory and whether a
// worktree is even possible, so the plan is refreshed as you type.
watch(branch, () => void loadPlan())

let planTicket = 0
/** The newest plan request, which `submit` waits for: Enter pressed right
 *  after opening, or right after typing, would otherwise find no directory
 *  yet, or the one suggested for the branch as it was a keystroke ago. */
let planning: Promise<void> = Promise.resolve()
const planPending = ref(false)

function loadPlan() {
  planning = fetchPlan()
  return planning
}

async function fetchPlan() {
  const id = projectId.value
  const name = branch.value.trim()
  const ticket = ++planTicket
  if (!id || !name) {
    planPending.value = false
    return
  }
  planPending.value = true
  try {
    const plan = await api.planWorkspace(id, name, row.value?.pr?.number ?? null)
    if (ticket !== planTicket) return
    path.value = plan.suggestedPath
    conflict.value = plan.conflict
    mode.value = plan.conflict ? 'clone' : 'worktree'
  } catch {
    /* a bad branch name simply leaves the previous suggestion in place */
  } finally {
    if (ticket === planTicket) planPending.value = false
  }
}

function toggleStep(id: string) {
  steps.value = steps.value.includes(id)
    ? steps.value.filter((s) => s !== id)
    : [...steps.value, id]
}

/** At most one built-in terminal target is ticked: a second would be typed
 *  into whatever the first started. */
function clashes(target: OpenTarget, id: string): boolean {
  return (
    target.kind === 'terminal' &&
    openTargetsFor(projectId.value).find((t) => t.id === id)?.kind === 'terminal'
  )
}

function toggleTarget(target: OpenTarget) {
  runTargets.value = runTargets.value.includes(target.id)
    ? runTargets.value.filter((id) => id !== target.id)
    : [...runTargets.value.filter((id) => !clashes(target, id)), target.id]
}

/** Start the task and get out of the way. Its progress shows in the task box
 *  and on the pending card in the detail pane, so the branch is selected to
 *  put that card on screen. */
const submitting = ref(false)

async function submit() {
  if (submitting.value) return
  submitting.value = true
  try {
    while (planPending.value) await planning
    await start()
  } finally {
    submitting.value = false
  }
}

async function start() {
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
    // In the order they are listed, and only those still on offer.
    openTargets: targets.value.filter((t) => runTargets.value.includes(t.id)).map((t) => t.id),
  }

  try {
    await api.createWorkspace(request, row.value?.pr?.url ?? null)
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
          <input id="branch" ref="branchInput" v-model="branch" />
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

        <div v-if="targets.length" class="field">
          <label>Then open in</label>
          <label v-for="target in targets" :key="target.id" class="check">
            <input
              type="checkbox"
              :checked="runTargets.includes(target.id)"
              @change="toggleTarget(target)"
            />
            <span>{{ target.icon }} {{ target.label }}</span>
          </label>
          <span class="hint">
            Run once the workspace is set up. Only one can use the built-in terminal; if
            you have moved on to another branch by then, it starts without taking you
            back.
          </span>
        </div>
      </div>

      <div class="modal-footer">
        <span class="spacer" />
        <button type="button" class="btn" @click="emit('close')">Cancel</button>
        <!-- Enabled while a plan is on its way: implicit submission does
             nothing at all when the default button is disabled. -->
        <button type="submit" class="btn primary" :disabled="!canSubmit && !planPending">
          Create
        </button>
      </div>
    </form>
  </div>
</template>
