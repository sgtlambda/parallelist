<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import Icon from './Icon.vue'
import TerminalMark from './TerminalMark.vue'
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
  projectId,
  refresh,
  selectBranch,
  selectedBranch,
  staleWorkspacePaths,
  state,
  viewer,
  visibleRows,
} from '../store'
import { relativeTime } from '../format'
import { atRisk, removedBadges, rowBadges } from '../status'
import { enterShell } from '../enterShell'
import { openIn, targetBusy } from '../openIn'
import { terminalActive } from '../terminalState'
import { popupMenu, type MenuItem } from '../menu'
import { animateReorder } from '../reorder'

const emit = defineEmits<{ (e: 'create'): void }>()

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
  { id: 'workspaces' as const, label: 'Workspaces', title: 'Branches with a workspace on disk — a worktree or a clone — or a terminal still open in a removed one', count: counts.value.workspaces },
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
  void popupMenu(actionItems.value, onActionSelect, { x: rect.left, y: rect.bottom + 5 })
}

function onActionSelect(id: string) {
  if (id === 'refresh') void refresh({ fetch: true })
  else if (id === 'cleanup-workspaces') {
    openCleanup(staleWorkspacePaths.value, [], 'Cleanup workspaces')
  } else if (id === 'cleanup-branches') {
    openCleanup([], danglingBranches.value, 'Cleanup branches')
  }
}

function menuItems(row: Row): MenuItem[] {
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
        disabled: targetBusy(t, c.path),
      })),
    })
  })

  // Marks for the branch's running terminals, as if Claude Code had sent
  // them; shown but disabled when it has none, so they can be found.
  const terminals = menuTerminals(row)
  const marks = [
    { mark: 'unread', label: 'Mark unread' },
    { mark: 'blocked', label: 'Mark blocked' },
  ] as const
  const slots = terminals.length ? terminals : [null]
  slots.forEach((path, i) => {
    marks.forEach((m, j) => {
      items.push({
        id: `mark:${m.mark}:${i}`,
        label: terminals.length > 1 ? `${m.label} — ${basename(path!)}` : m.label,
        separator: i === 0 && j === 0,
        disabled: !path,
      })
    })
  })

  // Nothing on disk and nothing on origin: this ref is the only copy left, so
  // the branch list is the only place it can be deleted from.
  if (isDangling(row)) {
    items.push({ id: 'delete-branch', label: 'Delete branch…', separator: true })
  }

  return items
}

/** The row's workspaces with a running shell, including any removed from
 *  under it, as the row's terminal mark counts them. */
function menuTerminals(row: Row): string[] {
  return [...row.workspaces, ...row.unlisted].map((w) => w.path).filter(terminalActive)
}

function openMenu(row: Row) {
  selectBranch(row.branch.name)
  void popupMenu(menuItems(row), (id) => void onMenuSelect(row, id))
}

async function onMenuSelect(row: Row, id: string) {
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
  } else if (id.startsWith('mark:')) {
    const [, mark, index] = id.split(':')
    const path = menuTerminals(row)[Number(index)]
    if (path && (mark === 'unread' || mark === 'blocked')) {
      await api.terminalMark(path, mark).catch(() => {})
    }
  } else if (id.startsWith('open:')) {
    const [, index, targetId] = id.split(':')
    const workspace = row.workspaces[Number(index)]
    if (!workspace || !targetId || !projectId.value) return
    await openIn(projectId.value, targetId, {
      path: workspace.path,
      branch: workspace.branch ?? row.branch.name,
      primary: project.value?.path ?? '',
      prUrl: row.pr?.url ?? '',
    })
  }
}

const listEl = ref<HTMLElement | null>(null)

/** A selection made from elsewhere — the command palette — can be scrolled
 *  out of sight, or not listed yet while its project loads; then it is
 *  brought into view once its row arrives. */
let scrollPending = false

function scrollToSelected() {
  const el = listEl.value?.querySelector('.branch.selected')
  if (!el) return
  el.scrollIntoView({ block: 'nearest' })
  scrollPending = false
}

watch(
  selectedBranch,
  (branch) => {
    scrollPending = !!branch
    void nextTick(scrollToSelected)
  },
)
watch(visibleRows, () => scrollPending && void nextTick(scrollToSelected))

// Rows that change places slide there, as long as it is the same list: same
// project, tab and search.
animateReorder(
  listEl,
  () => visibleRows.value.map((r) => r.branch.name),
  () => [state.overview?.project.id, state.filter, state.search].join('\0'),
)

/** Where this branch lives on disk, each entry labelled by its own kind, and
 *  where it did until it was removed from inside a terminal still open there. */
function folders(row: Row): { kind: WorkspaceKind; name: string; path: string; removed: boolean }[] {
  const entry = (c: Workspace, removed: boolean) => ({
    kind: c.kind,
    name: basename(c.path),
    path: c.path,
    removed,
  })
  return [
    ...row.workspaces.map((c) => entry(c, false)),
    ...row.removed.map((c) => entry(c, true)),
  ]
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

    <div ref="listEl" class="pane-body">
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
        :data-reorder="row.branch.name"
        class="branch"
        :class="{
          selected: row.branch.name === selectedBranch,
          removed: row.removed.length && !row.workspaces.length,
        }"
        @click="selectBranch(row.branch.name)"
        @dblclick="enterShell(row)"
        @contextmenu.prevent="openMenu(row)"
      >
        <div class="row">
          <span class="branch-name truncate">{{ row.branch.name }}</span>
          <span v-if="folders(row).length" class="folder">
            <span
              v-for="(entry, i) in folders(row)"
              :key="entry.path"
              class="folder-entry"
              :class="{ removed: entry.removed }"
              :title="`${WORKSPACE_LABELS[entry.kind]}\n${entry.path}`"
            >
              <span v-if="i > 0" class="folder-sep">·</span>
              <Icon :name="entry.kind" :size="12" />
              <span class="truncate">{{ entry.name }}</span>
            </span>
          </span>
          <span class="spacer" />
          <span class="age faint" style="font-size: 11px">{{ relativeTime(row.branch.committedAt) }}</span>
          <TerminalMark :paths="[...row.workspaces, ...row.unlisted].map((w) => w.path)" />
        </div>

        <div class="branch-meta">
          <span v-if="row.pr" class="badge" :class="prClass(row)">
            <span v-if="checkDot(row)" class="dot" :class="checkDot(row)!" />
            #{{ row.pr.number }}
            {{ row.pr.isDraft && row.pr.state === 'open' ? 'draft' : row.pr.state }}
          </span>
          <span
            v-for="badge in [...removedBadges(row), ...rowBadges(row.workspaces)]"
            :key="badge.key"
            class="badge"
            :class="{ warn: badge.warn }"
            :title="badge.title"
          >
            {{ badge.text }}
          </span>
          <!-- Stale says the workspace can go; the badges above say it can't
               yet. Those are the ones to act on, so they stand alone. -->
          <span
            v-if="row.stale && !row.workspaces.some(atRisk)"
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

    <!-- The whole strip is the button, on the same line as the sidebar's
         "Add project". -->
    <button v-if="projectId" class="add-strip" @click="emit('create')">
      + Add branch
      <span class="shortcut">⌘N</span>
    </button>
  </section>
</template>
