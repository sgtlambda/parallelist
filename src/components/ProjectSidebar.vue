<script setup lang="ts">
import { api } from '../api'
import { basename } from '../format'
import { ref } from 'vue'
import TerminalMark from './TerminalMark.vue'
import { popupMenu, type MenuItem } from '../menu'
import {
  addProject,
  dropCached,
  loadConfig,
  refresh,
  saveConfig,
  selectProject,
  settingsOpen,
  state,
} from '../store'

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

async function forget(id: string) {
  await api.removeProject(id)
  dropCached(id)
  await loadConfig()
  await refresh({ fetch: false })
}

const menuItems: MenuItem[] = [
  { id: 'settings', label: 'Project settings…' },
  { id: 'remove', label: 'Remove project', separator: true },
]

function openMenu(id: string) {
  void popupMenu(menuItems, (action) => void onMenuSelect(id, action))
}

async function onMenuSelect(id: string, action: string) {
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
       footer buttons. -->
  <aside class="pane sidebar" data-tauri-drag-region="deep">
    <div class="pane-body">
      <div class="section-title">Projects</div>
      <!-- A div, not a <button>, for the reason the queue's cards are: WebKit
           sizes a button's children to fit their text, so a name with a mark
           beside it pushed the mark out past the row's background. -->
      <div
        v-for="(project, index) in state.config?.projects ?? []"
        :key="project.id"
        class="project-item"
        role="button"
        tabindex="0"
        :class="{
          selected: project.id === state.config?.selectedProjectId,
          dragging: dragging === index,
          'drop-before': dropAt === index,
          'drop-after': dropAt === index + 1 && index === (state.config?.projects.length ?? 0) - 1,
        }"
        @click="onSelect(project.id)"
        @keydown.enter.prevent="onSelect(project.id)"
        @keydown.space.prevent="onSelect(project.id)"
        @contextmenu.prevent="openMenu(project.id)"
        @pointerdown="onPointerDown(index, $event)"
      >
        <span class="row">
          <span class="project-name truncate">{{ project.name }}</span>
          <span class="spacer" />
          <TerminalMark :project-id="project.id" />
        </span>
        <span class="project-sub truncate">{{ basename(project.path) }}</span>
      </div>

    </div>

    <button class="add-strip" @click="addProject">+ Add project</button>
  </aside>
</template>
