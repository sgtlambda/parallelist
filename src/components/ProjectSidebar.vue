<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { api } from '../api'
import { basename } from '../format'
import { ref } from 'vue'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import TerminalMark from './TerminalMark.vue'
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
/** Set on pointerup after a real drag, so the click that follows is ignored. */
let suppressClick = false

/**
 * Pointer-based reordering.
 *
 * Not HTML5 drag-and-drop: in WKWebView that produces a drag image which does
 * not follow the cursor and a "copy" pointer that never becomes a drop, and
 * the app's global `user-select: none` makes it worse. Pointer capture is what
 * the pane splitters already use.
 */
function onPointerDown(index: number, event: PointerEvent) {
  if (event.button !== 0) return
  const el = event.currentTarget as HTMLElement
  const list = el.parentElement
  if (!list) return

  const startY = event.clientY
  let moved = false
  el.setPointerCapture(event.pointerId)

  const onMove = (move: PointerEvent) => {
    // A few pixels of slop, so a click doesn't register as a drag.
    if (!moved && Math.abs(move.clientY - startY) < 4) return
    if (!moved) {
      moved = true
      dragging.value = index
    }
    const boxes = Array.from(list.querySelectorAll<HTMLElement>('.project-item')).map((row) =>
      row.getBoundingClientRect(),
    )
    const before = boxes.findIndex((box) => move.clientY < box.top + box.height / 2)
    dropAt.value = before === -1 ? boxes.length : before
  }

  const onUp = async () => {
    el.removeEventListener('pointermove', onMove)
    el.removeEventListener('pointerup', onUp)
    el.removeEventListener('pointercancel', onUp)
    el.releasePointerCapture(event.pointerId)

    const to = dropAt.value
    dragging.value = null
    dropAt.value = null
    if (!moved || to === null) return

    suppressClick = true
    requestAnimationFrame(() => {
      suppressClick = false
    })
    await reorder(index, to)
  }

  el.addEventListener('pointermove', onMove)
  el.addEventListener('pointerup', onUp)
  el.addEventListener('pointercancel', onUp)
}

async function reorder(from: number, to: number) {
  if (!state.config) return
  const projects = [...state.config.projects]
  const [moved] = projects.splice(from, 1)
  // Removing the dragged row shifts everything after it up by one.
  projects.splice(to > from ? to - 1 : to, 0, moved)
  if (projects.every((p, i) => p.id === state.config!.projects[i].id)) return
  await saveConfig({ ...state.config, projects })
}

function onSelect(id: string) {
  if (suppressClick) return
  void selectProject(id)
}

async function addProject() {
  const picked = await open({ directory: true, multiple: false, title: "Choose the project's main workspace" })
  if (typeof picked !== 'string') return
  try {
    const added = await api.addProject(picked)
    await loadConfig()
    await selectProject(added.id)
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
  <!-- The whole column moves the window, like a Finder sidebar. Tauri's
       "deep" region never claims clickable elements, so the project rows —
       buttons — still select on click and reorder on drag, and so do the
       footer buttons. The context menu is teleported out of this subtree. -->
  <aside class="pane sidebar" data-tauri-drag-region="deep">
    <div class="pane-body">
      <div class="section-title">Projects</div>
      <button
        v-for="(project, index) in state.config?.projects ?? []"
        :key="project.id"
        class="project-item"
        :class="{
          selected: project.id === state.config?.selectedProjectId,
          dragging: dragging === index,
          'drop-before': dropAt === index,
          'drop-after': dropAt === index + 1 && index === (state.config?.projects.length ?? 0) - 1,
        }"
        @click="onSelect(project.id)"
        @contextmenu.prevent="openMenu(project.id, $event)"
        @pointerdown="onPointerDown(index, $event)"
      >
        <span class="row">
          <span class="project-name truncate">{{ project.name }}</span>
          <span class="spacer" />
          <TerminalMark :project-id="project.id" />
        </span>
        <span class="project-sub truncate">{{ basename(project.path) }}</span>
      </button>

      <p v-if="!state.config?.projects.length" class="empty" style="padding: 20px 14px">
        Add a project's main workspace to get started.
      </p>
    </div>

    <div class="pane-header pane-footer">
      <button class="btn ghost small" @click="addProject">+ Add project</button>
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
