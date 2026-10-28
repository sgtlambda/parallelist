<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api } from '../api'
import { useEscape } from '../useEscape'

const props = defineProps<{ ask: { running: number; waiting: number } | null }>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(
  () => !!props.ask,
  () => emit('close'),
)

const quit = ref<HTMLButtonElement | null>(null)
watch(
  () => props.ask,
  (ask) => ask && void nextTick(() => quit.value?.focus()),
)

const summary = computed(() => {
  const n = props.ask?.running ?? 0
  return n === 1 ? '1 terminal is running.' : `${n} terminals are running.`
})

const waiting = computed(() => {
  const n = props.ask?.waiting ?? 0
  if (!n) return ''
  return n === 1 ? '1 of them is waiting for an answer.' : `${n} of them are waiting for an answer.`
})
</script>

<template>
  <div v-if="ask" class="scrim" @click.self="emit('close')">
    <div class="modal" role="alertdialog" aria-labelledby="quit-title">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span id="quit-title">Quit parallelist?</span>
      </div>
      <div class="modal-body">
        <p>
          {{ summary }} {{ waiting }}
          Quitting ends every shell and the programs running in them.
        </p>
      </div>
      <div class="modal-footer">
        <span class="spacer" />
        <button class="btn" type="button" @click="emit('close')">Cancel</button>
        <button ref="quit" class="btn primary danger" type="button" @click="api.quit()">
          Quit
        </button>
      </div>
    </div>
  </div>
</template>
