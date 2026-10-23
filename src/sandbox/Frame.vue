<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { log } from './backend'
import { findScenario } from './scenarios'
import { applyTheme, storedTheme } from './theme'

const props = defineProps<{ id: string }>()

const scenario = findScenario(props.id)
const ready = ref(false)
const failure = ref<string | null>(null)
const bound = reactive<Record<string, unknown>>({ ...scenario?.props })

// A dialog loads what it shows when `open` turns true, so one mounted already
// open would show nothing. It is mounted shut and opened once it is there.
const opening = bound.open === true
if (opening) bound.open = false

// The component's own events are logged, and a dialog's `close` closes it, as
// the app around it would. Bound only for what it declares, so a component
// with several roots isn't handed listeners it never emits.
const listeners = computed(() => {
  const declared = (scenario?.component as { emits?: string[] | Record<string, unknown> })?.emits
  const names = Array.isArray(declared) ? declared : Object.keys(declared ?? {})
  return Object.fromEntries(
    names.map((name) => [
      `on${name[0].toUpperCase()}${name.slice(1)}`,
      (...args: unknown[]) => {
        log('emit', name, args)
        if (name === 'close' && 'open' in bound) bound.open = false
      },
    ]),
  )
})

// The page around the frame changes the theme through storage, which the
// two share.
const onStorage = (e: StorageEvent) => e.key === 'sandbox:theme' && applyTheme(storedTheme())

onMounted(() => window.addEventListener('storage', onStorage))
watch(ready, async (now) => {
  if (!now || !opening) return
  await nextTick()
  bound.open = true
})
onUnmounted(() => window.removeEventListener('storage', onStorage))

try {
  Promise.resolve(scenario?.setup?.())
    .catch((e) => (failure.value = String(e)))
    .finally(() => (ready.value = true))
} catch (e) {
  failure.value = String(e)
  ready.value = true
}
</script>

<template>
  <div class="stage" :class="scenario?.layout ?? 'fill'">
    <p v-if="!scenario" class="message">No scenario “{{ id }}”.</p>
    <p v-else-if="failure" class="message">Setup failed: {{ failure }}</p>
    <div
      v-else-if="ready && scenario.layout === 'box'"
      class="box"
      :style="{ width: `${scenario.size?.width ?? 400}px`, height: `${scenario.size?.height ?? 600}px` }"
    >
      <component :is="scenario.component" v-bind="bound" v-on="listeners" />
    </div>
    <component :is="scenario.component" v-else-if="ready" v-bind="bound" v-on="listeners" />
  </div>
</template>

<style scoped>
.stage {
  height: 100%;
  position: relative;
}
.stage.box {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}
.box {
  position: relative;
  display: flex;
  flex-direction: column;
  max-width: 100%;
  max-height: 100%;
  overflow: hidden;
  background: var(--bg-raised);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}
.message {
  padding: 24px;
  color: var(--text-dim);
}
</style>
