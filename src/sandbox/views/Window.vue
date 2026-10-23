<script setup lang="ts">
import AttentionQueue from '../../components/AttentionQueue.vue'
import Onboarding from '../../components/Onboarding.vue'
import Icon from '../../components/Icon.vue'
import TaskBox from '../../components/TaskBox.vue'
import { DETAIL_MIN, LIST_MIN, listWidth, sidebarWidth } from '../../store'

/**
 * The window's grid, with only what a scenario is about in it. The attention
 * queue is a strip across the bottom of the three columns and sizes itself to
 * the grid's last row, so it needs this to be the height it is in the app.
 * The task box sits over its right end, or its middle when no cards are there.
 * The onboarding takes the whole grid, with the title bar's gear and no queue.
 */
defineProps<{ show: 'onboarding' | 'queue' }>()
</script>

<template>
  <div
    class="app"
    :style="{
      '--sidebar-w': `${sidebarWidth}px`,
      '--list-w': `${listWidth}px`,
      '--list-min': `${LIST_MIN}px`,
      '--detail-min': `${DETAIL_MIN}px`,
      ...(show === 'onboarding' ? { '--queue-h': '0px' } : {}),
    }"
  >
    <template v-if="show === 'onboarding'">
      <div class="titlebar wide">
        <button class="btn ghost small icon" title="Settings (⌘,)"><Icon name="settings" /></button>
      </div>
      <Onboarding />
    </template>
    <p v-else class="above faint">The sidebar, branch list and detail pane are above.</p>
    <AttentionQueue v-if="show === 'queue'" />
    <TaskBox v-if="show === 'queue'" />
  </div>
</template>

<style scoped>
.above {
  grid-column: 1 / -1;
  grid-row: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
}
</style>
