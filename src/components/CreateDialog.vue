<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api, onSetupLog, type CreateRequest } from '../api'
import { shortPath } from '../format'
import { openTargets, refresh, project, projectId, selectedRow, state } from '../store'
import { useEscape } from '../useEscape'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

type Phase = 'form' | 'running' | 'done' | 'failed'

const phase = ref<Phase>('form')
const branch = ref('')
const path = ref('')
const mode = ref<'worktree' | 'clone'>('worktree')
const steps = ref<string[]>([])
const conflict = ref<{ branch: string; path: string } | null>(null)
const log = ref<{ line: string; error: boolean }[]>([])
const createdPath = ref('')
const logEl = ref<HTMLElement | null>(null)

// Not while a checkout is being created — the dialog owns that job.
useEscape(
  () => props.open && phase.value !== 'running',
  () => emit('close'),
)

const setupSteps = computed(() => project.value?.setupSteps ?? [])
const base = computed(() => state.overview?.baseBranch ?? 'main')

/** A branch we've never seen has to be created rather than checked out. */
const isNewBranch = computed(
  () =>
    branch.value.trim().length > 0 &&
    !(state.overview?.branches ?? []).some((b) => b.name === branch.value.trim()),
)

const canSubmit = computed(
  () => phase.value === 'form' && branch.value.trim().length > 0 && path.value.trim().length > 0,
)

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) return
    phase.value = 'form'
    log.value = []
    createdPath.value = ''
    branch.value = selectedRow.value?.branch.name ?? ''
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
    const plan = await api.planCheckout(id, name, selectedRow.value?.pr?.number ?? null)
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

async function submit() {
  const id = projectId.value
  if (!id || !canSubmit.value) return

  const jobId = `job-${Date.now()}`
  const request: CreateRequest = {
    projectId: id,
    branch: branch.value.trim(),
    path: path.value.trim(),
    mode: mode.value,
    newBranch: isNewBranch.value,
    from: isNewBranch.value ? `origin/${base.value}` : null,
    steps: steps.value,
  }

  phase.value = 'running'
  log.value = []
  const unlisten = await onSetupLog(jobId, (event) => {
    log.value.push({ line: event.line, error: event.error })
    void nextTick(() => logEl.value?.scrollTo({ top: logEl.value.scrollHeight }))
  })

  try {
    createdPath.value = await api.createCheckout(jobId, request)
    phase.value = 'done'
    await refresh()
    state.selectedBranch = request.branch
  } catch (e) {
    log.value.push({ line: String(e), error: true })
    phase.value = 'failed'
  } finally {
    unlisten()
  }
}

async function openCreated(targetId: string) {
  await api.openIn(targetId, {
    path: createdPath.value,
    branch: branch.value.trim(),
    repo: project.value?.path ?? '',
    prUrl: selectedRow.value?.pr?.url ?? '',
  })
}
</script>

<template>
  <div v-if="open" class="scrim" @click.self="phase !== 'running' && emit('close')">
    <div class="modal">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>New checkout</span>
        <span class="spacer" />
        <span class="faint" style="font-weight: 400">{{ project?.name }}</span>
      </div>

      <div class="modal-body">
        <template v-if="phase === 'form'">
          <div class="field">
            <label for="branch">Branch</label>
            <input id="branch" v-model="branch" spellcheck="false" />
            <span v-if="isNewBranch" class="hint">
              Doesn't exist yet — it will be created from <code>origin/{{ base }}</code> with no
              upstream set, so your first push goes to its own branch.
            </span>
          </div>

          <div class="field">
            <label>How</label>
            <div v-if="conflict" class="banner">
              <strong>{{ conflict.branch }}</strong> is already checked out at
              <code>{{ shortPath(conflict.path) }}</code
              >. Git allows a branch in only one worktree, so a second copy has to be a clone.
            </div>

            <label class="choice" :class="{ selected: mode === 'worktree', disabled: !!conflict }">
              <input v-model="mode" type="radio" value="worktree" :disabled="!!conflict" />
              <span>
                <strong>Worktree</strong>
                <div class="hint">Shares the object store with the primary clone. Cheap and instant.</div>
              </span>
            </label>

            <label class="choice" :class="{ selected: mode === 'clone' }">
              <input v-model="mode" type="radio" value="clone" />
              <span>
                <strong>Full clone</strong>
                <div class="hint">
                  Independent repo, so the same branch can be checked out twice. Cloned locally with
                  hardlinked objects, then pointed back at origin.
                </div>
              </span>
            </label>
          </div>

          <div class="field">
            <label for="path">Directory</label>
            <input id="path" v-model="path" spellcheck="false" class="mono" />
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
        </template>

        <template v-else>
          <div ref="logEl" class="log">
            <div
              v-for="(entry, i) in log"
              :key="i"
              :class="{ err: entry.error, step: entry.line.startsWith('▸') }"
            >
              {{ entry.line }}
            </div>
            <div v-if="phase === 'running'" class="muted">…</div>
          </div>

          <div v-if="phase === 'done'" class="field">
            <label>Ready at</label>
            <code class="selectable">{{ shortPath(createdPath) }}</code>
            <div class="actions" style="margin-top: 6px">
              <button
                v-for="target in openTargets.filter((t) => !t.requiresPr)"
                :key="target.id"
                class="btn small"
                @click="openCreated(target.id)"
              >
                {{ target.icon }} {{ target.label }}
              </button>
            </div>
          </div>
        </template>
      </div>

      <div class="modal-footer">
        <span v-if="phase === 'running'" class="spin" />
        <span v-if="phase === 'running'" class="muted">Working…</span>
        <span v-if="phase === 'failed'" class="muted">Failed — see the log above.</span>
        <span class="spacer" />
        <button class="btn" :disabled="phase === 'running'" @click="emit('close')">
          {{ phase === 'done' ? 'Close' : 'Cancel' }}
        </button>
        <button v-if="phase === 'form'" class="btn primary" :disabled="!canSubmit" @click="submit">
          Create
        </button>
      </div>
    </div>
  </div>
</template>
