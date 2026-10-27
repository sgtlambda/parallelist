<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api, onDragDrop } from '../api'
import { openFolder } from '../store'
import { pastePaths, shownTerminal } from '../terminal'

/**
 * Where something dragged from Finder can be dropped. Two areas, each from
 * the top of the window down to the attention queue:
 *
 * - **projects**, over the sidebar and the branch list, for folders only:
 *   each goes to its project or workspace, or is added as a project
 *   (`open_folder`). With no terminal on screen it takes the whole width.
 * - **terminal**, over the detail pane while it shows a terminal: the paths
 *   are pasted into it, folders and files alike.
 *
 * Anything with a file in it gets only the terminal, and dropped anywhere
 * else does nothing.
 *
 * The drop goes to Tauri's native handler, never the page, so nothing here
 * takes the pointer: where the pointer is comes from the events. On macOS
 * their position is the web view's points, which are CSS pixels, although
 * Tauri types it as physical (wry's `dragging_updated` reads it off AppKit
 * and passes it on unscaled).
 */

type Kind = 'folders' | 'files'
type Area = { id: 'projects' | 'terminal'; left: number; top: number; width: number; height: number }

/** What is being dragged; `null` while there is no drag, or while the
 *  backend is still being asked. */
const kind = ref<Kind | null>(null)
/** How many paths are dragged. */
const count = ref(0)
const areas = ref<Area[]>([])
/** The area under the pointer. */
const over = ref<Area['id'] | null>(null)
/** The terminal the paths would be pasted into. */
let terminal: string | null = null

/** Settles to what the current drag carries. */
let classified: Promise<Kind> | null = null
/** Bumped by every drag, so an answer about an earlier one is ignored. */
let drag = 0

/** Measured on every event: panes are resized and terminals come and go,
 *  and a drag over the window sends a steady stream of them anyway. */
function measure(x: number, y: number) {
  const bottom = document.querySelector('.attention-queue')?.getBoundingClientRect().top ?? window.innerHeight
  const shown = shownTerminal()
  terminal = shown?.path ?? null
  const split = shown ? shown.pane.left : window.innerWidth
  const next: Area[] = []
  if (kind.value !== 'files') next.push({ id: 'projects', left: 0, top: 0, width: split, height: bottom })
  if (shown) next.push({ id: 'terminal', left: split, top: 0, width: window.innerWidth - split, height: bottom })
  areas.value = next
  const hit = next.find((a) => x >= a.left && x < a.left + a.width && y >= a.top && y < a.top + a.height)
  over.value = hit?.id ?? null
}

function reset() {
  drag++
  kind.value = null
  classified = null
  areas.value = []
  over.value = null
  terminal = null
}

const projectsHint = 'Drop to add'
const terminalHint = computed(() => (count.value > 1 ? 'Drop to paste paths' : 'Drop to paste path'))

let stop: (() => void) | null = null
let last = { x: -1, y: -1 }

onMounted(async () => {
  stop = await onDragDrop(async (event) => {
    if (event.type === 'leave') return reset()
    last = { x: event.position.x, y: event.position.y }
    if (event.type === 'enter') {
      const mine = ++drag
      const { paths } = event
      count.value = paths.length
      classified = api
        .areFolders(paths)
        .then((dirs): Kind => (dirs.length > 0 && dirs.every(Boolean) ? 'folders' : 'files'))
        .catch((): Kind => 'files')
      void classified.then((k) => {
        if (drag !== mine) return
        kind.value = k
        measure(last.x, last.y)
      })
    }
    measure(last.x, last.y)
    if (event.type !== 'drop') return

    const pending = classified
    const at = over.value
    const into = terminal
    reset()
    const k = await pending
    if (at === 'terminal' && into) pastePaths(into, event.paths)
    else if (at === 'projects' && k === 'folders') {
      // In turn, so the last one is where the window lands.
      for (const path of event.paths) await openFolder(path)
    }
  })
})

onUnmounted(() => stop?.())
</script>

<template>
  <template v-if="kind">
    <div
      v-for="a in areas"
      :key="a.id"
      class="drop-area"
      :class="{ over: over === a.id }"
      :style="{ left: `${a.left}px`, top: `${a.top}px`, width: `${a.width}px`, height: `${a.height}px` }"
    >
      <div class="drop-hint">{{ a.id === 'projects' ? projectsHint : terminalHint }}</div>
    </div>
  </template>
</template>

<style scoped>
/* Laid out by the numbers the pointer is tested against; the outline sits
   inside them, so two areas side by side keep a gap between them.

   The outline and the label are always one colour: a dim blue while the
   area waits, a brighter one under the pointer (towards the text colour, so
   brighter in the dark theme and deeper in the light one). The wash under
   them hides most of the window, so that blue reads over anything. */
.drop-area {
  --drop-color: color-mix(in srgb, var(--accent) 60%, var(--bg));
  --drop-wash: color-mix(in srgb, var(--bg) 88%, transparent);
  position: fixed;
  z-index: 80;
  box-sizing: border-box;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 6px;
  pointer-events: none;
  animation: fade-in 0.12s ease both;
}

.drop-area.over {
  --drop-color: color-mix(in srgb, var(--accent) 80%, var(--text));
}

.drop-area::before {
  content: '';
  position: absolute;
  inset: 6px;
  border: 2px dashed var(--drop-color);
  border-radius: 10px;
  background: var(--drop-wash);
  transition:
    background-color 0.1s ease,
    border-color 0.1s ease;
}

.drop-area.over::before {
  background: color-mix(in srgb, var(--accent) 6%, var(--drop-wash));
}

/* A small tag in the outline's colour. No border and no shadow: a
   see-through patch, darker in the dark theme and lighter in the light
   one, so the words stay readable over whatever the wash lets through. */
.drop-hint {
  position: relative;
  margin: 0 18px;
  padding: 3px 9px;
  text-align: center;
  border-radius: 5px;
  background: var(--label-scrim);
  font-size: 14.4px;
  color: var(--drop-color);
  transition: color 0.1s ease;
}
</style>
