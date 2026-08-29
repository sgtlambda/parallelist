<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { AppConfig, OpenTarget } from '../api'
import { refresh, saveConfig, state } from '../store'
import { useEscape } from '../useEscape'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

type Tab = 'project' | 'targets' | 'general'
useEscape(() => props.open, () => emit('close'))

const tab = ref<Tab>('project')
const draft = ref<AppConfig | null>(null)
const saving = ref(false)

watch(
  () => props.open,
  (isOpen) => {
    // Edit a copy so Cancel really cancels. Cloned through JSON, not
    // structuredClone: the config is a reactive proxy and structuredClone
    // throws DataCloneError on those, which silently left the dialog empty.
    draft.value =
      isOpen && state.config ? (JSON.parse(JSON.stringify(state.config)) as AppConfig) : null
    tab.value = 'project'
  },
)

const project = computed(
  () => draft.value?.projects.find((p) => p.id === draft.value?.selectedProjectId) ?? null,
)

function addStep() {
  project.value?.setupSteps.push({
    id: `step-${Date.now()}`,
    label: 'New step',
    defaultEnabled: false,
    script: '',
  })
}

const targetGroups = computed(() => [
  {
    key: 'global' as const,
    title: 'Every project',
    items: draft.value?.openTargets ?? [],
  },
  {
    key: 'project' as const,
    title: project.value ? `Only ${project.value.name}` : 'This project',
    items: project.value?.openTargets ?? [],
  },
])

function addTarget(items: OpenTarget[]) {
  items.push({
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
          <button class="tab" :class="{ active: tab === 'project' }" @click="tab = 'project'">
            {{ project?.name ?? 'Project' }}
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
        <!-- Project -->
        <template v-if="tab === 'project'">
          <p v-if="!project" class="empty">Select a project first.</p>
          <template v-else>
            <div class="field">
              <label>Name</label>
              <input v-model="project.name" />
            </div>
            <div class="field">
              <label>Primary clone</label>
              <input :value="project.path" readonly class="mono" />
              <span class="hint">
                New worktrees hang off this clone and setup steps copy files out of it.
              </span>
            </div>
            <div class="field">
              <label>Base branch</label>
              <input
                :value="project.baseBranch ?? ''"
                :placeholder="state.overview?.baseBranch ?? 'detected from origin/HEAD'"
                @input="project.baseBranch = ($event.target as HTMLInputElement).value || null"
              />
              <span class="hint">Ahead/behind counts and merge detection measure against this.</span>
            </div>
            <div class="field">
              <label>Directory template</label>
              <input v-model="project.pathTemplate" class="mono" />
              <span class="hint">
                <code>{parent}</code> <code>{project}</code> <code>{slug}</code>
                <code>{branch}</code> <code>{pr}</code> — the slug is the branch's last segment,
                prefixed with the PR number when there is one.
              </span>
            </div>

            <div class="field">
              <label>Setup steps</label>
              <span class="hint">
                Shell commands, offered as checkboxes when creating a workspace and
                run inside it. <code>{primary}</code> <code>{path}</code>
                <code>{branch}</code> are substituted and shell-quoted. A committed
                <code>.switcheroo.json</code> in the project overrides these.
              </span>
              <div
                v-for="step in project.setupSteps"
                :key="step.id"
                class="card"
                style="gap: 6px; margin-top: 6px"
              >
                <div class="row">
                  <input v-model="step.label" style="flex: 1" placeholder="Label" />
                  <button
                    class="btn ghost small danger"
                    @click="project.setupSteps = project.setupSteps.filter((s) => s.id !== step.id)"
                  >
                    ✕
                  </button>
                </div>
                <input v-model="step.script" class="mono" spellcheck="false" placeholder="pnpm install" />
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
            <code>{path}</code> <code>{branch}</code> <code>{primary}</code> <code>{prUrl}</code>
            are substituted and shell-quoted.
          </span>
          <template v-for="group in targetGroups" :key="group.key">
          <div class="card-title" style="margin-top: 4px">{{ group.title }}</div>
          <p v-if="group.key === 'project' && !project" class="hint">
            Select a project to add targets that only apply to it.
          </p>
          <div v-for="target in group.items" :key="target.id" class="card" style="gap: 6px">
            <div class="row">
              <input v-model="target.icon" style="width: 52px; text-align: center" />
              <input v-model="target.label" style="flex: 1" placeholder="Label" />
              <button
                class="btn ghost small danger"
                @click="group.items.splice(group.items.indexOf(target), 1)"
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
          <button
            v-if="group.key === 'global' || project"
            class="btn small"
            style="align-self: flex-start"
            @click="addTarget(group.items)"
          >
            + Add target
          </button>
          </template>
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
