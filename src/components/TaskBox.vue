<script setup lang="ts">
import { AnimatePresence, MotionConfig, motion } from 'motion-v'
import { computed } from 'vue'
import Icon from './Icon.vue'
import {
  boxTasks,
  dismissTask,
  isReady,
  openTasks,
  revealTask,
  taskLine,
  type LiveTask,
} from '../tasks'
import { openTargetsFor } from '../store'
import { openTaskIn, targetBusy, targetTitle } from '../openIn'

const emit = defineEmits<{ (e: 'error', message: string): void }>()

/**
 * The box at the right end of the attention queue's strip: one featured
 * task, and a count of the rest. The featured
 * task is the first of `boxTasks` — something asking for attention, else the
 * oldest running — so it only changes when that task is dealt with or
 * finishes, never because another one printed a line. Clicking opens the
 * task dialog on it.
 */
const featured = computed(() => boxTasks.value[0] ?? null)
const others = computed(() => Math.max(0, boxTasks.value.length - 1))

/**
 * Motion, via motion-v. The box rises into the strip and, once nothing is
 * left to show, sinks back out of it. `AnimatePresence` is
 * Vue's own `<Transition>` underneath, so the leaving box keeps its last
 * render — the finished task's checkmark — for the whole exit.
 *
 * Inside, a different featured task cross-fades in while `layout` eases the
 * lines to their new place (the Ready buttons, centred in the strip, push
 * them up). The fades
 * overlap: `popLayout` pins the outgoing content where it was so the incoming
 * content takes its place at once. `wait` faded one fully out before the
 * other came in, which read as a blank beat whenever a finished task handed
 * the slot to one still running. A task changing state in place is not a
 * swap: the spinner just gives way to the check.
 */
const tucked = { opacity: 0, y: 6, scale: 0.97 }
const settled = { opacity: 1, y: 0, scale: 1 }
const spring = { type: 'spring', stiffness: 520, damping: 40, mass: 0.8 } as const
// Accelerating out: it is leaving, not arriving.
const sink = { ...tucked, transition: { duration: 0.22, ease: [0.4, 0, 1, 1] } }

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
  <!-- "user": macOS's Reduce motion drops the movement and keeps the fades. -->
  <MotionConfig reduced-motion="user">
    <AnimatePresence>
      <!-- The strip's own colour, faded in from the left: whatever cards
           run on under the box are covered rather than showing through. -->
      <motion.div
        v-if="featured"
        key="task-backdrop"
        class="task-backdrop"
        :initial="{ opacity: 0 }"
        :animate="{ opacity: 1, transition: { duration: 0.18 } }"
        :exit="{ opacity: 0, transition: { duration: 0.22 } }"
      />
      <motion.div
        v-if="featured"
        key="task-box"
        class="task-box"
        layout
        :initial="tucked"
        :animate="settled"
        :exit="sink"
        :transition="spring"
        role="button"
        tabindex="0"
        title="Show all tasks and their logs"
        @click="openTasks(featured.id)"
        @keydown.enter="openTasks(featured.id)"
      >
        <AnimatePresence mode="popLayout" :initial="false">
          <motion.div
            :key="featured.id"
            class="task-box-body"
            layout="position"
            :initial="{ opacity: 0 }"
            :animate="{ opacity: 1, transition: { duration: 0.18, ease: 'easeOut' } }"
            :exit="{ opacity: 0, transition: { duration: 0.12, ease: 'easeIn' } }"
          >
            <div class="row">
              <span v-if="featured.status === 'running'" class="spin tiny" />
              <motion.span
                v-else-if="featured.status === 'done'"
                class="task-icon done"
                :initial="{ scale: 0.4, opacity: 0 }"
                :animate="{ scale: 1, opacity: 1 }"
                :transition="{ type: 'spring', stiffness: 600, damping: 22 }"
              >
                <Icon name="check" :size="12" class="icon-draw" />
              </motion.span>
              <span v-else class="task-icon" :class="featured.status">
                {{ featured.status === 'failed' ? '✕' : '◷' }}
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
                :disabled="targetBusy(target, featured.path ?? '')"
                :title="targetTitle(target, featured.path ?? '')"
                @click="openIn(target.id, featured)"
              >
                {{ target.icon }} {{ target.label }}
              </button>
              <button class="btn ghost small" @click="reveal(featured)">Show</button>
            </div>
          </motion.div>
        </AnimatePresence>
      </motion.div>
    </AnimatePresence>
  </MotionConfig>
</template>
