<script setup lang="ts">
import { computed } from 'vue'
import {
  boxTasks,
  dismissTask,
  isReady,
  openTaskIn,
  openTasks,
  revealTask,
  taskLine,
  type LiveTask,
} from '../tasks'
import { openTargetsFor } from '../store'

const emit = defineEmits<{ (e: 'error', message: string): void }>()

/**
 * The top-right box: one featured task, and a count of the rest. The featured
 * task is the first of `boxTasks` — something asking for attention, else the
 * oldest running — so it only changes when that task is dealt with or
 * finishes, never because another one printed a line. Clicking opens the
 * task dialog on it.
 */
const featured = computed(() => boxTasks.value[0] ?? null)
const others = computed(() => Math.max(0, boxTasks.value.length - 1))

async function openIn(targetId: string, task: LiveTask) {
  try {
    await openTaskIn(targetId, task)
    await dismissTask(task.id)
  } catch (e) {
    emit('error', String(e))
  }
}

async function reveal(task: LiveTask) {
  await dismissTask(task.id)
  await revealTask(task)
}
</script>

<template>
  <div
    v-if="featured"
    class="task-box"
    :class="{ failed: featured.status === 'failed', ready: featured.status === 'done' }"
    role="button"
    tabindex="0"
    title="Show all tasks and their logs"
    @click="openTasks(featured.id)"
    @keydown.enter="openTasks(featured.id)"
  >
    <div class="row">
      <span v-if="featured.status === 'running'" class="spin tiny" />
      <span v-else class="task-icon" :class="featured.status">
        {{ { queued: '◷', done: '✓', failed: '✕', running: '' }[featured.status] }}
      </span>
      <span class="truncate task-title">{{ featured.title }}</span>
      <span class="spacer" />
      <button
        v-if="featured.attention"
        class="btn ghost small task-dismiss"
        title="Dismiss"
        @click.stop="dismissTask(featured.id)"
      >
        ✕
      </button>
      <span
        v-if="others"
        class="badge task-count"
        :title="`${others} more task${others === 1 ? '' : 's'}`"
      >
        +{{ others }}
      </span>
    </div>
    <div class="truncate task-line" :class="{ err: featured.status === 'failed' }">
      {{ taskLine(featured) }}
    </div>

    <div v-if="isReady(featured)" class="actions" @click.stop>
      <button
        v-for="target in openTargetsFor(featured.projectId).filter((t) => !t.requiresPr)"
        :key="target.id"
        class="btn small"
        @click="openIn(target.id, featured)"
      >
        {{ target.icon }} {{ target.label }}
      </button>
      <button class="btn ghost small" @click="reveal(featured)">Show</button>
    </div>
  </div>
</template>
