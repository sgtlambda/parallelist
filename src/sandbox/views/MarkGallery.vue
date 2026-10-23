<script setup lang="ts">
import TerminalMark from '../../components/TerminalMark.vue'

/** Every state `terminalMark` can produce, one per row, as the branch list
 *  draws it. The scenario seeds a terminal under each path. */
defineProps<{ rows: { label: string; path: string }[]; projectId: string }>()
</script>

<template>
  <div class="gallery">
    <div v-for="row in rows" :key="row.path" class="row">
      <TerminalMark :paths="[row.path]" :size="14" />
      <span>{{ row.label }}</span>
    </div>
    <div class="row project">
      <TerminalMark :project-id="projectId" :size="14" />
      <span>The project, combining all of the above</span>
    </div>
  </div>
</template>

<style scoped>
.gallery {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 24px;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 22px;
}
.project {
  margin-top: 8px;
  padding-top: 12px;
  border-top: 1px solid var(--border-soft);
}
</style>
