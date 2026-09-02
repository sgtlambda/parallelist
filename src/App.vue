<script setup lang="ts">
import { homeDir } from '@tauri-apps/api/path'
import { onMounted, onUnmounted, ref } from 'vue'
import BranchList from './components/BranchList.vue'
import CleanupDialog from './components/CleanupDialog.vue'
import CreateDialog from './components/CreateDialog.vue'
import DetailPane from './components/DetailPane.vue'
import ProjectSidebar from './components/ProjectSidebar.vue'
import SettingsDialog from './components/SettingsDialog.vue'
import { setHomeDirectory } from './format'
import {
  DETAIL_RANGE,
  SIDEBAR_RANGE,
  cleanupBranches,
  cleanupOpen,
  cleanupPaths,
  cleanupTitle,
  detailWidth,
  persistPaneWidths,
  setPaneWidths,
  sidebarWidth,
  listenProgress,
  loadConfig,
  openCleanup,
  refresh,
  settingsOpen,
  startBackgroundSync,
  state,
  stopBackgroundSync,
} from './store'

const createOpen = ref(false)

/** Suppress WebKit's page context menu, which is pure web-app tell — except
 *  over editable or selectable text, where Cut/Copy/Paste is the right menu. */
function onContextMenu(event: MouseEvent) {
  const target = event.target as HTMLElement | null
  if (target?.closest('input, textarea, [contenteditable], .selectable')) return
  event.preventDefault()
}

type Edge = 'sidebar' | 'detail'
const dragging = ref<Edge | null>(null)

/** Drag a splitter. The middle column is `1fr`, so it absorbs the difference;
 *  each edge is clamped so the middle can never be squeezed out. */
function startDrag(edge: Edge, event: PointerEvent) {
  event.preventDefault()
  dragging.value = edge
  const startX = event.clientX
  const startSidebar = sidebarWidth.value
  const startDetail = detailWidth.value
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture(event.pointerId)

  const MIDDLE_MIN = 300

  const onMove = (move: PointerEvent) => {
    const delta = move.clientX - startX
    if (edge === 'sidebar') {
      const room = window.innerWidth - startDetail - MIDDLE_MIN
      const next = clamp(startSidebar + delta, SIDEBAR_RANGE.min, Math.min(SIDEBAR_RANGE.max, room))
      setPaneWidths(next, startDetail)
    } else {
      const room = window.innerWidth - startSidebar - MIDDLE_MIN
      const next = clamp(startDetail - delta, DETAIL_RANGE.min, Math.min(DETAIL_RANGE.max, room))
      setPaneWidths(startSidebar, next)
    }
  }

  const onUp = () => {
    dragging.value = null
    target.releasePointerCapture(event.pointerId)
    target.removeEventListener('pointermove', onMove)
    target.removeEventListener('pointerup', onUp)
    void persistPaneWidths()
  }

  target.addEventListener('pointermove', onMove)
  target.addEventListener('pointerup', onUp)
}

function clamp(value: number, min: number, max: number) {
  return Math.max(min, Math.min(value, Math.max(min, max)))
}

function onKey(event: KeyboardEvent) {
  if (!event.metaKey) return
  switch (event.key) {
    case 'f':
      event.preventDefault()
      document.querySelector<HTMLInputElement>('input[type="search"]')?.focus()
      break
    case 'r':
      event.preventDefault()
      void refresh({ fetch: true })
      break
    case 'n':
      if (state.selectedBranch) {
        event.preventDefault()
        createOpen.value = true
      }
      break
  }
}

let unlistenProgress: (() => void) | null = null

onMounted(async () => {
  // Purely cosmetic: without it paths render absolute rather than under `~`.
  try {
    setHomeDirectory(await homeDir())
  } catch {
    /* leave paths absolute */
  }

  // Subscribe before the first refresh so its phases are not missed.
  unlistenProgress = await listenProgress()
  await loadConfig()
  await refresh({ fetch: true })
  startBackgroundSync()
  window.addEventListener('keydown', onKey)
  window.addEventListener('contextmenu', onContextMenu)
})

onUnmounted(() => {
  unlistenProgress?.()
  stopBackgroundSync()
  window.removeEventListener('keydown', onKey)
  window.removeEventListener('contextmenu', onContextMenu)
})
</script>

<template>
  <div
    class="app"
    :style="{ '--sidebar-w': `${sidebarWidth}px`, '--detail-w': `${detailWidth}px` }"
  >
    <!-- Full-width strip under the overlay title bar: clears the traffic
         lights and gives the window somewhere to be dragged from. -->
    <div class="titlebar" data-tauri-drag-region />

    <ProjectSidebar @error="state.error = $event" />
    <BranchList />
    <DetailPane
      @create="createOpen = true"
      @remove="openCleanup($event, [], 'Remove workspace')"
      @error="state.error = $event"
    />

    <div
      class="splitter"
      :class="{ dragging: dragging === 'sidebar' }"
      :style="{ left: `${sidebarWidth}px` }"
      @pointerdown="startDrag('sidebar', $event)"
    />
    <div
      class="splitter"
      :class="{ dragging: dragging === 'detail' }"
      :style="{ right: `${detailWidth}px` }"
      @pointerdown="startDrag('detail', $event)"
    />

    <CreateDialog :open="createOpen" @close="createOpen = false" />
    <CleanupDialog
      :open="cleanupOpen"
      :paths="cleanupPaths"
      :branches="cleanupBranches"
      :title="cleanupTitle"
      @close="cleanupOpen = false"
    />
    <SettingsDialog :open="settingsOpen" @close="settingsOpen = false" />

    <div v-if="state.toast" class="toast">{{ state.toast }}</div>
  </div>
</template>
