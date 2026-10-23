<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue'
import BranchList from '../../components/BranchList.vue'

/**
 * The branch list with a script of changes played into it: a step every
 * `every` milliseconds, round and round, or one at a time with Next. Each
 * step changes the stores or the backend's answers the way the app would
 * see it happen, and says what it did.
 */
export interface Step {
  label: string
  run: () => unknown
}

const props = defineProps<{ steps: Step[]; every?: number }>()

const playing = ref(true)
const at = ref(-1)
let timer = 0

async function next() {
  at.value = (at.value + 1) % props.steps.length
  await props.steps[at.value].run()
}

function schedule() {
  window.clearInterval(timer)
  if (playing.value) timer = window.setInterval(() => void next(), props.every ?? 2400)
}

function toggle() {
  playing.value = !playing.value
  schedule()
}

/** A step by hand restarts the clock, so the next one doesn't land on it. */
function step() {
  void next()
  schedule()
}

onMounted(schedule)
onUnmounted(() => window.clearInterval(timer))
</script>

<template>
  <div class="steps">
    <BranchList />
    <div class="controls">
      <button class="btn small" @click="toggle">{{ playing ? 'Pause' : 'Play' }}</button>
      <button class="btn small" @click="step">Next step</button>
      <span class="truncate faint">
        {{ at < 0 ? 'Starting…' : `${at + 1}/${steps.length} · ${steps[at].label}` }}
      </span>
    </div>
  </div>
</template>

<style scoped>
.steps {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.steps > :deep(.pane) {
  flex: 1;
  min-height: 0;
}
.controls {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-top: 1px solid var(--border);
  font-size: 12px;
}
</style>
