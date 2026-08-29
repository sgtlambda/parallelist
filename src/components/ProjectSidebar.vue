<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { api } from '../api'
import { basename } from '../format'
import { ref } from 'vue'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import {
  dropCached,
  loadConfig,
  refresh,
  saveConfig,
  selectProject,
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

  const projects = [...state.config.projects]
  const [moved] = projects.splice(from, 1)
  // Removing the dragged row shifts everything after it up by one.
  projects.splice(to > from ? to - 1 : to, 0, moved)
  if (projects.every((r, i) => r.id === state.config!.projects[i].id)) return

  await saveConfig({ ...state.config, projects })
}

async function addRepo() {
  const picked = await open({ directory: true, multiple: false, title: "Choose the project's main clone" })
  if (typeof picked !== 'string') return
  try {
    const repo = await api.addProject(picked)
    await loadConfig()
    await selectProject(repo.id)
  } catch (e) {
    emit('error', String(e))
  }
}

async function forget(id: string) {
  await api.removeProject(id)
  dropCached(id)
  await loadConfig()
  await refresh({ fetch: false })
}

const menu = ref<{ open: boolean; x: number; y: number; id: string | null }>({
  open: false,
  x: 0,
  y: 0,
  id: null,
})

const menuItems: MenuItem[] = [
  { id: 'settings', label: 'Project settings…' },
  { id: 'remove', label: 'Remove project', separator: true, danger: true },
]

function openMenu(id: string, event: MouseEvent) {
  menu.value = { open: true, x: event.clientX, y: event.clientY, id }
}

async function onMenuSelect(action: string) {
  const id = menu.value.id
  if (!id) return
  if (action === 'settings') {
    await selectProject(id)
    settingsOpen.value = true
  } else if (action === 'remove') {
    // Only un-manages it; nothing on disk is touched.
    await forget(id)
  }
}
</script>

<template>
  <aside class="pane sidebar">
    <div class="pane-body">
      <div class="section-title">Projects</div>
      <button
        v-for="(repo, index) in state.config?.projects ?? []"
        :key="repo.id"
        class="project-item"
        :class="{
          selected: repo.id === state.config?.selectedProjectId,
          dragging: dragging === index,
          'drop-before': dropAt === index,
          'drop-after': dropAt === index + 1 && index === (state.config?.projects.length ?? 0) - 1,
        }"
        draggable="true"
        @click="selectProject(repo.id)"
        @contextmenu.prevent="openMenu(repo.id, $event)"
        @dragstart="onDragStart(index, $event)"
        @dragover="onDragOver(index, $event)"
        @drop.prevent="onDrop"
        @dragend="dragging = null; dropAt = null"
      >
        <span class="project-name truncate">{{ repo.name }}</span>
        <span class="project-sub truncate">{{ basename(repo.path) }}</span>
      </button>

      <p v-if="!state.config?.projects.length" class="empty" style="padding: 20px 14px">
        Add the main clone of a project to get started.
      </p>
    </div>

    <div class="pane-header" style="border-top: 1px solid var(--border-soft); border-bottom: none">
      <button class="btn ghost small" @click="addRepo">+ Add project</button>
      <span class="spacer" />
      <button class="btn ghost small" title="Settings" @click="settingsOpen = true">⚙</button>
    </div>
    <ContextMenu
      :open="menu.open"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @close="menu.open = false"
      @select="onMenuSelect"
    />
  </aside>
</template>
