<script setup lang="ts">
import { computed } from 'vue'
import Icon from './Icon.vue'
import { projectTerminalMark, terminalMark } from '../terminalState'

/** The terminal mark beside a project, a branch row, a workspace card or a
 *  palette entry: nothing unless a shell is running, blinking while
 *  something works in one, boxed while one waits for the user. Given a
 *  project, it covers all of that project's terminals. */
const props = withDefaults(defineProps<{ paths?: string[]; projectId?: string; size?: number }>(), {
  size: 12,
})

const mark = computed(() =>
  props.projectId ? projectTerminalMark(props.projectId) : terminalMark(props.paths ?? []),
)
</script>

<template>
  <span v-if="mark" class="terminal-mark" :class="mark.box" :title="mark.title">
    <Icon :name="mark.icon" :size="size" />
  </span>
</template>
