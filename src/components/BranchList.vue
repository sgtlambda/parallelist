<script setup lang="ts">
import { computed, ref } from 'vue'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import Icon from './Icon.vue'
import { api, type Workspace, type WorkspaceKind } from '../api'
import { openUrl } from '@tauri-apps/plugin-opener'
import { WORKSPACE_LABELS, basename } from '../format'
import type { Row } from '../store'
import {
  copyToClipboard,
  counts,
  danglingBranches,
  isDangling,
  openCleanup,
  openTargets,
  project,
  refresh,
  staleWorkspacePaths,
  state,
  viewer,
  visibleRows,
} from '../store'
import { relativeTime } from '../format'

const tabs = computed(() => [
  { id: 'active' as const, label: 'Active', title: 'Open PR, has a workspace, or recently committed to', count: counts.value.active },
  { id: 'prs' as const, label: 'Open PRs', title: 'Every open pull request', count: counts.value.prs },
  // Hidden when gh couldn't tell us who we are — an always-empty tab is worse
  // than no tab.
  ...(viewer.value
    ? [
        {
          id: 'assigned' as const,
          label: 'Assigned',
          title: `Open pull requests assigned to ${viewer.value}`,
          count: counts.value.assigned,
        },
      ]
    : []),
  { id: 'workspaces' as const, label: 'Workspaces', title: 'Branches with a workspace on disk — a worktree or a clone', count: counts.value.workspaces },
  { id: 'all' as const, label: 'All', title: 'Every branch on origin', count: counts.value.all },
])

function prClass(row: Row): string {
  if (!row.pr) return ''
  if (row.pr.isDraft && row.pr.state === 'open') return 'draft'
  return row.pr.state
}

function checkDot(row: Row): string | null {
  if (!row.pr || row.pr.state !== 'open') return null
  switch (row.pr.checks) {
    case 'passing':
      return 'green'
    case 'failing':
      return 'red'
    case 'pending':
      return 'amber'
    default:
      return null
  }
}

/* Fixed shapes rather than random ones, so the placeholder can't jitter as
   Vue re-renders during the load. */
const SKELETON = [
  { name: '52%', pills: ['74px', '26px', '58px'] },
  { name: '38%', pills: ['74px', '26px'] },
  { name: '64%', pills: ['74px', '40px', '26px'] },
  { name: '44%', pills: ['26px', '58px'] },
  { name: '58%', pills: ['74px', '26px'] },
  { name: '34%', pills: ['26px'] },
  { name: '48%', pills: ['74px', '40px'] },
  { name: '40%', pills: ['26px', '58px'] },
]

const actions = ref({ open: false, x: 0, y: 0 })

const actionItems = computed<MenuItem[]>(() => [
  { id: 'refresh', label: 'Refresh now' },
  // Two sweeps, because they cost different things: one frees directories,
  // the other drops refs. Each carries the number it would list.
  {
    id: 'cleanup-workspaces',
    label: countedLabel('Cleanup workspaces', counts.value.staleWorkspaces),
    separator: true,
  },
  {
    id: 'cleanup-branches',
    label: countedLabel('Cleanup branches', counts.value.staleBranches),
  },
])

function countedLabel(label: string, count: number): string {
  return count ? `${label} (${count})…` : `${label}…`
}

function openActions(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  actions.value = { open: true, x: rect.right, y: rect.bottom + 5 }
}

function onActionSelect(id: string) {
  if (id === 'refresh') void refresh({ fetch: true })
  else if (id === 'cleanup-workspaces') {
    openCleanup(staleWorkspacePaths.value, [], 'Cleanup workspaces')
  } else if (id === 'cleanup-branches') {
    openCleanup([], danglingBranches.value, 'Cleanup branches')
  }
}

const menu = ref<{ open: boolean; x: number; y: number; row: Row | null }>({
  open: false,
  x: 0,
  y: 0,
  row: null,
})

const menuItems = computed<MenuItem[]>(() => {
  const row = menu.value.row
  if (!row) return []
  const items: MenuItem[] = [{ id: 'branch', label: 'Copy branch name' }]

  // One entry per workspace, named when there is more than one to tell apart.
  row.workspaces.forEach((c: Workspace, i: number) => {
    items.push({
      id: `path:${i}`,
      label:
        row.workspaces.length > 1
          ? `Copy path — ${basename(c.path)}`
          : 'Copy workspace path',
      separator: i === 0,
    })
  })

  if (row.pr) {
    items.push({ id: 'pr-url', label: 'Copy pull request URL', separator: true })
    items.push({ id: 'pr-open', label: `Open pull request #${row.pr.number}` })
  }

  // One flyout per workspace, so a branch with two of them can send each
  // somewhere different.
  const targets = openTargets.value.filter((t) => !t.requiresPr || row.pr)
  row.workspaces.forEach((c: Workspace, i: number) => {
    items.push({
      id: `open:${i}`,
      label:
        row.workspaces.length > 1 ? `Open ${basename(c.path)} in…` : 'Open in…',
      separator: i === 0,
      children: targets.map((t) => ({
        id: `open:${i}:${t.id}`,
        label: `${t.icon}  ${t.label}`,
      })),
    })
  })

  // Nothing on disk and nothing on origin: this ref is the only copy left, so
  // the branch list is the only place it can be deleted from.
  if (isDangling(row)) {
    items.push({ id: 'delete-branch', label: 'Delete branch…', separator: true, danger: true })
  }

  return items
})

function openMenu(event: MouseEvent, row: Row) {
  state.selectedBranch = row.branch.name
  menu.value = { open: true, x: event.clientX, y: event.clientY, row }
}

