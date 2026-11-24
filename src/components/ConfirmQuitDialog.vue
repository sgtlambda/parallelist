<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { api } from '../api'
import { useEscape } from '../useEscape'

/**
 * Asked on ⌘Q while terminals are running. With a relay holding them, the
 * focused choice keeps them running for the next launch, and ending them
 * takes a click of its own. Without one, quitting can only end them.
 */
const props = defineProps<{ ask: { running: number; waiting: number; persistent: boolean } | null }>()
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
        <p v-if="ask.persistent">
          {{ summary }} {{ waiting }}
          They can keep running, and come back with their output when
          parallelist opens again.
        </p>
        <p v-else>
          {{ summary }} {{ waiting }}
          Quitting ends every shell and the programs running in them.
        </p>
      </div>
      <div class="modal-footer">
        <span class="spacer" />
        <button class="btn" type="button" @click="emit('close')">Cancel</button>
        <template v-if="ask.persistent">
          <button class="btn danger" type="button" @click="api.quit(true)">
            End all and quit
          </button>
          <button ref="quit" class="btn primary" type="button" @click="api.quit(false)">
            Quit, keep terminals
          </button>
        </template>
        <button v-else ref="quit" class="btn primary danger" type="button" @click="api.quit(true)">
          Quit
        </button>
      </div>
    </div>
  </div>
</template>
