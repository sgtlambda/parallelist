<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import Icon from './Icon.vue'
import TerminalMark from './TerminalMark.vue'
import { WORKSPACE_LABELS, basename, relativeTime } from '../format'
import { highlight } from '../fuzzy'
import { paletteLoading, paletteOpen, refreshPalette, searchPalette, type Result } from '../palette'
import { enterShellOf } from '../enterShell'
import { goToBranch } from '../store'
import { useEscape } from '../useEscape'

/** Rendered at most; the query narrows it long before anyone scrolls that
 *  far. */
const LIMIT = 60

const query = ref('')
const index = ref(0)
const input = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)

const results = computed(() => (paletteOpen.value ? searchPalette(query.value, LIMIT) : []))

/** Whatever had the keyboard before, so dismissing hands it back — typically
 *  a terminal. Not after going to a branch: that is somewhere else. */
let returnFocus: HTMLElement | null = null

watch(paletteOpen, (open) => {
  if (!open) {
    returnFocus?.focus()
    returnFocus = null
    return
  }
  returnFocus = document.activeElement as HTMLElement | null
  query.value = ''
  index.value = 0
  refreshPalette()
  void nextTick(() => input.value?.focus())
})

watch(query, () => {
  index.value = 0
  list.value?.scrollTo({ top: 0 })
})

function close() {
  paletteOpen.value = false
}

useEscape(() => paletteOpen.value, close)

function choose(result: Result | undefined) {
  if (!result) return
  returnFocus = null
  paletteOpen.value = false
  goToBranch(result.entry.projectId, result.entry.branch.name)
}

/** Straight into the branch's shell, as double-clicking it in the list does. */
function enter(result: Result | undefined) {
  if (!result) return
  returnFocus = null
  paletteOpen.value = false
  void enterShellOf(result.entry.projectId, result.entry.branch.name)
}

function move(by: number) {
  const count = results.value.length
  if (!count) return
  index.value = (index.value + by + count) % count
  void nextTick(() =>
    list.value?.querySelector('.palette-item.active')?.scrollIntoView({ block: 'nearest' }),
  )
}

function onKey(event: KeyboardEvent) {
  if (event.isComposing) return
  const ctrl = event.ctrlKey && !event.metaKey && !event.altKey
  if (event.key === 'ArrowDown' || (ctrl && (event.key === 'n' || event.key === 'j'))) {
    event.preventDefault()
    move(1)
  } else if (event.key === 'ArrowUp' || (ctrl && (event.key === 'p' || event.key === 'k'))) {
    event.preventDefault()
    move(-1)
  } else if (event.key === 'Enter' && event.metaKey) {
    event.preventDefault()
    // The palette is gone by the time this reaches the window, whose ⌘↵
    // would then go to whatever has waited longest as well.
    event.stopPropagation()
    enter(results.value[index.value])
  } else if (event.key === 'Enter') {
    event.preventDefault()
    choose(results.value[index.value])
  }
}

/** On movement, not on enter: a list scrolled under a resting pointer would
 *  otherwise steal the selection from the keyboard. */
function hover(i: number) {
  index.value = i
}
</script>

<template>
  <div v-if="paletteOpen" class="scrim palette-scrim" @click.self="close">
    <div class="modal palette">
      <div class="palette-search">
        <Icon name="search" />
        <input
          ref="input"
          v-model="query"
          placeholder="Go to a branch in any project…"
          spellcheck="false"
          autocomplete="off"
          @keydown="onKey"
        />
        <span v-if="paletteLoading" class="spin tiny" title="Reading branches" />
      </div>

      <div ref="list" class="palette-list">
        <p v-if="!results.length" class="empty">
          {{ paletteLoading ? 'Reading branches…' : 'No branch matches.' }}
        </p>
        <button
          v-for="(result, i) in results"
          :key="`${result.entry.projectId}\0${result.entry.branch.name}`"
          class="palette-item"
          :class="{ active: i === index }"
          @mousemove="hover(i)"
          @click="choose(result)"
        >
          <span class="branch-name truncate">
            <span
              v-for="(part, j) in highlight(result.entry.branch.name, result.branchHits)"
              :key="j"
              :class="{ hit: part.hit }"
              >{{ part.text }}</span
            >
          </span>
          <span v-if="result.entry.workspaces.length" class="folder">
            <span
              v-for="w in result.entry.workspaces"
              :key="w.path"
              class="folder-entry"
              :title="`${WORKSPACE_LABELS[w.kind]}\n${w.path}`"
            >
              <Icon :name="w.kind" :size="12" />
              <span class="truncate">{{ basename(w.path) }}</span>
            </span>
          </span>
          <span class="spacer" />
          <TerminalMark :paths="result.entry.workspaces.map((w) => w.path)" />
          <span class="palette-project truncate" :class="{ current: result.entry.current }">
            <span
              v-for="(part, j) in highlight(result.entry.projectName, result.projectHits)"
              :key="j"
              :class="{ hit: part.hit }"
              >{{ part.text }}</span
            >
          </span>
          <span class="palette-age faint">{{ relativeTime(result.entry.branch.committedAt) }}</span>
        </button>
      </div>

      <div class="palette-footer faint">
        <span><kbd>↑</kbd><kbd>↓</kbd> move</span>
        <span><kbd>↵</kbd> go to branch</span>
        <span><kbd>⌘↵</kbd> open shell</span>
        <span><kbd>esc</kbd> close</span>
      </div>
    </div>
  </div>
</template>
