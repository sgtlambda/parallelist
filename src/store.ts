import { computed, reactive, ref, watch } from 'vue'
import {
  api,
  onProgress,
  type AheadBehind,
  type AppConfig,
  type BranchInfo,
  type Checkout,
  type Overview,
  type PullRequest,
} from './api'

export type Filter = 'active' | 'prs' | 'assigned' | 'checkedOut' | 'all'

/** A branch with everything we know about it hung off the side. */
export interface Row {
  branch: BranchInfo
  pr: PullRequest | null
  checkouts: Checkout[]
  aheadBehind: AheadBehind | null
  /** Merged, or gone from origin, with a checkout still sitting on disk. */
  stale: boolean
}

const DAY = 86_400

export const state = reactive({
  config: null as AppConfig | null,
  overview: null as Overview | null,
  loading: false,
  fetching: false,
  error: null as string | null,
  /** Last fetch failure. Non-fatal: the list still renders from disk. */
  fetchError: null as string | null,
  selectedBranch: null as string | null,
  filter: 'active' as Filter,
  search: '',
  aheadBehind: {} as Record<string, AheadBehind>,
  progress: null as { phase: string; detail: string } | null,
  /** When the current phase started, so a slow one can say how slow. */
  progressSince: 0,
  now: Date.now(),
})

const PHASE_LABELS: Record<string, string> = {
  fetching: 'Fetching from origin',
  branches: 'Reading branches',
  checkouts: 'Checking working trees',
  pullRequests: 'Loading pull requests',
}

export function describeProgress(): string | null {
  if (!state.progress) return null
  const label = PHASE_LABELS[state.progress.phase] ?? 'Working'
  const parts = [label]
  if (state.progress.detail) parts.push(state.progress.detail)
  // Only once it's slow enough to wonder about, so the line doesn't flicker
  // with a counter on every quick refresh.
  const elapsed = Math.floor((state.now - state.progressSince) / 1000)
  if (state.progressSince && elapsed >= 3) parts.push(`${elapsed}s`)
  return parts.join(' · ')
}

/** Backend refresh phases. Ignores repos other than the one on screen. */
export function listenProgress() {
  return onProgress((event) => {
    if (event.repoId !== repoId.value) return
    if (event.phase === 'done') {
      state.progress = null
      state.progressSince = 0
      return
    }
    if (state.progress?.phase !== event.phase) {
      state.progressSince = Date.now()
      state.now = Date.now()
    }
    state.progress = { phase: event.phase, detail: event.detail }
  })
}

export const repoId = computed(() => state.config?.selectedRepoId ?? null)
export const repo = computed(
  () => state.config?.repos.find((r) => r.id === repoId.value) ?? null,
)

/** The logged-in GitHub user, when `gh` could tell us. */
export const viewer = computed(() => state.overview?.viewer ?? null)

function assignedToMe(row: Row): boolean {
  const me = viewer.value
  return !!me && (row.pr?.assignees.includes(me) ?? false)
}

/** The branch the primary clone currently has checked out. */
const primaryBranch = computed(
  () => state.overview?.checkouts.find((c) => c.kind === 'primary')?.branch ?? null,
)

/** Checkouts that aren't attached to any branch we can show in the list. */
export const looseCheckouts = computed(() =>
  (state.overview?.checkouts ?? []).filter((c) => c.kind === 'orphan' || !c.branch),
)

const rows = computed<Row[]>(() => {
  const ov = state.overview
  if (!ov) return []
  const byBranch = new Map<string, Checkout[]>()
  for (const c of ov.checkouts) {
    if (!c.branch) continue
    const list = byBranch.get(c.branch) ?? []
    list.push(c)
    byBranch.set(c.branch, list)
  }
  return ov.branches.map((branch) => {
    const pr = ov.pullRequests[branch.name] ?? null
    const checkouts = byBranch.get(branch.name) ?? []
    return {
      branch,
      pr,
      checkouts,
      aheadBehind: state.aheadBehind[branch.name] ?? null,
      stale:
        checkouts.some((c) => c.kind !== 'primary') &&
        (branch.merged || !branch.hasRemote || pr?.state === 'merged' || pr?.state === 'closed'),
    }
  })
})

/** Keep the primary clone's branch first. It is the branch everything else is
 *  measured against, so it shouldn't drift around the list as commits land. */
function pinPrimary(list: Row[]): Row[] {
  const name = primaryBranch.value
  if (!name) return list
  const index = list.findIndex((r) => r.branch.name === name)
  if (index <= 0) return list
  const pinned = [...list]
  pinned.unshift(pinned.splice(index, 1)[0])
  return pinned
}

/** Everything about a row that search should be able to match. */
function haystack(row: Row): string {
  return [
    row.branch.name,
    row.branch.subject,
    row.branch.author,
    row.pr ? `#${row.pr.number}` : '',
    row.pr?.title ?? '',
    // Directory names are how a checkout gets referred to in practice ("the
    // app-b one"), and for a clone on a branch that exists nowhere else
    // it's the only name you'd think to type.
    ...row.checkouts.map((c) => c.path),
  ]
    .join(' ')
    .toLowerCase()
}

