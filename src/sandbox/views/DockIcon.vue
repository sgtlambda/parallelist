<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import DockIconArt from './DockIconArt.vue'
import { CURSOR_PERIOD, cursorOn } from '../dockIcon'

/**
 * A playground for the Dock icon while terminals are working: the prompt's
 * cursor blinks while anything is, at the app's cursor rhythm. The
 * badge beside it is the count waiting, as `update_dock` sets it.
 */
const props = defineProps<{ working?: boolean; waiting?: number }>()

const working = ref(props.working ?? true)
const waiting = ref(props.waiting ?? 0)
const period = ref(CURSOR_PERIOD)
const playing = ref(true)

const t = ref(0)
let raf = 0
let last = 0
function tick(now: number) {
  if (last && playing.value) t.value += (now - last) / 1000
  last = now
  raf = requestAnimationFrame(tick)
}
onMounted(() => (raf = requestAnimationFrame(tick)))
onUnmounted(() => cancelAnimationFrame(raf))

const on = computed(() => cursorOn(working.value, t.value, period.value))
const idleOn = true
const workingOn = computed(() => cursorOn(true, t.value, period.value))
const counts = [0, 1, 2, 3]
</script>

<template>
  <div class="playground">
    <!-- Shared by every copy of the icon on the page. -->
    <svg width="0" height="0" class="defs" aria-hidden="true">
      <defs>
        <linearGradient id="dock-body" x1="0" y1="100" x2="0" y2="924" gradientUnits="userSpaceOnUse">
          <stop offset="0" stop-color="#7189FF" />
          <stop offset="1" stop-color="#4636D8" />
        </linearGradient>
        <filter id="dock-shadow" x="-10%" y="-10%" width="120%" height="125%">
          <feGaussianBlur in="SourceAlpha" stdDeviation="10" />
          <feOffset dy="10" />
          <feComponentTransfer><feFuncA type="linear" slope="0.3" /></feComponentTransfer>
          <feMerge><feMergeNode /><feMergeNode in="SourceGraphic" /></feMerge>
        </filter>
        <filter id="dock-lift" x="-20%" y="-20%" width="140%" height="150%">
          <feGaussianBlur in="SourceAlpha" stdDeviation="14" />
          <feOffset dy="12" />
          <feFlood flood-color="#1B1464" flood-opacity="0.35" />
          <feComposite operator="in" in2="SourceAlpha" />
          <feComposite operator="in" />
          <feMerge><feMergeNode /><feMergeNode in="SourceGraphic" /></feMerge>
        </filter>
      </defs>
    </svg>

    <form class="controls" @submit.prevent>
      <fieldset>
        <legend>Working</legend>
        <div class="seg">
          <button type="button" :class="{ on: !working }" @click="working = false">No</button>
          <button type="button" :class="{ on: working }" @click="working = true">Yes</button>
        </div>
      </fieldset>

      <fieldset>
        <legend>Waiting (badge)</legend>
        <div class="seg">
          <button v-for="n in counts" :key="n" type="button" :class="{ on: waiting === n }" @click="waiting = n">
            {{ n }}
          </button>
        </div>
      </fieldset>

      <label>
        <span>
          Period <b>{{ period.toFixed(1) }} s</b>
          <button v-if="period !== CURSOR_PERIOD" type="button" class="reset" @click="period = CURSOR_PERIOD">
            As in the app
          </button>
        </span>
        <input v-model.number="period" type="range" min="0.4" max="3" step="0.1" />
      </label>

      <button type="button" class="play" @click="playing = !playing">{{ playing ? 'Pause' : 'Play' }}</button>

      <p class="cost">
        Two images, idle and blinked off. While anything works, the Dock is redrawn
        <b>{{ working ? (2 / period).toFixed(1) : 0 }}</b> times a second.
      </p>
    </form>

    <div class="previews">
      <section>
        <h3>Large</h3>
        <DockIconArt :cursor-on="on" :size="256" />
      </section>

      <section>
        <h3>In the Dock</h3>
        <div v-for="tone in ['light', 'dark']" :key="tone" class="dock" :class="tone">
          <span class="neighbour" />
          <span class="tile">
            <DockIconArt :cursor-on="on" :size="64" />
            <span v-if="waiting" class="badge">{{ waiting }}</span>
            <span class="dot" />
          </span>
          <span class="neighbour" />
        </div>
      </section>

      <section>
        <h3>Side by side</h3>
        <div class="pair">
          <figure>
            <DockIconArt :cursor-on="idleOn" :size="96" />
            <figcaption>Nothing working</figcaption>
          </figure>
          <figure>
            <DockIconArt :cursor-on="workingOn" :size="96" />
            <figcaption>Something working</figcaption>
          </figure>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.defs {
  position: absolute;
}
.playground {
  display: flex;
  flex-wrap: wrap;
  gap: 24px;
  padding: 20px;
  min-height: 100%;
  box-sizing: border-box;
  background: var(--bg);
  color: var(--text);
}
.controls {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: 240px;
  flex: none;
}
fieldset {
  border: 0;
  margin: 0;
  padding: 0;
}
legend,
label > span:first-child {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11px;
  color: var(--text-dim);
  margin-bottom: 4px;
}
label b {
  color: var(--text);
  font-weight: 500;
}
input[type='range'] {
  width: 100%;
}
.seg {
  display: flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
}
.seg button {
  flex: 1;
  border: 0;
  background: var(--bg-raised);
  color: var(--text);
  padding: 4px 0;
  font: inherit;
  font-size: 12px;
}
.seg button + button {
  border-left: 1px solid var(--border);
}
.seg button.on {
  background: var(--accent);
  color: var(--accent-fg);
}
.reset,
.play {
  border: 1px solid var(--border);
  background: var(--bg-raised);
  color: var(--text);
  border-radius: 5px;
  padding: 1px 6px;
  font: inherit;
  font-size: 11px;
}
.play {
  align-self: flex-start;
  padding: 4px 12px;
  font-size: 12px;
}
.cost {
  margin: 0;
  font-size: 12px;
  color: var(--text-dim);
}
.cost b {
  color: var(--text);
}
.previews {
  display: flex;
  flex-direction: column;
  gap: 24px;
  flex: 1;
  min-width: 300px;
}
h3 {
  margin: 0 0 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-dim);
}
/* Roughly the Dock's material in each appearance; not the app's theme. */
.dock {
  display: inline-flex;
  align-items: flex-start;
  gap: 10px;
  padding: 6px 10px 10px;
  border-radius: 18px;
  margin-right: 12px;
  margin-bottom: 8px;
}
.dock.light {
  background: rgba(246, 246, 248, 0.85);
  box-shadow: 0 0 0 0.5px rgba(0, 0, 0, 0.15);
}
.dock.dark {
  background: rgba(40, 40, 44, 0.9);
  box-shadow: 0 0 0 0.5px rgba(255, 255, 255, 0.15);
}
.neighbour {
  width: 52px;
  height: 52px;
  margin: 6px;
  border-radius: 12px;
  background: rgba(128, 128, 140, 0.35);
}
.tile {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
}
.badge {
  position: absolute;
  top: 2px;
  right: 2px;
  min-width: 20px;
  height: 20px;
  padding: 0 6px;
  box-sizing: border-box;
  border-radius: 10px;
  background: #ff3b30;
  color: #fff;
  font: 600 13px/20px -apple-system, system-ui, sans-serif;
  text-align: center;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
}
.dot {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  margin-top: 1px;
}
.dock.light .dot {
  background: rgba(0, 0, 0, 0.55);
}
.dock.dark .dot {
  background: rgba(255, 255, 255, 0.7);
}
.pair {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
}
figure {
  margin: 0;
  text-align: center;
}
figcaption {
  font-size: 11px;
  color: var(--text-dim);
}
</style>
