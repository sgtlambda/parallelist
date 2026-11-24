<script setup lang="ts">
import { ref } from 'vue'
import Icon from '../../components/Icon.vue'
import Onboarding from '../../components/Onboarding.vue'
import FirstRunChecklist from '../firstRun/FirstRunChecklist.vue'
import { DETAIL_MIN, LIST_MIN, listWidth, sidebarWidth } from '../../store'

/**
 * A first launch's window, as `Window` draws the onboarding, with the
 * checklist concept as a dialog over it.
 */
defineProps<{ concept: 'checklist' }>()
const asking = ref(true)
</script>

<template>
  <div
    class="app"
    :style="{
      '--sidebar-w': `${sidebarWidth}px`,
      '--list-w': `${listWidth}px`,
      '--list-min': `${LIST_MIN}px`,
      '--detail-min': `${DETAIL_MIN}px`,
      '--queue-h': '0px',
    }"
  >
    <div class="titlebar wide">
      <button class="btn ghost small icon" title="Settings (⌘,)"><Icon name="settings" /></button>
    </div>
    <!-- Behind the dialog only once it is done: with no targets chosen the
         onboarding would ask the same question itself. -->
    <Onboarding v-if="!asking" />
    <FirstRunChecklist v-else @close="asking = false" />
  </div>
</template>
