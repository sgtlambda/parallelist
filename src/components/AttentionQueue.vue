<script setup lang="ts">
import { AnimatePresence, MotionConfig, motion } from 'motion-v'
import { computed, reactive, watch } from 'vue'
import Icon from './Icon.vue'
import TerminalMark from './TerminalMark.vue'
import { goToItem, nextWaiting, queue, working, type QueueItem } from '../attention'
import { api } from '../api'
import { basename, splitWorkspaceName } from '../format'
import { state } from '../store'
import { boxTasks } from '../tasks'

/**
 * The strip along the bottom of the window: one card per terminal waiting
 * for the user, the longest-waiting on the left, newcomers joining on the
 * right; then, past a divider, the ones working. Always there, empty or
 * not, so the panes above never change height — and the terminal in them
 * never reflows — as things come and go.
 */

/** Both lines as one list, so a card moving from working to waiting is the
 *  same element sliding over rather than one leaving and another arriving.
 *  The divider between them is a row of its own, keyed once, so it stays
 *  put while the cards either side of it change. */
type Row = { key: string; item: QueueItem } | { key: 'divider'; item: null }
const cards = computed(() => [...queue.value, ...working.value])
const rows = computed<Row[]>(() => [
  ...queue.value.map((item) => ({ key: item.path, item })),
  ...(queue.value.length && working.value.length ? [{ key: 'divider' as const, item: null }] : []),
  ...working.value.map((item) => ({ key: item.path, item })),
])

/**
 * What each waiting terminal last asked, from Claude Code's transcript,
 * once per wait. Asked a moment after the wait begins and once more later:
 * the status that starts it can arrive before the transcript has the turn.
 */
const questions = reactive(new Map<string, string | null>())
const ASK_AFTER_MS = [300, 2500]

function waitKey(item: QueueItem) {
  return `${item.path}\0${item.since}`
}

watch(
  queue,
  (items) => {
    const current = new Set(items.map(waitKey))
    for (const key of questions.keys()) if (!current.has(key)) questions.delete(key)
    for (const item of items) {
      const key = waitKey(item)
      if (questions.has(key)) continue
      questions.set(key, null)
      for (const delay of ASK_AFTER_MS) {
        window.setTimeout(() => {
          if (!questions.has(key)) return
          api
            .terminalQuestion(item.path)
            .then((q) => q && questions.has(key) && questions.set(key, q))
            .catch(() => {})
        }, delay)
      }
    }
  },
  { immediate: true },
)

/** The card's second line: what a working program says it does, or what a
 *  waiting one asked. A blocked one's own message, if it sent one, is the
 *  more current of the two. */
function said(item: QueueItem): string | null {
  if (item.kind === 'working') return item.message
  if (item.kind === 'blocked' && item.message) return item.message
  return questions.get(waitKey(item)) ?? null
}

const TITLES = {
  blocked: 'Blocked — waiting for your answer',
  ready: 'Ready — finished, waiting for your reply',
  working: 'Working',
}

/** How long it has waited or worked, ticking with the store's clock. */
function elapsed(item: QueueItem): string {
  const seconds = Math.max(0, (state.now - item.since) / 1000)
  if (seconds < 60) return `${Math.floor(seconds)}s`
  const minutes = seconds / 60
  if (minutes < 60) return `${Math.floor(minutes)}m`
  const hours = minutes / 60
  if (hours < 24) return `${Math.floor(hours)}h`
  return `${Math.floor(hours / 24)}d`
}

/** The directory name as branch and project, when it is named that way. */
function named(item: QueueItem) {
  const project = state.config?.projects.find((p) => p.id === item.entry.projectId)
  return project ? splitWorkspaceName(item.path, project.path, item.entry.branch.name) : null
}

function title(item: QueueItem): string {
  const where = `${item.entry.projectName} · ${item.entry.branch.name}\n${item.path}`
  const text = said(item)
  return [TITLES[item.kind], text, where].filter(Boolean).join('\n\n')
}

/** Joining: in from the right. Leaving: down out of the strip, while the
 *  cards behind close the gap (`popLayout`). */
const arriving = { opacity: 0, x: 24 }
const settled = { opacity: 1, x: 0, y: 0 }
const spring = { type: 'spring', stiffness: 520, damping: 42, mass: 0.8 } as const
const leaving = { opacity: 0, y: 10, transition: { duration: 0.18, ease: [0.4, 0, 1, 1] } }

</script>

<template>
  <div class="attention-queue">
    <MotionConfig reduced-motion="user">
      <AnimatePresence mode="popLayout">
        <template v-for="{ key, item } in rows" :key="key">
          <motion.span
            v-if="!item"
            class="attention-divider"
            layout
            :initial="{ opacity: 0 }"
            :animate="{ opacity: 1 }"
            :exit="{ opacity: 0 }"
          />
          <!-- A div, not a <button>: WebKit lays a button's children out in an
               anonymous box that shrinks to fit them, so its lines grew past
               the card instead of truncating. -->
          <motion.div
            v-else
            class="attention-card"
            :class="[item.kind, { here: item.here }]"
            role="button"
            tabindex="0"
            :title="title(item)"
            layout
            :initial="arriving"
            :animate="settled"
            :exit="leaving"
            :transition="spring"
            @click="goToItem(item)"
            @keydown.enter.prevent="goToItem(item)"
            @keydown.space.prevent="goToItem(item)"
          >
            <!-- The workspace's directory name, not its branch: it is what the
                 user knows the terminal by, and the project is usually in it
                 (`app-1234-fix-header`). Named `<project>-<branch>`, the
                 branch part leads and the project follows it, the first to
                 give way. Project, branch and path are in the tooltip. -->
            <span class="row">
              <TerminalMark :paths="[item.path]" />
              <span v-if="named(item)" class="attention-workspace split">
                <span class="truncate attention-branch">{{ named(item)!.branch }}</span>
                <span class="truncate attention-project">{{ named(item)!.project }}</span>
              </span>
              <span v-else class="truncate attention-workspace">
                {{ basename(item.path) }}
              </span>
              <span class="spacer" />
              <span class="attention-age faint">{{ elapsed(item) }}</span>
            </span>
            <span class="row">
              <span class="truncate attention-said" :class="{ faint: !said(item) }">
                {{ said(item) ?? (item.kind === 'working' ? 'Working…' : '') }}
              </span>
              <span class="spacer" />
              <kbd v-if="item.path === nextWaiting?.path" class="attention-kbd" title="Go here with ⌘↵"
                >⌘↵</kbd
              >
              <span v-else-if="item.here" class="attention-here faint">here</span>
            </span>
          </motion.div>
        </template>
      </AnimatePresence>
    </MotionConfig>
    <!-- The task box takes the middle of an empty strip, so the logo gives way. -->
    <span v-if="!cards.length && !boxTasks.length" class="attention-empty faint">
      <Icon name="logo" :size="18" class="logo-glow" />
    </span>
  </div>
</template>
