<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { api } from '../api'
import { basename } from '../format'
import { ref } from 'vue'
import {
  dropCached,
  loadConfig,
  refresh,
  saveConfig,
  selectRepo,
  settingsOpen,
  state,
} from '../store'

const emit = defineEmits<{ (e: 'error', message: string): void }>()

const dragging = ref<number | null>(null)
const dropAt = ref<number | null>(null)

function onDragStart(index: number, event: DragEvent) {
  dragging.value = index
  // Required for the drag to start at all in WebKit.
  event.dataTransfer?.setData('text/plain', String(index))
  if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
}

function onDragOver(index: number, event: DragEvent) {
  if (dragging.value === null) return
  event.preventDefault()
  // Insert after the row when the pointer is past its midpoint, so dragging
  // to the very bottom can reach the last position.
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect()
  dropAt.value = event.clientY > box.top + box.height / 2 ? index + 1 : index
}

async function onDrop() {
  const from = dragging.value
  const to = dropAt.value
  dragging.value = null
  dropAt.value = null
  if (from === null || to === null || !state.config) return

  const repos = [...state.config.repos]
  const [moved] = repos.splice(from, 1)
  // Removing the dragged row shifts everything after it up by one.
  repos.splice(to > from ? to - 1 : to, 0, moved)
  if (repos.every((r, i) => r.id === state.config!.repos[i].id)) return

  await saveConfig({ ...state.config, repos })
}

async function addRepo() {
  const picked = await open({ directory: true, multiple: false, title: "Choose the project's main clone" })
  if (typeof picked !== 'string') return
  try {
    const repo = await api.addRepo(picked)
    await loadConfig()
    await selectRepo(repo.id)
  } catch (e) {
    emit('error', String(e))
  }
}

async function forget(id: string) {
  await api.removeRepo(id)
  dropCached(id)
  await loadConfig()
  await refresh({ fetch: false })
}
</script>

<template>
  <aside class="pane sidebar">
    <div class="pane-body">
      <div class="section-title">Projects</div>
      <button
        v-for="(repo, index) in state.config?.repos ?? []"
        :key="repo.id"
        class="repo-item"
        :class="{
          selected: repo.id === state.config?.selectedRepoId,
          dragging: dragging === index,
          'drop-before': dropAt === index,
          'drop-after': dropAt === index + 1 && index === (state.config?.repos.length ?? 0) - 1,
        }"
        draggable="true"
        @click="selectRepo(repo.id)"
        @contextmenu.prevent="forget(repo.id)"
        @dragstart="onDragStart(index, $event)"
        @dragover="onDragOver(index, $event)"
        @drop.prevent="onDrop"
        @dragend="dragging = null; dropAt = null"
      >
        <span class="repo-name truncate">{{ repo.name }}</span>
        <span class="repo-sub truncate">{{ basename(repo.path) }}</span>
      </button>

      <p v-if="!state.config?.repos.length" class="empty" style="padding: 20px 14px">
        Add the main clone of a project to get started.
      </p>
    </div>

    <div class="pane-header" style="border-top: 1px solid var(--border-soft); border-bottom: none">
      <button class="btn ghost small" @click="addRepo">+ Add project</button>
      <span class="spacer" />
      <button class="btn ghost small" title="Settings" @click="settingsOpen = true">⚙</button>
    </div>
  </aside>
</template>
