<script setup lang="ts">
import { computed, ref } from 'vue'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import Icon from './Icon.vue'
import type { Checkout, CheckoutKind } from '../api'
import { openUrl } from '@tauri-apps/plugin-opener'
import { CHECKOUT_LABELS, basename } from '../format'
import type { Row } from '../store'
import {
  cleanupOpen,
  copyToClipboard,
  counts,
  describeProgress,
  refresh,
  state,
  viewer,
  visibleRows,
} from '../store'
import { relativeTime } from '../format'

const tabs = computed(() => [
  { id: 'active' as const, label: 'Active', title: 'Open PR, checked out, or recently committed to', count: counts.value.active },
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
  { id: 'checkedOut' as const, label: 'Checked out', title: 'Branches with a worktree or clone on disk', count: counts.value.checkedOut },
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
  {
    id: 'cleanup',
    label: counts.value.stale ? `Cleanup (${counts.value.stale})…` : 'Cleanup…',
    separator: true,
  },
])

function openActions(event: MouseEvent) {
  const rect = (event.currentTarget as HTMLElement).getBoundingClientRect()
  actions.value = { open: true, x: rect.right, y: rect.bottom + 5 }
}

function onActionSelect(id: string) {
  if (id === 'refresh') void refresh({ fetch: true })
  else if (id === 'cleanup') cleanupOpen.value = true
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

  // One entry per checkout, named when there is more than one to tell apart.
  row.checkouts.forEach((c: Checkout, i: number) => {
    items.push({
      id: `path:${i}`,
      label:
        row.checkouts.length > 1
          ? `Copy path — ${basename(c.path)}`
          : 'Copy checkout path',
      separator: i === 0,
    })
  })

  if (row.pr) {
    items.push({ id: 'pr-url', label: 'Copy pull request URL', separator: true })
    items.push({ id: 'pr-open', label: `Open pull request #${row.pr.number}` })
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
    const checkout = row.checkouts[Number(id.slice(5))]
    if (checkout) await copyToClipboard(checkout.path, 'path')
  } else if (id === 'pr-url' && row.pr) {
    await copyToClipboard(row.pr.url, 'pull request URL')
  } else if (id === 'pr-open' && row.pr) {
    await openUrl(row.pr.url)
  }
}

/** Where this branch lives on disk, each entry labelled by its own kind. */
function folders(row: Row): { kind: CheckoutKind; name: string; path: string }[] {
  return row.checkouts.map((c) => ({
    kind: c.kind,
    name: basename(c.path),
    path: c.path,
  }))
}

/** Only real work counts here: untracked strays are not "uncommitted". */
function dirty(row: Row): boolean {
  return row.checkouts.some((c) => c.modified > 0 || c.unpushed > 0)
}
</script>

<template>
  <section class="pane">
    <div class="pane-header" data-tauri-drag-region="deep">
      <input
        v-model="state.search"
        type="search"
        placeholder="Search branches, PRs, authors…"
        style="flex: 1; min-width: 0"
      />
      <button
        v-if="counts.stale"
        class="btn small"
        title="Merged, gone, or orphaned checkouts"
        @click="cleanupOpen = true"
      >
        Cleanup {{ counts.stale }}
      </button>
      <button class="btn ghost small icon" title="Fetch now" @click="refresh({ fetch: true })">
        <span v-if="state.fetching" class="spin" />
        <span v-else>↻</span>
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
      <div v-if="state.fetchError" class="banner">
        Couldn't reach origin — showing what's on disk.
        <span class="faint">{{ state.fetchError }}</span>
      </div>
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
          <span v-if="row.checkouts.length" class="folder">
            <span
              v-for="(entry, i) in folders(row)"
              :key="entry.path"
              class="folder-entry"
              :title="`${CHECKOUT_LABELS[entry.kind]}\n${entry.path}`"
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
          <span
            v-for="entry in kinds(row)"
            :key="entry.kind"
            class="badge kind"
            :title="CHECKOUT_LABELS[entry.kind] + (entry.count > 1 ? ` × ${entry.count}` : '')"
          >
            <Icon :name="entry.kind" />
            <span v-if="entry.count > 1">{{ entry.count }}</span>
          </span>
          <span
            v-for="folder in matchingFolders(row)"
            :key="folder"
            class="badge path"
            title="Matching checkout directory"
          >
            {{ folder }}
          </span>
          <span v-if="dirty(row)" class="badge warn">uncommitted</span>
          <span v-if="row.stale" class="badge warn">stale</span>
          <span v-if="row.aheadBehind" class="counts muted">
            ↑{{ row.aheadBehind.ahead }} ↓{{ row.aheadBehind.behind }}
          </span>
          <span class="muted truncate">{{ row.branch.subject }}</span>
        </div>
      </button>
    </div>

    <!-- Refreshing over data we already have: a quiet line rather than a
         spinner that hides the list. Sits below the scroll area so it can
         never push the rows down. -->
    <div v-if="describeProgress()" class="status-line">
      <span class="spin" />
      <span>{{ describeProgress() }}…</span>
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