async function onMenuSelect(id: string) {
  const row = menu.value.row
  if (!row) return
  if (id === 'branch') {
    await copyToClipboard(row.branch.name, 'branch name')
  } else if (id.startsWith('path:')) {
    const workspace = row.workspaces[Number(id.slice(5))]
    if (workspace) await copyToClipboard(workspace.path, 'path')
  } else if (id === 'pr-url' && row.pr) {
    await copyToClipboard(row.pr.url, 'pull request URL')
  } else if (id === 'pr-open' && row.pr) {
    await openUrl(row.pr.url)
  } else if (id === 'delete-branch') {
    // Into Cleanup rather than a bespoke confirm: it already knows how to show
    // what a deletion costs before it happens.
    openCleanup([], [row.branch.name], 'Delete branch')
  } else if (id.startsWith('open:')) {
    const [, index, targetId] = id.split(':')
    const workspace = row.workspaces[Number(index)]
    if (!workspace || !targetId) return
    await api.openIn(targetId, {
      path: workspace.path,
      branch: workspace.branch ?? row.branch.name,
      primary: project.value?.path ?? '',
      prUrl: row.pr?.url ?? '',
    })
  }
}

/** Where this branch lives on disk, each entry labelled by its own kind. */
function folders(row: Row): { kind: WorkspaceKind; name: string; path: string }[] {
  return row.workspaces.map((c) => ({
    kind: c.kind,
    name: basename(c.path),
    path: c.path,
  }))
}

/** Only real work counts here: untracked strays are not "uncommitted". */
function dirty(row: Row): boolean {
  return row.workspaces.some((c) => c.modified > 0 || c.unpushed > 0)
}
</script>

<template>
  <section class="pane">
    <div class="pane-header search">
      <input
        v-model="state.search"
        type="search"
        placeholder="Search branches, PRs, authors…"
        style="flex: 1; min-width: 0"
      />
      <button
        v-if="state.search"
        class="btn ghost small icon clear"
        title="Clear search"
        @click="state.search = ''"
      >
        <Icon name="close" />
      </button>
      <button class="btn ghost small icon" title="Actions" @click="openActions">
        <Icon name="more" />
      </button>
    </div>

    <div class="tabs">
      <button
        v-for="tab in tabs"
        :key="tab.id"
        class="tab"
        :title="tab.title"
        :class="{ active: state.filter === tab.id && !state.search }"
        @click="state.filter = tab.id"
      >
        {{ tab.label }}<span class="count">{{ tab.count }}</span>
      </button>
    </div>

    <div class="pane-body">
      <div v-if="state.error" class="banner error">{{ state.error }}</div>
      <div v-for="warning in state.overview?.warnings ?? []" :key="warning" class="banner">
        {{ warning }}
      </div>

      <template v-if="state.loading">
        <div
          v-for="(row, i) in SKELETON"
          :key="i"
          class="skeleton-row"
          :style="{ '--i': i }"
        >
          <div class="shimmer bar" :style="{ width: row.name }" />
          <div class="skeleton-meta">
            <div
              v-for="(width, j) in row.pills"
              :key="j"
              class="shimmer pill"
              :style="{ width, '--i': i + j * 0.5 }"
            />
          </div>
        </div>
      </template>
      <p v-else-if="!state.overview" class="empty">Select a project.</p>
      <p v-else-if="!visibleRows.length" class="empty">
        Nothing here.<br />
        <span class="faint">Try another tab, or search across all branches.</span>
      </p>

      <button
        v-for="row in visibleRows"
        :key="row.branch.name"
        class="branch"
        :class="{ selected: row.branch.name === state.selectedBranch }"
        @click="state.selectedBranch = row.branch.name"
        @contextmenu.prevent="openMenu($event, row)"
      >
        <div class="row">
          <span class="branch-name truncate">{{ row.branch.name }}</span>
          <span v-if="row.workspaces.length" class="folder">
            <span
              v-for="(entry, i) in folders(row)"
              :key="entry.path"
              class="folder-entry"
              :title="`${WORKSPACE_LABELS[entry.kind]}\n${entry.path}`"
            >
              <span v-if="i > 0" class="folder-sep">·</span>
              <Icon :name="entry.kind" :size="12" />
              <span class="truncate">{{ entry.name }}</span>
            </span>
          </span>
          <span class="spacer" />
          <span class="faint" style="font-size: 11px">{{ relativeTime(row.branch.committedAt) }}</span>
        </div>

        <div class="branch-meta">
          <span v-if="row.pr" class="badge" :class="prClass(row)">
            <span v-if="checkDot(row)" class="dot" :class="checkDot(row)!" />
            #{{ row.pr.number }}
            {{ row.pr.isDraft && row.pr.state === 'open' ? 'draft' : row.pr.state }}
          </span>
          <span v-if="dirty(row)" class="badge warn">uncommitted</span>
          <!-- Stale says the workspace can go; uncommitted says it can't yet.
               The second is the one to act on, so it stands alone. -->
          <span
            v-if="row.stale && !dirty(row)"
            class="badge warn"
            title="Merged or deleted on origin — the workspace can be removed"
          >
            stale
          </span>
          <span v-if="row.aheadBehind" class="counts muted">
            ↑{{ row.aheadBehind.ahead }} ↓{{ row.aheadBehind.behind }}
          </span>
          <span class="muted truncate">{{ row.branch.subject }}</span>
        </div>
      </button>
    </div>

    <ContextMenu
      :open="actions.open"
      :x="actions.x"
      :y="actions.y"
      :items="actionItems"
      align="right"
      @close="actions.open = false"
      @select="onActionSelect"
    />

    <ContextMenu
      :open="menu.open"
      :x="menu.x"
      :y="menu.y"
      :items="menuItems"
      @close="menu.open = false"
      @select="onMenuSelect"
    />
  </section>
</template>
