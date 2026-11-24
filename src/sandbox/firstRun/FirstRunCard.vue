<script setup lang="ts">
import { computed, ref } from 'vue'
import Icon from '../../components/Icon.vue'
import { adoptTargets, useSuggestions } from '../../firstRun'
import { shortPath } from '../../format'
import { state } from '../../store'
import type { OpenTarget } from '../../api'

/**
 * Concept 3, on the card. No wizard: until the choice is made, a workspace
 * card offers what was found where its "Open in" buttons go, drawn dashed.
 * Clicking one opens the workspace in it, and keeps it, so the first use
 * is the choice. The rest found are one click further, in a list under the
 * buttons. "Keep these" takes every suggestion on show; "Not now" ends the
 * offer. Shown in a pane the width of the detail pane, with two cards.
 */
const { list, found, chosen, toggle } = useSuggestions()
const offering = ref(true)
const showMore = ref(false)

const kept = computed(() => state.config?.openTargets ?? [])
const keptIds = computed(() => new Set(kept.value.map((t) => t.id)))
const left = computed(() => found.value.filter((s) => !keptIds.value.has(s.target.id)))
// The recommended primaries are the buttons; everything else found is "more".
const suggested = computed(() =>
  offering.value ? left.value.filter((s) => s.recommended && s.target.primary) : [],
)
const more = computed(() =>
  offering.value ? left.value.filter((s) => !(s.recommended && s.target.primary)) : [],
)
const buttons = computed(() => kept.value.filter((t) => t.primary))
const inMenu = computed(() => kept.value.filter((t) => !t.primary).length)

async function keep(targets: OpenTarget[]) {
  await adoptTargets(targets)
  if (!left.value.length) offering.value = false
}

async function use(target: OpenTarget) {
  // In the app this also runs it, in this workspace.
  await keep([target])
}

async function keepShown() {
  await keep([...suggested.value, ...more.value.filter((s) => chosen.has(s.target.id))].map((s) => s.target))
  offering.value = false
}

async function notNow() {
  await adoptTargets([])
  offering.value = false
}

const cards = [
  { title: 'Main workspace', kind: 'primary', path: '/Users/sandbox/Code/app' },
  { title: 'Workspace · worktree', kind: 'worktree', path: '/Users/sandbox/Code/app-1234-fix-header' },
] as const
</script>

<template>
  <div class="column">
    <div v-for="(card, i) in cards" :key="card.path" class="card workspace" role="button" tabindex="0">
      <div class="row card-head">
        <span class="card-title">{{ card.title }}</span>
        <span class="spacer" />
        <span class="terminal-hint"><Icon name="terminal" /></span>
      </div>
      <div class="path-row">
        <span class="kind-icon"><Icon :name="card.kind" /></span>
        <code class="truncate selectable">{{ shortPath(card.path) }}</code>
      </div>

      <!-- The offer is made on the first card only; the others just show
           the dashed buttons, so the window isn't repeating itself. -->
      <div v-if="i === 0 && offering && list && (suggested.length || more.length)" class="offer">
        <span>Found on this Mac.</span>
        <span class="spacer" />
        <button class="btn ghost small" @click.stop="notNow">Not now</button>
        <button class="btn small" @click.stop="keepShown">Keep these</button>
      </div>

      <div class="actions">
        <button v-for="t in buttons" :key="t.id" class="btn small" @click.stop>
          {{ t.icon }} {{ t.label }}
        </button>
        <button v-if="inMenu" class="btn ghost small" @click.stop>+{{ inMenu }} more</button>
        <button
          v-for="s in suggested"
          :key="s.target.id"
          class="btn small suggestion"
          :title="`Open in ${s.target.label} and keep it`"
          @click.stop="use(s.target)"
        >
          {{ s.target.icon }} {{ s.target.label }}
        </button>
        <button
          v-if="more.length"
          class="btn ghost small"
          :aria-expanded="showMore"
          @click.stop="showMore = !showMore"
        >
          {{ more.length }} more
        </button>
        <span class="spacer" />
        <button v-if="card.kind !== 'primary'" class="btn ghost small danger" @click.stop>Remove</button>
      </div>

      <div v-if="i === 0 && showMore && more.length" class="more" @click.stop>
        <label v-for="s in more" :key="s.target.id" class="more-row">
          <input type="checkbox" :checked="chosen.has(s.target.id)" @change="toggle(s.target.id)" />
          <span class="more-icon">{{ s.target.icon }}</span>
          <span class="more-label">{{ s.target.label }}</span>
          <span class="spacer" />
          <button class="btn ghost small" @click.prevent="use(s.target)">Open</button>
        </label>
      </div>
    </div>
  </div>
</template>

<style scoped>
.column {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
  padding: 12px;
  background: var(--bg);
  overflow-y: auto;
}

.suggestion {
  border-style: dashed;
  background: transparent;
  color: var(--text-dim);
}

.suggestion:hover {
  border-style: solid;
  color: var(--text);
  background: var(--bg-raised);
}

.offer {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
  padding: 7px 8px 7px 10px;
  border-radius: 7px;
  background: color-mix(in srgb, var(--accent) 8%, transparent);
  font-size: 11.5px;
  line-height: 1.4;
  color: var(--text-dim);
}

.more {
  display: flex;
  flex-direction: column;
  margin-top: 8px;
  border: 1px solid var(--border-soft);
  border-radius: 7px;
  background: var(--bg);
  cursor: default;
}

.more-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 6px 4px 9px;
  font-size: 12px;
}

.more-row + .more-row {
  border-top: 1px solid var(--border-soft);
}

.more-icon {
  width: 16px;
  text-align: center;
}
</style>
