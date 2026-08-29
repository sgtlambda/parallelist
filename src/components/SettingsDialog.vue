<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { AppConfig, SetupStep } from '../api'
import { refresh, saveConfig, state } from '../store'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

type Tab = 'repo' | 'targets' | 'general'
const tab = ref<Tab>('repo')
const draft = ref<AppConfig | null>(null)
const saving = ref(false)

watch(
  () => props.open,
  (isOpen) => {
    // Edit a copy so Cancel really cancels.
    draft.value = isOpen && state.config ? structuredClone(state.config) : null
    tab.value = 'repo'
  },
)

const repo = computed(() =>
  draft.value?.repos.find((r) => r.id === draft.value?.selectedRepoId) ?? null,
)

/** Setup steps carry their payload in a kind-specific field. */
function stepValue(step: SetupStep): string {
  if (step.kind === 'command') return step.script ?? ''
  return (step.kind === 'copyFiles' ? step.files : step.paths)?.join(', ') ?? ''
}

function setStepValue(step: SetupStep, value: string) {
  if (step.kind === 'command') {
    step.script = value
    return
  }
  const list = value
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
  if (step.kind === 'copyFiles') step.files = list
  else step.paths = list
}

function changeStepKind(step: SetupStep, kind: SetupStep['kind']) {
  const value = stepValue(step)
  step.kind = kind
  delete step.files
  delete step.paths
  delete step.script
  setStepValue(step, value)
}

function addStep() {
  repo.value?.setupSteps.push({
    id: `step-${Date.now()}`,
    label: 'New step',
    defaultEnabled: false,
    kind: 'command',
    script: '',
  })
}

function addTarget() {
  draft.value?.openTargets.push({
    id: `target-${Date.now()}`,
    label: 'New target',
    icon: '▸',
    command: 'open {path}',
    primary: false,
    requiresPr: false,
  })
}

async function save() {
  if (!draft.value) return
  saving.value = true
  try {
    await saveConfig(draft.value)
    await refresh()
    emit('close')
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <div v-if="open && draft" class="scrim" @click.self="emit('close')">
    <div class="modal wide">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>Settings</span>
        <span class="spacer" />
        <div class="tabs" style="padding: 0; border: none">
          <button class="tab" :class="{ active: tab === 'repo' }" @click="tab = 'repo'">
            Repository
          </button>
          <button class="tab" :class="{ active: tab === 'targets' }" @click="tab = 'targets'">
            Open in
          </button>
          <button class="tab" :class="{ active: tab === 'general' }" @click="tab = 'general'">
            General
          </button>
        </div>
      </div>

      <div class="modal-body">
        <!-- Repository -->
        <template v-if="tab === 'repo'">
          <p v-if="!repo" class="empty">Select a repository first.</p>
          <template v-else>
            <div class="field">
              <label>Name</label>
              <input v-model="repo.name" />
            </div>
            <div class="field">
              <label>Primary clone</label>
              <input :value="repo.path" readonly class="mono" />
              <span class="hint">
                New worktrees hang off this clone and setup steps copy files out of it.
              </span>
            </div>
            <div class="field">
              <label>Base branch</label>
              <input
                :value="repo.baseBranch ?? ''"
                :placeholder="state.overview?.baseBranch ?? 'detected from origin/HEAD'"
                @input="repo.baseBranch = ($event.target as HTMLInputElement).value || null"
              />
              <span class="hint">Ahead/behind counts and merge detection measure against this.</span>
            </div>
            <div class="field">
              <label>Directory template</label>
              <input v-model="repo.pathTemplate" class="mono" />
              <span class="hint">
                <code>{parent}</code> <code>{repo}</code> <code>{slug}</code>
                <code>{branch}</code> <code>{pr}</code> — the slug is the branch's last segment,
                prefixed with the PR number when there is one.
              </span>
            </div>

            <div class="field">
              <label>Setup steps</label>
              <span class="hint">
                Offered as checkboxes when creating a checkout. A committed
                <code>.switcheroo.json</code> in the repo overrides these.
              </span>
              <div
                v-for="step in repo.setupSteps"
                :key="step.id"
                class="card"
                style="gap: 6px; margin-top: 6px"
              >
                <div class="row">
                  <input v-model="step.label" style="flex: 1" placeholder="Label" />
                  <select :value="step.kind" @change="changeStepKind(step, ($event.target as HTMLSelectElement).value as SetupStep['kind'])">
                    <option value="copyFiles">copy files</option>
                    <option value="symlink">symlink</option>
                    <option value="command">command</option>
                  </select>
                  <button
                    class="btn ghost small danger"
                    @click="repo.setupSteps = repo.setupSteps.filter((s) => s.id !== step.id)"
                  >
                    ✕
                  </button>
                </div>
                <input
                  class="mono"
                  :value="stepValue(step)"
                  :placeholder="step.kind === 'command' ? 'pnpm install' : '.env, .env.cypress'"
                  @input="setStepValue(step, ($event.target as HTMLInputElement).value)"
                />
                <label class="check">
                  <input v-model="step.defaultEnabled" type="checkbox" />
                  <span class="hint">Ticked by default</span>
                </label>
              </div>
              <button class="btn small" style="align-self: flex-start" @click="addStep">
                + Add step
              </button>
            </div>
          </template>
        </template>

        <!-- Open in -->
        <template v-else-if="tab === 'targets'">
          <span class="hint">
            Each target is a shell command run in your login shell.
            <code>{path}</code> <code>{branch}</code> <code>{repo}</code> <code>{prUrl}</code>
            are substituted and shell-quoted.
          </span>
          <div v-for="target in draft.openTargets" :key="target.id" class="card" style="gap: 6px">
            <div class="row">
              <input v-model="target.icon" style="width: 52px; text-align: center" />
              <input v-model="target.label" style="flex: 1" placeholder="Label" />
              <button
                class="btn ghost small danger"
                @click="draft.openTargets = draft.openTargets.filter((t) => t.id !== target.id)"
              >
                ✕
              </button>
            </div>
            <input v-model="target.command" class="mono" spellcheck="false" />
            <div class="row">
              <label class="check">
                <input v-model="target.primary" type="checkbox" />
                <span class="hint">Show as a button</span>
              </label>
              <label class="check">
                <input v-model="target.requiresPr" type="checkbox" />
                <span class="hint">Only when there's a PR</span>
              </label>
            </div>
          </div>
          <button class="btn small" style="align-self: flex-start" @click="addTarget">
            + Add target
          </button>
        </template>

        <!-- General -->
        <template v-else>
          <div class="field">
            <label>Fetch every (seconds)</label>
            <input v-model.number="draft.fetchIntervalSecs" type="number" min="30" />
            <span class="hint">
              <code>git fetch --all --prune</code> in the background, so the branch list and
              ahead/behind counts stay honest.
            </span>
          </div>
          <div class="field">
            <label>Refresh pull requests every (seconds)</label>
            <input v-model.number="draft.prRefreshSecs" type="number" min="30" />
          </div>
          <div class="field">
            <label>"Active" window (days)</label>
            <input v-model.number="draft.activeDays" type="number" min="1" />
            <span class="hint">
              A branch counts as active if it has an open PR, is checked out, or was committed to
              within this window.
            </span>
          </div>
          <label class="check">
            <input v-model="draft.fetchOnFocus" type="checkbox" />
            <span>Also fetch when the window regains focus</span>
          </label>
        </template>
      </div>

      <div class="modal-footer">
        <span class="spacer" />
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn primary" :disabled="saving" @click="save">Save</button>
      </div>
    </div>
  </div>
</template>
