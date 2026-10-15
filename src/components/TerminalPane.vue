<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { attach, detach } from '../terminal'

/** Hosts one workspace's terminal. The session lives in `terminal.ts`; this
 *  only lends it a place in the document. */
const props = defineProps<{ path: string }>()

const host = ref<HTMLElement | null>(null)
let current: string | null = null

function show(path: string) {
  if (current) detach(current)
  current = path
  if (host.value) attach(path, host.value)
}

onMounted(() => show(props.path))
watch(
  () => props.path,
  (path) => show(path),
)
onBeforeUnmount(() => {
  if (current) detach(current)
  current = null
})
</script>

<template>
  <div ref="host" class="terminal-host" />
</template>
