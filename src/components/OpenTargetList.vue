<script setup lang="ts">
import { nextTick, ref } from 'vue'
import type { OpenTarget } from '../api'
import Icon from './Icon.vue'

/**
 * The "Open in" targets of one scope (every project, or one), edited in
 * place: `items` is the Settings draft's own array. One line per target,
 * which says what it is, what it runs and how it is offered; clicking it
 * opens the target's fields under it, one target at a time.
 */
const props = defineProps<{ items: OpenTarget[]; muted?: boolean }>()

const openId = ref<string | null>(null)
const root = ref<HTMLElement | null>(null)

const KINDS = {
  command: {
    label: 'Launches on its own',
    short: 'App',
    icon: 'launch',
    note: 'Run detached through the login shell, to open another app.',
    placeholder: 'code {path}',
  },
  terminal: {
    label: 'Runs in the terminal',
    short: 'Terminal',
    icon: 'terminal',
    note: "Typed into the workspace's terminal here, which is then shown.",
    placeholder: 'claude',
  },
} as const

const OPTIONS = [
  {
    key: 'primary',
    label: 'Primary',
    note: "Shown as a button on the workspace card. The others are in the card's menu.",
  },
  {
    key: 'runOnCreate',
    label: 'Ticked for new workspaces',
    note: 'Starts ticked in the New workspace dialog, and runs once the workspace is set up.',
  },
] as const

function toggle(id: string) {
  openId.value = openId.value === id ? null : id
}

async function add() {
  const target: OpenTarget = {
    id: `target-${Date.now()}`,
    label: '',
    icon: '▸',
    command: '',
    kind: 'command',
    primary: false,
    runOnCreate: false,
  }
  props.items.push(target)
  openId.value = target.id
  await nextTick()
  root.value?.querySelector<HTMLInputElement>(`[data-id="${target.id}"] .ot-name`)?.focus()
}

function remove(target: OpenTarget) {
  props.items.splice(props.items.indexOf(target), 1)
  if (openId.value === target.id) openId.value = null
}
</script>

<template>
  <div ref="root" class="ot" :class="{ muted }">
    <p v-if="!items.length" class="ot-empty">No targets yet.</p>
    <div
      v-for="target in items"
      :key="target.id"
      class="ot-item"
      :class="{ open: openId === target.id }"
      :data-id="target.id"
    >
      <button
        type="button"
        class="ot-row"
        :aria-expanded="openId === target.id"
        @click="toggle(target.id)"
      >
        <span class="ot-icon">{{ target.icon || '▸' }}</span>
        <span class="ot-text">
          <span class="ot-label">{{ target.label || 'Untitled' }}</span>
          <span class="ot-command mono">{{ target.command || 'No command' }}</span>
        </span>
        <span class="ot-tags">
          <span v-if="target.primary" class="ot-tag">Primary</span>
          <span v-if="target.runOnCreate" class="ot-tag accent" title="Ticked for new workspaces">
            New
          </span>
        </span>
        <span class="ot-kind" :title="KINDS[target.kind].label">
          <Icon :name="KINDS[target.kind].icon" :size="13" />
        </span>
        <Icon name="back" :size="12" class="ot-chevron" />
      </button>

      <div class="ot-drawer">
        <div class="ot-drawer-inner">
          <div class="ot-form">
            <div class="ot-field">
              <label :for="`ot-name-${target.id}`">Name</label>
              <div class="ot-name-row">
                <input
                  v-model="target.icon"
                  class="ot-icon-input"
                  maxlength="2"
                  aria-label="Icon"
                  title="Icon: one character or emoji"
                />
                <input
                  :id="`ot-name-${target.id}`"
                  v-model="target.label"
                  class="ot-name"
                  placeholder="VS Code"
                />
              </div>
            </div>

            <div class="ot-field">
              <label>Runs</label>
              <div class="segmented ot-segmented" role="radiogroup" aria-label="Where the command runs">
                <button
                  v-for="(kind, key) in KINDS"
                  :key="key"
                  type="button"
                  role="radio"
                  :aria-checked="target.kind === key"
                  :class="{ on: target.kind === key }"
                  :title="kind.note"
                  @click="target.kind = key"
                >
                  <Icon :name="kind.icon" :size="12" />
                  {{ kind.short }}
                </button>
              </div>
            </div>

            <div class="ot-field ot-field-wide">
              <label :for="`ot-cmd-${target.id}`">Command</label>
              <div class="command">
                <span class="command-prompt" aria-hidden="true">$</span>
                <input
                  :id="`ot-cmd-${target.id}`"
                  v-model="target.command"
                  class="mono"
                  :placeholder="KINDS[target.kind].placeholder"
                  spellcheck="false"
                />
              </div>
              <p class="ot-hint">{{ KINDS[target.kind].note }}</p>
            </div>
          </div>

          <div class="ot-options">
            <label v-for="option in OPTIONS" :key="option.key" class="ot-option">
              <span class="ot-option-text">
                <span>{{ option.label }}</span>
                <small>{{ option.note }}</small>
              </span>
              <input v-model="target[option.key]" type="checkbox" class="switch" />
            </label>
          </div>

          <div class="ot-actions">
            <button type="button" class="ot-remove" @click="remove(target)">Remove target</button>
            <button type="button" class="ot-done" @click="openId = null">Done</button>
          </div>
        </div>
      </div>
    </div>
    <button type="button" class="set-add" @click="add">
      <Icon name="add" :size="12" />
      Add target
    </button>
  </div>
</template>
