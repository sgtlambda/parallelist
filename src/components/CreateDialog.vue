<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import BranchPicker from './BranchPicker.vue'
import type { Issue, OpenTarget } from '../api'
import { branchName, loadIssues } from '../branchPicker'
import { shortPath } from '../format'
import {
  baseBranch,
  createRequest,
  defaultSteps,
  defaultTargets,
  isNewBranch as isNew,
  offeredTargets,
  planWorkspace,
  rowFor,
  startCreate,
  targetsClash,
} from '../newWorkspace'
import { isActive, normalizePath, tasks } from '../tasks'
import { project, projectId } from '../store'
import { useEscape } from '../useEscape'

/** `initialBranch` fills the branch field: the selected branch from its
 *  workspace cards, empty from "Add branch". */
const props = defineProps<{ open: boolean; initialBranch: string }>()
const emit = defineEmits<{ (e: 'close'): void }>()

/** The field as typed, which may be a search; `name` is it as a branch. */
const branch = ref('')
const name = computed(() => branchName(branch.value))
/** The issue the branch was picked for: its number leads the directory. */
const issue = ref<Issue | null>(null)
const picker = ref<InstanceType<typeof BranchPicker> | null>(null)
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
const row = computed(() => rowFor(name.value))

const setupSteps = computed(() => project.value?.setupSteps ?? [])

/** "Open in" targets to run once the workspace is set up. */
const targets = computed(() => offeredTargets(projectId.value, row.value))
const base = computed(() => baseBranch())

const isNewBranch = computed(() => isNew(name.value))

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
  () => name.value.length > 0 && path.value.trim().length > 0 && !busy.value,
)

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) return
    branch.value = props.initialBranch
    issue.value = null
    if (projectId.value) loadIssues(projectId.value)
    // An empty branch has no plan to load, so nothing would replace the last
    // opening's directory and conflict.
    path.value = ''
    conflict.value = null
    mode.value = 'worktree'
    // What a double-click on a branch with no workspace makes, unchanged.
    steps.value = defaultSteps(projectId.value)
    runTargets.value = defaultTargets(projectId.value)
    // Focus is what lets Enter submit; prefilled or not, the branch field
    // is where it belongs.
    void nextTick(() => picker.value?.focus())
    await loadPlan()
  },
)

// Retyping the branch changes both the suggested directory and whether a
// worktree is even possible, so the plan is refreshed as you type. Picking an
// issue changes the number the directory leads with.
watch([name, issue], () => void loadPlan())

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
  const wanted = name.value
  const ticket = ++planTicket
  if (!id || !wanted) {
    planPending.value = false
    return
  }
  planPending.value = true
  try {
    // A PR's number over the issue's: the PR is the branch's own.
    const number = row.value?.pr?.number ?? issue.value?.number ?? null
    const plan = await planWorkspace(id, wanted, number)
    if (ticket !== planTicket) return
    path.value = plan.path
    conflict.value = plan.conflict
    mode.value = plan.mode
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

function toggleTarget(target: OpenTarget) {
  runTargets.value = runTargets.value.includes(target.id)
    ? runTargets.value.filter((id) => id !== target.id)
    : [...runTargets.value.filter((id) => !targetsClash(projectId.value, target, id)), target.id]
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

  const request = createRequest({
    projectId: id,
    branch: name.value,
    path: path.value,
    mode: mode.value,
    steps: steps.value,
    targets: runTargets.value,
  })
  if (await startCreate(request)) emit('close')
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
          <BranchPicker
            ref="picker"
            v-model="branch"
            v-model:issue="issue"
            :project-id="projectId"
            :base="base"
            :is-new="isNewBranch"
          />
        </div>

        <div class="field">
          <label>Kind</label>
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
