<script setup lang="ts">
import { homeDir } from '@tauri-apps/api/path'
import { onMounted, onUnmounted, ref } from 'vue'
import BranchList from './components/BranchList.vue'
import CleanupDialog from './components/CleanupDialog.vue'
import CreateDialog from './components/CreateDialog.vue'
import DetailPane from './components/DetailPane.vue'
import RepoSidebar from './components/RepoSidebar.vue'
import SettingsDialog from './components/SettingsDialog.vue'
import { setHomeDirectory } from './format'
import {
  cleanupOpen,
  listenProgress,
  loadConfig,
  refresh,
  settingsOpen,
  startBackgroundSync,
  state,
  stopBackgroundSync,
} from './store'

const createOpen = ref(false)
/** null means "offer everything stale"; a list means "just these". */
const removalPaths = ref<string[] | null>(null)

function openCleanup(paths: string[] | null) {
  removalPaths.value = paths
  cleanupOpen.value = true
}

/** Suppress WebKit's page context menu, which is pure web-app tell — except
 *  over editable or selectable text, where Cut/Copy/Paste is the right menu. */
function onContextMenu(event: MouseEvent) {
  const target = event.target as HTMLElement | null
  if (target?.closest('input, textarea, [contenteditable], .selectable')) return
  event.preventDefault()
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
  <div class="app">
    <!-- Full-width strip under the overlay title bar: clears the traffic
         lights and gives the window somewhere to be dragged from. -->
    <div class="titlebar" data-tauri-drag-region />

    <RepoSidebar @error="state.error = $event" />
    <BranchList />
    <DetailPane
      @create="createOpen = true"
      @remove="openCleanup($event)"
      @error="state.error = $event"
    />

    <CreateDialog :open="createOpen" @close="createOpen = false" />
    <CleanupDialog
      :open="cleanupOpen"
      :paths="removalPaths"
      @close="cleanupOpen = false"
    />
    <SettingsDialog :open="settingsOpen" @close="settingsOpen = false" />

    <div v-if="state.toast" class="toast">{{ state.toast }}</div>
  </div>
</template>
