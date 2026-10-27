<script setup lang="ts">
import DropTargets from '../../components/DropTargets.vue'
import TerminalPane from '../../components/TerminalPane.vue'

/** The window's layout with only what a drag is measured against: a column
 *  standing in for the sidebar and the branch list, the detail pane with a
 *  terminal in it or not, and the attention queue's strip under both. */
defineProps<{ path?: string }>()
</script>

<template>
  <div class="terminal-drop">
    <div class="rest faint">The sidebar and branch list would be here.</div>
    <section class="pane">
      <TerminalPane v-if="path" :path="path" />
      <p v-else class="rest faint">The branch's overview, with no terminal.</p>
    </section>
    <div class="attention-queue faint">The attention queue would be here.</div>
    <DropTargets />
  </div>
</template>

<style scoped>
.terminal-drop {
  display: grid;
  grid-template-columns: 280px 1fr;
  grid-template-rows: minmax(0, 1fr) var(--queue-h);
  height: 100vh;
}

.rest {
  display: flex;
  align-items: center;
  justify-content: center;
  margin: 0;
  padding: 24px;
  text-align: center;
  border-right: 1px solid var(--border);
}

.pane {
  display: flex;
  min-width: 0;
  min-height: 0;
  padding: 8px;
}

.attention-queue {
  grid-column: 1 / -1;
  display: flex;
  align-items: center;
  justify-content: center;
  border-top: 1px solid var(--border);
}
</style>
