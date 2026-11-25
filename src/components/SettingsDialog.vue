<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api, type AppConfig, type ProjectFile, type ScreenSearchModel } from '../api'
import { shortPath, templatePath } from '../format'
import { screenStatus } from '../screenSearch'
import { refresh, saveConfig, state } from '../store'
import { useEscape } from '../useEscape'
import Icon from './Icon.vue'
import OpenTargetList from './OpenTargetList.vue'

const props = defineProps<{ open: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

/** `general`, `targets`, or a project's id. */
const section = ref<string>('general')
const draft = ref<AppConfig | null>(null)
/** The draft as it was opened, to tell whether anything changed. */
const baseline = ref('')
const files = ref<Record<string, ProjectFile>>({})
const models = ref<ScreenSearchModel[]>([])
const saving = ref(false)
const error = ref<string | null>(null)
/** Times Esc or the scrim was refused over unsaved changes; replays the notice. */
const nudge = ref(0)
const form = ref<HTMLFormElement | null>(null)
const body = ref<HTMLElement | null>(null)

// Esc is a stray key as often as a click beside the window is a stray click,
// so it closes on the same terms (`dismiss`); Cancel is the way to discard.
useEscape(() => props.open, () => dismiss())

/** Lower bounds of the General numbers, used by both the inputs and `save`. */
const MIN = { fetchIntervalSecs: 30, prRefreshSecs: 30, activeDays: 1 }

watch(
  () => props.open,
  (isOpen) => {
    // Edit a copy so Cancel really cancels. Cloned through JSON, not
    // structuredClone: the config is a reactive proxy and structuredClone
    // throws DataCloneError on those, which silently left the dialog empty.
    draft.value =
      isOpen && state.config ? (JSON.parse(JSON.stringify(state.config)) as AppConfig) : null
    baseline.value = draft.value ? JSON.stringify(draft.value) : ''
    error.value = null
    nudge.value = 0
    if (!draft.value) return
    // Opens on the project on screen: "Project settings…" and the gear both
    // mean the one being looked at.
    const current = draft.value.selectedProjectId
    section.value = draft.value.projects.some((p) => p.id === current) ? current! : 'general'
    files.value = {}
    api.projectFiles().then(
      (found) => (files.value = found),
      () => (files.value = {}),
    )
    if (!models.value.length) {
      api.screenSearchModels().then(
        (found) => (models.value = found),
        () => {},
      )
    }
  },
)

/** The model picked in the draft: its name, and the memory it takes. */
const modelDetail = computed(() => {
  const m = models.value.find((m) => m.id === draft.value?.screenSearchModel)
  if (!m) return null
  const memory = m.memoryMb >= 1000 ? `${(m.memoryMb / 1000).toFixed(1)} GB` : `${m.memoryMb} MB`
  return `${m.name}, about ${memory} of memory while searching`
})

/** What the saved model is doing, worth saying: getting ready, or failed.
 *  Only for the model saved, not one picked in the draft and not saved. */
const modelNote = computed(() => {
  const status = screenStatus.value
  const saved = state.config
  if (!saved?.screenSearch || !draft.value?.screenSearch) return null
  if (draft.value.screenSearchModel !== saved.screenSearchModel) return null
  if (status.state === 'loading') return { error: false, text: 'Getting the model ready…' }
  if (status.state === 'failed') return { error: true, text: `The model couldn't start: ${status.error}` }
  return null
})

watch(section, () => {
  if (body.value) body.value.scrollTop = 0
})

const dirty = computed(() => !!draft.value && JSON.stringify(draft.value) !== baseline.value)

const project = computed(() => draft.value?.projects.find((p) => p.id === section.value) ?? null)
const file = computed(() => (project.value ? files.value[project.value.id] ?? null : null))
const overrides = computed(() => (file.value && !file.value.error ? file.value.overrides : null))

/** The base branch shown as a placeholder: what the overview detected, for
 *  the project on screen. */
const detectedBase = computed(() =>
  project.value && project.value.id === state.config?.selectedProjectId && state.overview?.baseBranch
    ? state.overview.baseBranch
    : 'detected from origin/HEAD',
)

const EXAMPLE = { branch: 'fix/login-redirect', pr: 1234 }
const preview = computed(() => {
  const p = project.value
  if (!p) return ''
  const template = overrides.value?.pathTemplate ?? p.pathTemplate
  return shortPath(templatePath(template, p.path, EXAMPLE.branch, EXAMPLE.pr))
})

function addStep() {
  project.value?.setupSteps.push({
    id: `step-${Date.now()}`,
    label: '',
    defaultEnabled: true,
    script: '',
  })
}

function removeStep(id: string) {
  if (project.value) project.value.setupSteps = project.value.setupSteps.filter((s) => s.id !== id)
}

/** The section holding the first field that won't save. The browser only
 *  checks the inputs on screen, and the other sections aren't rendered. */
function invalidSection(config: AppConfig): string | null {
  const keys = Object.keys(MIN) as (keyof typeof MIN)[]
  if (keys.some((key) => !Number.isInteger(config[key]) || config[key] < MIN[key])) return 'general'
  return config.projects.find((p) => !p.name.trim())?.id ?? null
}

async function save() {
  if (!draft.value) return
  if (!dirty.value) return emit('close')
  const bad = invalidSection(draft.value)
  if (bad) {
    section.value = bad
    await nextTick()
    form.value?.reportValidity()
    return
  }
  saving.value = true
  error.value = null
  try {
    await saveConfig(draft.value)
    emit('close')
    await refresh()
  } catch (e) {
    error.value = String(e)
  } finally {
    saving.value = false
  }
}

/** Esc or a click beside the window closes it only when nothing would be
 *  lost. With unsaved changes it stays, and the footer's notice flashes so
 *  the key or click visibly did something. */
function dismiss() {
  if (!dirty.value) return emit('close')
  nudge.value++
}

function onKey(event: KeyboardEvent) {
  if (event.metaKey && event.key.toLowerCase() === 's') {
    event.preventDefault()
    void save()
  }
}
</script>

<template>
  <div v-if="open && draft" class="scrim settings-scrim" @click.self="dismiss">
    <form
      ref="form"
      class="settings"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      @submit.prevent="save"
      @keydown="onKey"
    >
      <nav class="settings-nav" data-tauri-drag-region="deep">
        <div class="settings-nav-title" data-tauri-drag-region>Settings</div>
        <button
          type="button"
          class="settings-nav-item"
          :class="{ active: section === 'general' }"
          @click="section = 'general'"
        >
          <Icon name="sliders" :size="14" />
          General
        </button>
        <button
          type="button"
          class="settings-nav-item"
          :class="{ active: section === 'targets' }"
          @click="section = 'targets'"
        >
          <Icon name="launch" :size="14" />
          Open in
        </button>
        <button
          type="button"
          class="settings-nav-item"
          :class="{ active: section === 'screens' }"
          @click="section = 'screens'"
        >
          <Icon name="search" :size="14" />
          Screen search
        </button>

        <div class="settings-nav-heading">Projects</div>
        <div class="settings-nav-projects">
          <button
            v-for="p in draft.projects"
            :key="p.id"
            type="button"
            class="settings-nav-item"
            :class="{ active: section === p.id }"
            :title="shortPath(p.path)"
            @click="section = p.id"
          >
            <Icon name="folder" :size="14" />
            <span class="truncate">{{ p.name || 'Untitled' }}</span>
          </button>
          <p v-if="!draft.projects.length" class="settings-nav-empty">No projects yet.</p>
        </div>
      </nav>

      <section class="settings-main">
        <header class="settings-head" data-tauri-drag-region="deep">
          <div class="settings-head-text" data-tauri-drag-region>
            <h2 v-if="section === 'general'" data-tauri-drag-region>General</h2>
            <h2 v-else-if="section === 'targets'" data-tauri-drag-region>Open in</h2>
            <h2 v-else-if="section === 'screens'" data-tauri-drag-region>Screen search</h2>
            <template v-else-if="project">
              <h2 class="truncate" data-tauri-drag-region>{{ project.name || 'Untitled' }}</h2>
              <p class="mono truncate selectable">{{ shortPath(project.path) }}</p>
            </template>
          </div>
          <button type="button" class="settings-close" title="Close (Esc)" @click="emit('close')">
            <Icon name="close" :size="13" />
          </button>
        </header>

        <div ref="body" class="settings-body">
          <!-- General -->
          <template v-if="section === 'general'">
            <div class="set-group">
              <h3>Background refresh</h3>
              <div class="set-card">
                <div class="set-row" title="Runs git fetch --all --prune">
                  <div class="set-row-text">
                    <label for="set-fetch">Fetch origin every</label>
                  </div>
                  <div class="unit-input">
                    <input
                      id="set-fetch"
                      v-model.number="draft.fetchIntervalSecs"
                      type="number"
                      :min="MIN.fetchIntervalSecs"
                      step="1"
                      required
                    />
                    <span>sec</span>
                  </div>
                </div>
                <label class="set-row clickable">
                  <div class="set-row-text">
                    <span class="set-label">Also fetch when the window is focused</span>
                  </div>
                  <input v-model="draft.fetchOnFocus" type="checkbox" class="switch" />
                </label>
                <div class="set-row">
                  <div class="set-row-text">
                    <label for="set-prs">Refresh pull requests every</label>
                  </div>
                  <div class="unit-input">
                    <input
                      id="set-prs"
                      v-model.number="draft.prRefreshSecs"
                      type="number"
                      :min="MIN.prRefreshSecs"
                      step="1"
                      required
                    />
                    <span>sec</span>
                  </div>
                </div>
              </div>
            </div>

            <div class="set-group">
              <h3>Branch list</h3>
              <div class="set-card">
                <div class="set-row">
                  <div class="set-row-text">
                    <label for="set-active">Active window</label>
                    <p>
                      A branch is active if it has an open pull request, has a workspace, or was
                      committed to within this many days.
                    </p>
                  </div>
                  <div class="unit-input">
                    <input
                      id="set-active"
                      v-model.number="draft.activeDays"
                      type="number"
                      :min="MIN.activeDays"
                      step="1"
                      required
                    />
                    <span>days</span>
                  </div>
                </div>
              </div>
            </div>
          </template>

          <!-- Open in, for every project -->
          <template v-else-if="section === 'targets'">
            <div class="set-group">
              <h3>Every project</h3>
              <OpenTargetList :items="draft.openTargets" />
              <p class="set-foot">
                <code>{path}</code> the workspace, <code>{primary}</code> the main workspace and
                <code>{branch}</code> are filled in, shell-quoted.
              </p>
            </div>
          </template>

          <!-- Screen search -->
          <template v-else-if="section === 'screens'">
            <div class="set-group">
              <div class="set-card">
                <label class="set-row clickable">
                  <div class="set-row-text">
                    <span class="set-label">Search what terminals show</span>
                    <p>
                      The palette matches each terminal's screen by its words and by meaning, with a
                      model that runs on this machine.
                    </p>
                  </div>
                  <input v-model="draft.screenSearch" type="checkbox" class="switch" />
                </label>
                <div class="set-row" :class="{ 'set-disabled': !draft.screenSearch }">
                  <div class="set-row-text">
                    <label for="set-model">Model</label>
                    <p v-if="modelDetail">{{ modelDetail }}</p>
                    <p v-if="modelNote" :class="{ 'set-error': modelNote.error }">
                      {{ modelNote.text }}
                    </p>
                  </div>
                  <select
                    id="set-model"
                    v-model="draft.screenSearchModel"
                    class="set-select"
                    :disabled="!draft.screenSearch"
                  >
                    <option v-for="m in models" :key="m.id" :value="m.id">
                      {{ m.label }} · {{ m.downloadMb }} MB download
                    </option>
                  </select>
                </div>
              </div>
            </div>

          </template>

          <!-- A project -->
          <template v-else-if="project">
            <div v-if="file" class="set-notice" :class="{ error: file.error }">
              <Icon name="file" :size="14" />
              <p v-if="file.error">
                <code>{{ file.file }}</code> in this project isn't valid and is ignored:
                {{ file.error }}
              </p>
              <p v-else>
                This project commits a <code>{{ file.file }}</code>. What it sets wins over what is
                entered here, and is marked below.
              </p>
            </div>

            <div class="set-group">
              <h3>Project</h3>
              <div class="set-card">
                <div class="set-row">
                  <div class="set-row-text">
                    <label for="set-name">Name</label>
                  </div>
                  <input id="set-name" v-model="project.name" class="set-input" required />
                </div>
                <div class="set-row">
                  <div class="set-row-text">
                    <label for="set-base">Base branch</label>
                    <p v-if="overrides?.baseBranch" class="set-overridden">
                      Set to <code>{{ overrides.baseBranch }}</code> by
                      <code>{{ file!.file }}</code>.
                    </p>
                    <p v-else>Ahead/behind counts and merge detection measure against it.</p>
                  </div>
                  <input
                    id="set-base"
                    class="set-input"
                    :value="project.baseBranch ?? ''"
                    :placeholder="detectedBase"
                    spellcheck="false"
                    @input="project.baseBranch = ($event.target as HTMLInputElement).value || null"
                  />
                </div>
              </div>
            </div>

            <div class="set-group">
              <h3>New workspaces</h3>
              <div class="set-card">
                <div class="set-row stacked">
                  <div class="set-row-text">
                    <label for="set-template">Directory</label>
                    <p v-if="overrides?.pathTemplate" class="set-overridden">
                      Set to <code>{{ overrides.pathTemplate }}</code> by
                      <code>{{ file!.file }}</code>.
                    </p>
                    <p v-else>
                      <code>{parent}</code> is the folder holding the main workspace,
                      <code>{project}</code> its name, and <code>{slug}</code> the branch's last
                      segment after its pull request number. <code>{branch}</code> and
                      <code>{pr}</code> are there as well.
                    </p>
                  </div>
                  <input
                    id="set-template"
                    v-model="project.pathTemplate"
                    class="set-input mono"
                    spellcheck="false"
                  />
                  <p class="template-preview">
                    <span class="faint">{{ EXAMPLE.branch }} #{{ EXAMPLE.pr }}</span>
                    <span class="faint">→</span>
                    <span class="mono selectable truncate">{{ preview }}</span>
                  </p>
                </div>
              </div>
            </div>

            <div class="set-group">
              <h3>Setup steps</h3>
              <p v-if="overrides?.setupSteps" class="set-overridden">
                <code>{{ file!.file }}</code> sets {{ overrides.setupSteps.length }}
                {{ overrides.setupSteps.length === 1 ? 'step' : 'steps' }}, used instead of these.
              </p>
              <div class="set-list" :class="{ muted: overrides?.setupSteps }">
                <div v-if="project.setupSteps.length" class="set-list-head">
                  <span>Step</span>
                  <span>Ticked</span>
                </div>
                <div v-for="step in project.setupSteps" :key="step.id" class="set-item step">
                  <div class="step-text">
                    <input
                      v-model="step.label"
                      class="bare set-item-title"
                      placeholder="What it does"
                      aria-label="Label"
                    />
                    <div class="command">
                      <span class="command-prompt" aria-hidden="true">$</span>
                      <input
                        v-model="step.script"
                        class="bare mono"
                        placeholder="pnpm install"
                        aria-label="Command"
                        spellcheck="false"
                      />
                    </div>
                  </div>
                  <input
                    v-model="step.defaultEnabled"
                    type="checkbox"
                    class="switch"
                    title="Ticked by default in the New workspace dialog"
                    aria-label="Ticked by default"
                  />
                  <button type="button" class="set-remove" title="Remove step" @click="removeStep(step.id)">
                    <Icon name="close" :size="12" />
                  </button>
                </div>
                <button type="button" class="set-add" @click="addStep">
                  <Icon name="add" :size="12" />
                  Add step
                </button>
              </div>
              <p class="set-foot">
                <code>{path}</code> the new workspace, <code>{primary}</code> the main workspace and
                <code>{branch}</code> are filled in, shell-quoted.
              </p>
            </div>

            <div class="set-group">
              <h3>Open in, for this project only</h3>
              <p v-if="overrides?.openTargets" class="set-overridden">
                <code>{{ file!.file }}</code> sets {{ overrides.openTargets.length }}
                {{ overrides.openTargets.length === 1 ? 'target' : 'targets' }}, used instead of
                these.
              </p>
              <OpenTargetList :items="project.openTargets" :muted="!!overrides?.openTargets" />
            </div>
          </template>
        </div>

        <footer class="settings-foot">
          <span v-if="error" class="settings-error truncate" :title="error">{{ error }}</span>
          <span
            v-else-if="dirty"
            :key="nudge"
            class="settings-dirty"
            :class="{ nudged: nudge > 0 }"
            >Unsaved changes</span
          >
          <span class="spacer" />
          <button type="button" class="btn" @click="emit('close')">Cancel</button>
          <button type="submit" class="btn primary" :disabled="saving || !dirty" title="Save (⌘S)">
            Save
          </button>
        </footer>
      </section>
    </form>
  </div>
</template>
