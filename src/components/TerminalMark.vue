<script setup lang="ts">
import { computed } from 'vue'
import Icon from './Icon.vue'
import { markOf, projectTerminalMark, terminalMark } from '../terminalState'

/** The terminal mark beside a project, a branch row, a workspace card or a
 *  palette entry: nothing unless a shell is running, blinking while
 *  something works in one, boxed while one waits for the user. Given a
 *  project, it covers all of that project's terminals. */
const props = withDefaults(
  defineProps<{
    paths?: string[]
    projectId?: string
    /** A terminal this window doesn't hold (another build's), drawn from the
     *  state reported for it; `{}` for one whose state is unknown. */
    info?: { status?: string | null; ready?: boolean }
    size?: number
  }>(),
  { size: 12, paths: undefined, projectId: undefined, info: undefined },
)

const mark = computed(() =>
  props.info
    ? markOf({
        working: props.info.status === 'working',
        blocked: props.info.status === 'blocked',
        ready: !!props.info.ready && props.info.status !== 'blocked',
      })
    : props.projectId
      ? projectTerminalMark(props.projectId)
      : terminalMark(props.paths ?? []),
)
</script>

<template>
  <span v-if="mark" class="terminal-mark" :class="mark.box" :title="mark.title">
    <Icon :name="mark.icon" :size="size" />
  </span>
</template>
