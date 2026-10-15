<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { shortPath } from '../format'
import {
  STEP_MARK,
  dismissTask,
  elapsed,
  isReady,
  revealTask,
  selectedTaskId,
  taskById,
  taskGroups,
  taskLine,
  tasksOpen,
  type LiveTask,
} from '../tasks'
import { openTargetsFor, state } from '../store'
import { openTaskIn, targetBusy, targetTitle } from '../openIn'
import type { OpenTarget } from '../api'
import { useEscape } from '../useEscape'

const emit = defineEmits<{ (e: 'error', message: string): void }>()

const task = computed(() => taskById(selectedTaskId.value))

useEscape(
  () => tasksOpen.value,
  () => (tasksOpen.value = false),
)

const logEl = ref<HTMLElement | null>(null)

/** Follow the tail while lines arrive, unless the user has scrolled up to
 *  read something — yanking them back down on every line of `pnpm install`
 *  would make the log unreadable while it runs. */
let following = true

function onScroll() {
  const el = logEl.value
  if (!el) return
  following = el.scrollHeight - el.scrollTop - el.clientHeight < 24
}

function toBottom() {
  void nextTick(() => logEl.value?.scrollTo({ top: logEl.value.scrollHeight }))
}

watch(
  () => [tasksOpen.value, task.value?.id] as const,
  () => {
    following = true
    toBottom()
  },
)

// `lastLine` as well as the length: at the line cap the length stops moving.
watch(
  () => [task.value?.log.length, task.value?.lastLine] as const,
  () => following && toBottom(),
)

const projectName = computed(
  () => state.config?.projects.find((p) => p.id === task.value?.projectId)?.name ?? '',
)

function status(t: LiveTask): string {
  switch (t.status) {
    case 'queued':
      return 'Queued — waiting for the task before it'
    case 'running':
      return `Running · ${elapsed(t)}`
    case 'done':
      return `Finished in ${elapsed(t)} · ${clock(t.finishedAt)}`
    case 'failed':
      return `Failed after ${elapsed(t)} · ${clock(t.finishedAt)}`
  }
}

function clock(ms: number | null): string {
  return ms ? new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''
}

async function openIn(target: OpenTarget, t: LiveTask) {
  try {
    await openTaskIn(target.id, t)
    // Its terminal is now on screen, behind this.
    if (target.kind === 'terminal') tasksOpen.value = false
    await dismissTask(t.id)
  } catch (e) {
    emit('error', String(e))
  }
}

async function reveal(t: LiveTask) {
  tasksOpen.value = false
  await dismissTask(t.id)
  await revealTask(t)
}
</script>

<template>
  <div v-if="tasksOpen" class="scrim" @click.self="tasksOpen = false">
    <div class="modal tasks-modal">
      <div class="modal-header" data-tauri-drag-region="deep">
        <span>Tasks</span>
      </div>

      <div class="tasks-body">
        <nav class="task-list">
          <p v-if="!taskGroups.length" class="empty">Nothing has run yet.</p>
          <template v-for="group in taskGroups" :key="group.label">
            <div class="task-group">{{ group.label }}</div>
            <button
              v-for="t in group.tasks"
              :key="t.id"
              class="task-item"
              :class="{ selected: t.id === selectedTaskId }"
              @click="selectedTaskId = t.id"
            >
              <span v-if="t.status === 'running'" class="spin tiny" />
              <span v-else class="task-icon" :class="t.status">
                {{ { queued: '◷', done: '✓', failed: '✕', running: '' }[t.status] }}
              </span>
              <span class="task-item-text">
                <span class="truncate task-title">{{ t.title }}</span>
                <span class="truncate task-line" :class="{ err: t.status === 'failed' }">
                  {{ taskLine(t) }}
                </span>
              </span>
            </button>
          </template>
        </nav>

        <section v-if="task" class="task-detail">
          <div class="task-detail-head">
            <div class="truncate task-title">{{ task.title }}</div>
            <div class="muted truncate">
              {{ projectName }}
              <template v-if="task.path">
                · <code class="selectable" :title="task.path">{{ shortPath(task.path) }}</code>
              </template>
            </div>
            <div class="faint">{{ status(task) }}</div>
          </div>
          <div ref="logEl" class="log task-log selectable" @scroll="onScroll">
            <div
              v-for="line in task.log"
              :key="line.n"
              :class="{ err: line.error, step: line.text.startsWith(STEP_MARK) }"
            >
              {{ line.text }}
            </div>
            <div v-if="task.status === 'running'" class="muted">…</div>
            <div v-else-if="!task.log.length" class="muted">No output.</div>
          </div>
        </section>
        <section v-else class="task-detail">
          <p class="empty">Pick a task to see its log.</p>
        </section>
      </div>

      <div class="modal-footer">
        <template v-if="task && isReady(task)">
          <button
            v-for="target in openTargetsFor(task.projectId).filter((t) => !t.requiresPr)"
            :key="target.id"
            class="btn small"
            :disabled="targetBusy(target, task.path ?? '')"
            :title="targetTitle(target, task.path ?? '')"
            @click="openIn(target, task)"
          >
            {{ target.icon }} {{ target.label }}
          </button>
          <button class="btn ghost small" @click="reveal(task)">Show</button>
        </template>
        <span class="spacer" />
        <button v-if="task?.attention" class="btn" @click="dismissTask(task.id)">Dismiss</button>
        <button class="btn" @click="tasksOpen = false">Close</button>
      </div>
    </div>
  </div>
</template>