function isActive(row: Row, activeDays: number): boolean {
  if (row.pr?.state === 'open') return true
  if (row.checkouts.length > 0) return true
  const age = Date.now() / 1000 - row.branch.committedAt
  return age < activeDays * DAY
}

export const counts = computed(() => {
  const days = state.config?.activeDays ?? 30
  const all = rows.value
  return {
    active: all.filter((r) => isActive(r, days)).length,
    prs: all.filter((r) => r.pr?.state === 'open').length,
    assigned: all.filter((r) => r.pr?.state === 'open' && assignedToMe(r)).length,
    checkedOut: all.filter((r) => r.checkouts.length > 0).length,
    all: all.length,
    stale: all.filter((r) => r.stale).length + looseCheckouts.value.length,
  }
})

export const visibleRows = computed<Row[]>(() => {
  const days = state.config?.activeDays ?? 30
  const term = state.search.trim().toLowerCase()
  let list = rows.value

  // Search always looks at every branch, whatever the filter is set to —
  // otherwise finding an old branch means changing tab first.
  if (term) {
    return pinPrimary(list.filter((r) => haystack(r).includes(term)))
  }

  switch (state.filter) {
    case 'prs':
      return pinPrimary(list.filter((r) => r.pr?.state === 'open'))
    case 'assigned':
      return pinPrimary(list.filter((r) => r.pr?.state === 'open' && assignedToMe(r)))
    case 'checkedOut':
      return pinPrimary(list.filter((r) => r.checkouts.length > 0))
    case 'all':
      return pinPrimary(list)
    default:
      return pinPrimary(list.filter((r) => isActive(r, days)))
  }
})

export const selectedRow = computed<Row | null>(
  () => rows.value.find((r) => r.branch.name === state.selectedBranch) ?? null,
)

export async function loadConfig() {
  state.config = await api.getConfig()
}

export async function saveConfig(next: AppConfig) {
  await api.saveConfig(next)
  state.config = next
}

let inFlight = 0

export async function refresh(options: { fetch?: boolean } = {}) {
  const id = repoId.value
  if (!id) {
    state.overview = null
    return
  }
  const ticket = ++inFlight
  state.loading = state.overview === null
  try {
    if (options.fetch) {
      state.fetching = true
      // A failed fetch (offline, VPN down, a dead connection that hit the
      // timeout) shouldn't stop us rendering what we already have on disk —
      // but it must be visible, or the branch list quietly goes stale.
      try {
        await api.fetchRepo(id)
        state.fetchError = null
      } catch (e) {
        state.fetchError = String(e)
      }
      state.fetching = false
    }
    const overview = await api.overview(id)
    if (ticket !== inFlight) return
    state.overview = overview
    state.error = null
    if (state.selectedBranch && !overview.branches.some((b) => b.name === state.selectedBranch)) {
      state.selectedBranch = null
    }
    void loadAheadBehind()
  } catch (e) {
    if (ticket === inFlight) state.error = String(e)
  } finally {
    if (ticket === inFlight) {
      state.loading = false
      state.fetching = false
      state.progress = null
      state.progressSince = 0
    }
  }
}

/** Ahead/behind costs a git process per branch, so only the visible ones. */
async function loadAheadBehind() {
  const id = repoId.value
  const ov = state.overview
  if (!id || !ov) return
  const wanted = visibleRows.value
    .slice(0, 60)
    .map((r) => r.branch.name)
    .filter((name) => !(name in state.aheadBehind))
  if (wanted.length === 0) return
  try {
    const result = await api.aheadBehind(id, ov.baseBranch, wanted)
    state.aheadBehind = { ...state.aheadBehind, ...result }
  } catch {
    /* counts are decoration; failing to get them is not worth a banner */
  }
}

export async function selectRepo(id: string | null) {
  if (state.config) state.config.selectedRepoId = id
  state.selectedBranch = null
  state.aheadBehind = {}
  state.overview = null
  await api.selectRepo(id)
  await refresh({ fetch: true })
}

const timers: number[] = []

/** Background freshness: periodic fetch, a lighter PR-only poll, and a
 *  re-check whenever the window comes back to the front. */
export function startBackgroundSync() {
  stopBackgroundSync()
  const cfg = state.config
  if (!cfg) return

  timers.push(
    window.setInterval(() => void refresh({ fetch: true }), Math.max(30, cfg.fetchIntervalSecs) * 1000),
  )
  timers.push(
    window.setInterval(() => void refresh(), Math.max(30, cfg.prRefreshSecs) * 1000),
  )
  // Drives the elapsed counter on the status line.
  timers.push(
    window.setInterval(() => {
      if (state.progress) state.now = Date.now()
    }, 500),
  )

  if (cfg.fetchOnFocus) {
    window.addEventListener('focus', onFocus)
  }
}

let lastFocusSync = 0
function onFocus() {
  // Coming back to the window in quick succession shouldn't queue up fetches.
  if (Date.now() - lastFocusSync < 15_000) return
  lastFocusSync = Date.now()
  void refresh({ fetch: true })
}

export function stopBackgroundSync() {
  timers.splice(0).forEach((t) => window.clearInterval(t))
  window.removeEventListener('focus', onFocus)
}

watch(
  () => [state.filter, state.search] as const,
  () => void loadAheadBehind(),
)

export const settingsOpen = ref(false)
export const cleanupOpen = ref(false)
