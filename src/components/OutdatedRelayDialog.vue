<script setup lang="ts">
import { ref } from 'vue'
import { api } from '../api'
import { state } from '../store'
import { useEscape } from '../useEscape'

/**
 * Shown at launch when this build's terminals are held by a relay from an
 * older version of parallelist, which this one can't talk to. Ending them
 * restarts the app with a new relay; until then, terminals opened here end
 * with the app.
 */
const props = defineProps<{ build: string | null }>()
const emit = defineEmits<{ (e: 'close'): void }>()

useEscape(() => !!props.build && !ending.value, () => emit('close'))

const ending = ref(false)

async function endAndRestart() {
  ending.value = true
  try {
    await api.replaceOutdatedRelay()
  } catch (e) {
    state.error = String(e)
    ending.value = false
  }
}
</script>

<template>
  <div v-if="build" class="scrim" @click.self="!ending && emit('close')">
    <div class="modal" role="alertdialog" aria-labelledby="outdated-title">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span id="outdated-title">Terminals from an older version</span>
      </div>
      <div class="modal-body">
        <p>
          Terminals started by an older version of this build are still
          running. This version can't take them over. Until they are ended,
          terminals opened here end when parallelist quits.
        </p>
      </div>
      <div class="modal-footer">
        <span class="spacer" />
        <button class="btn" type="button" :disabled="ending" @click="emit('close')">Not now</button>
        <button class="btn primary danger" type="button" :disabled="ending" @click="endAndRestart">
          End them and restart
        </button>
      </div>
    </div>
  </div>
</template>
