<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { findScenario, groups, idOf } from './scenarios'
import { applyTheme, saveTheme, storedTheme, type Theme } from './theme'

const first = groups[0] && idOf(groups[0], groups[0].scenarios[0])
const current = ref(decodeURIComponent(location.hash.slice(1)) || first || '')
const scenario = computed(() => findScenario(current.value))

const themes: Theme[] = ['system', 'light', 'dark']
const theme = ref<Theme>(storedTheme())
watch(theme, (t) => {
  saveTheme(t)
  applyTheme(t)
})

// Window widths to try a scenario at. 0 is the whole pane.
const widths = [0, 1200, 900, 600, 400]
const width = ref(Number(localStorage.getItem('sandbox:width')) || 0)
watch(width, (w) => localStorage.setItem('sandbox:width', String(w)))

/** Bumped to remount the frame, which resets whatever the scenario seeded. */
const run = ref(0)
const src = computed(() => `sandbox.html?frame=${encodeURIComponent(current.value)}`)

interface Entry {
  kind: 'call' | 'emit' | 'open'
  name: string
  detail: unknown
}
const entries = ref<Entry[]>([])
const logOpen = ref(true)

watch([current, run], () => (entries.value = []))

function onMessage(e: MessageEvent) {
  if (e.origin !== location.origin || e.data?.type !== 'sandbox:log') return
  entries.value = [...entries.value.slice(-199), { kind: e.data.kind, name: e.data.name, detail: e.data.detail }]
}

function onHash() {
  const next = decodeURIComponent(location.hash.slice(1))
  if (next) current.value = next
}

onMounted(() => {
  window.addEventListener('message', onMessage)
  window.addEventListener('hashchange', onHash)
})
onUnmounted(() => {
  window.removeEventListener('message', onMessage)
  window.removeEventListener('hashchange', onHash)
})

const detail = (value: unknown) => (value === null || value === undefined ? '' : JSON.stringify(value))
</script>

<template>
  <div class="shell">
    <nav class="nav">
      <h1>Sandbox</h1>
      <section v-for="group in groups" :key="group.name">
        <h2>{{ group.name }}</h2>
        <a
          v-for="s in group.scenarios"
          :key="s.id"
          :href="`#${idOf(group, s)}`"
          :class="{ on: idOf(group, s) === current }"
        >
          {{ s.title }}
        </a>
      </section>
    </nav>

    <main class="main">
      <header class="bar">
        <div class="heading">
          <strong>{{ scenario?.title ?? 'Nothing selected' }}</strong>
          <span v-if="scenario?.note" class="note">{{ scenario.note }}</span>
        </div>
        <div class="seg" title="Colour scheme">
          <button v-for="t in themes" :key="t" :class="{ on: theme === t }" @click="theme = t">{{ t }}</button>
        </div>
        <div class="seg" title="Frame width">
          <button v-for="w in widths" :key="w" :class="{ on: width === w }" @click="width = w">
            {{ w || 'fill' }}
          </button>
        </div>
        <button class="plain" title="Remount and re-run the scenario's setup" @click="run++">Reset</button>
        <a class="plain" :href="src" target="_blank" title="Open the frame on its own">↗</a>
      </header>

      <div class="viewport">
        <iframe
          v-if="scenario"
          :key="`${current}:${run}`"
          :src="src"
          :style="{ width: width ? `${width}px` : '100%' }"
          title="Scenario"
        />
      </div>

      <footer class="log" :class="{ closed: !logOpen }">
        <button class="plain" @click="logOpen = !logOpen">
          {{ logOpen ? '▾' : '▸' }} Backend and events ({{ entries.length }})
        </button>
        <ol v-if="logOpen">
          <li v-for="(entry, i) in entries" :key="i" :class="entry.kind">
            <span class="kind">{{ entry.kind }}</span>
            <code>{{ entry.name }}</code>
            <span class="detail">{{ detail(entry.detail) }}</span>
          </li>
          <li v-if="!entries.length" class="empty">Nothing yet.</li>
        </ol>
      </footer>
    </main>
  </div>
</template>

<style scoped>
.shell {
  display: grid;
  grid-template-columns: 220px 1fr;
  height: 100%;
  -webkit-user-select: text;
  user-select: text;
}
.nav {
  overflow-y: auto;
  padding: 14px 10px;
  background: var(--bg-sidebar);
  border-right: 1px solid var(--border);
}
h1 {
  margin: 0 8px 12px;
  font-size: 13px;
}
h2 {
  margin: 14px 8px 4px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--text-faint);
}
.nav a {
  display: block;
  padding: 4px 8px;
  border-radius: 5px;
  color: var(--text);
  text-decoration: none;
}
.nav a:hover {
  background: var(--bg-hover);
}
.nav a.on {
  background: var(--accent);
  color: var(--accent-fg);
}
.main {
  display: grid;
  grid-template-rows: auto 1fr auto;
  min-width: 0;
  min-height: 0;
}
.bar {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 14px;
  border-bottom: 1px solid var(--border);
}
.heading {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
}
.note {
  color: var(--text-dim);
  font-size: 12px;
}
.seg {
  display: flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
}
.seg button {
  padding: 3px 9px;
  border: 0;
  background: none;
  color: var(--text-dim);
  font: inherit;
  font-size: 12px;
}
.seg button.on {
  background: var(--bg-active);
  color: var(--text);
}
.plain {
  padding: 3px 9px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: none;
  color: var(--text);
  font: inherit;
  font-size: 12px;
  text-decoration: none;
}
.viewport {
  display: flex;
  justify-content: center;
  min-height: 0;
  background: var(--bg-sidebar);
}
iframe {
  max-width: 100%;
  height: 100%;
  border: 0;
  border-inline: 1px solid var(--border);
  background: var(--bg);
}
.log {
  max-height: 200px;
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--border);
  font-size: 12px;
}
.log > .plain {
  align-self: flex-start;
  margin: 4px 8px;
  border: 0;
  color: var(--text-dim);
}
.log ol {
  margin: 0;
  padding: 0 14px 8px;
  overflow-y: auto;
  list-style: none;
}
.log li {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 1px 0;
}
.kind {
  flex-shrink: 0;
  width: 36px;
  color: var(--text-faint);
}
.emit .kind {
  color: var(--purple);
}
.open .kind {
  color: var(--amber);
}
.detail {
  min-width: 0;
  color: var(--text-dim);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.empty {
  color: var(--text-faint);
}
</style>
