import { open } from '@tauri-apps/plugin-dialog'
import { computed, reactive, ref, watch } from 'vue'
import {
  api,
  type AheadBehind,
  type AppConfig,
  type BranchInfo,
  type Workspace,
  type Overview,
  type PullRequest,
} from './api'
import { hasPendingCreate, normalizePath } from './tasks'
import { byTerminalRank, latestShown, running, shown, terminalRank } from './terminalState'

export type Filter = 'active' | 'prs' | 'assigned' | 'workspaces' | 'all'

/** A branch and the workspaces on it (`branchesOf`). */
export interface Placed {
  branch: BranchInfo
  workspaces: Workspace[]
  /** Running terminals on this branch whose workspace the overview doesn't
   *  list: removed from inside the terminal, typically, or newer than the
   *  overview. Counted nowhere, but listed so the terminal can be reached. */
  unlisted: Workspace[]
}

/** A branch with everything we know about it hung off the side. */
export interface Row extends Placed {
  pr: PullRequest | null
  aheadBehind: AheadBehind | null
  /** Finished work — merged or closed — with a workspace still on disk. */
  stale: boolean
}

/**
 * What is selected: a workspace when the branch has one, by its directory,
 * which stays the same whatever happens to the branch in it. `branch` is the
 * name last seen, for a branch with no workspace, whose name is all it has,
 * and for finding a row while the workspace isn't placed on one. Which branch
 * is selected is otherwise worked out from where the workspace is now
 * (`selectedBranch`), so a rename can't leave the selection behind.
 */
export interface Selection {
  path: string | null
  branch: string
}

const DAY = 86_400

export const state = reactive({
  config: null as AppConfig | null,
  overview: null as Overview | null,
  loading: false,
  error: null as string | null,
  selection: null as Selection | null,
  filter: 'active' as Filter,
  search: '',
  aheadBehind: {} as Record<string, AheadBehind>,
  /** When the refresh behind `overview` was asked for. Anything that finished
   *  after this may not be reflected in it yet. */
  overviewAt: 0,
  now: Date.now(),
  toast: null as string | null,
})

export const projectId = computed(() => state.config?.selectedProjectId ?? null)
export const project = computed(
  () => state.config?.projects.find((p) => p.id === projectId.value) ?? null,
)

/** Global targets plus a project's own; a project id wins on a clash. */
export function openTargetsFor(id: string | null) {
  const global = state.config?.openTargets ?? []
  const own = state.config?.projects.find((p) => p.id === id)?.openTargets ?? []
  const ids = new Set(own.map((t) => t.id))
  return [...own, ...global.filter((t) => !ids.has(t.id))]
}

export const openTargets = computed(() => openTargetsFor(projectId.value))

/** The logged-in GitHub user, when `gh` could tell us. */
export const viewer = computed(() => state.overview?.viewer ?? null)

function assignedToMe(row: Row): boolean {
  const me = viewer.value
  return !!me && (row.pr?.assignees.includes(me) ?? false)
}

/** The branch sitting in the main workspace. (`'primary'` is the wire name
 *  for that kind — see the note on vocabulary in AGENTS.md.) */
const mainWorkspaceBranch = computed(
  () => state.overview?.workspaces.find((c) => c.kind === 'primary')?.branch ?? null,
)

/** Workspaces that aren't attached to any branch we can show in the list. */
export const looseWorkspaces = computed(() =>
  (state.overview?.workspaces ?? []).filter((c) => c.kind === 'orphan' || !c.branch),
)

export function groupByBranch(workspaces: Workspace[]): Map<string, Workspace[]> {
  const byBranch = new Map<string, Workspace[]>()
  for (const c of workspaces) {
    if (!c.branch) continue
    const list = byBranch.get(c.branch) ?? []
    list.push(c)
    byBranch.set(c.branch, list)
  }
  return byBranch
}

/**
 * A project's branches with their workspaces on them: the overview's, and
 * every running terminal of the project. The branch list, the palette and
 * the attention queue all read this, which is what keeps them agreeing:
 * nothing can wait in the queue without a row in the list to find it on.
 *
 * A terminal goes wherever the overview puts its workspace. Where the
 * overview puts it on no branch, it goes on its home branch, the one it was
 * opened on (`TerminalInfo`): a workspace listed without a branch — a
 * detached HEAD, mid-rebase — as itself, and one not listed at all as
 * `unlisted`. A branch that no ref has any more, or not yet, gets a row of
 * its own: neither local nor on origin, which no listed branch otherwise is.
 */
export function branchesOf(id: string, ov: Overview | null): Placed[] {
  const branches = [...(ov?.branches ?? [])]
  const workspaces = [...(ov?.workspaces ?? [])]
  const unlisted: Workspace[] = []
  const listed = new Map(workspaces.map((w, i) => [normalizePath(w.path), i]))
  for (const [path, home] of running) {
    if (home.projectId !== id) continue
    const i = listed.get(path)
    if (i === undefined) {
      unlisted.push({ path, kind: home.kind, branch: home.branch, owner: null, status: null, statusError: null })
    } else if (!workspaces[i].branch) {
      workspaces[i] = { ...workspaces[i], branch: home.branch }
    }
  }
  const named = new Set(branches.map((b) => b.name))
  for (const { branch } of [...workspaces, ...unlisted]) {
    if (!branch || named.has(branch)) continue
    named.add(branch)
    branches.push({
      name: branch,
      hasLocal: false,
      hasRemote: false,
      wasPushed: false,
      merged: false,
      sha: '',
      committedAt: 0,
      author: '',
      subject: '',
    })
  }
  const byBranch = groupByBranch(workspaces)
  const unlistedByBranch = groupByBranch(unlisted)
  return branches.map((branch) => ({
    branch,
    workspaces: byBranch.get(branch.name) ?? [],
    unlisted: unlistedByBranch.get(branch.name) ?? [],
  }))
}

export const rows = computed<Row[]>(() => {
  const ov = state.overview
  if (!ov) return []
  return branchesOf(ov.project.id, ov).map(({ branch, workspaces, unlisted }) => {
    const pr = ov.pullRequests[branch.name] ?? null
    return {
      branch,
      pr,
      workspaces,
      unlisted,
      aheadBehind: state.aheadBehind[branch.name] ?? null,
      // Finished work, not merely absent from origin. A branch that was
      // never pushed is also "not on origin", and treating that as stale
      // offered unpushed commits up for deletion.
      stale:
        workspaces.some((c) => c.kind !== 'primary') &&
        (branch.merged || pr?.state === 'merged' || pr?.state === 'closed'),
    }
  })
})

/**
 * The main workspace's branch first: it is the branch everything else is
 * measured against, so it shouldn't drift around the list as commits land.
 * Then the branches with a running terminal, by `terminalRank` as in the
 * palette, then the rest as they came.
 *
 * Only a change of tier — a terminal starting, stopping, or coming to need
 * the user — re-sorts on its own. Which terminal was active most recently is
 * taken as it stands whenever the list re-sorts anyway (a refresh, another
 * tab, a search), so rows don't move under the pointer with every keystroke
 * typed into one.
 */
function inOrder(list: Row[]): Row[] {
  const main = mainWorkspaceBranch.value
  const ranks = new Map(
    list.map((r) => [r, terminalRank([...r.workspaces, ...r.unlisted].map((w) => w.path))]),
  )
  return [...list].sort(
    (a, b) =>
      Number(b.branch.name === main) - Number(a.branch.name === main) ||
      byTerminalRank(ranks.get(a)!, ranks.get(b)!),
  )
}

/** Everything about a row that search should be able to match. */
function haystack(row: Row): string {
  return [
    row.branch.name,
    row.branch.subject,
    row.branch.author,
    row.pr ? `#${row.pr.number}` : '',
    row.pr?.title ?? '',
    // Directory names are how a workspace gets referred to in practice ("the
    // app-b one"), and for a clone on a branch that exists nowhere else
    // it's the only name you'd think to type.
    ...row.workspaces.map((c) => c.path),
  ]
    .join(' ')
    .toLowerCase()
}

/** Workspace directories that can go: finished work, plus orphaned folders. */
export const staleWorkspacePaths = computed(() => {
  const finished = rows.value
    .filter((r) => r.stale)
    .flatMap((r) => r.workspaces)
    .filter((c) => c.kind !== 'primary')
    .map((c) => c.path)
  const orphans = looseWorkspaces.value.filter((c) => c.kind === 'orphan').map((c) => c.path)
  return [...new Set([...finished, ...orphans])]
})

/** A local branch with nothing on disk and nothing on origin — what a merge
 *  plus a worktree removal leaves behind. No directory points at one, so
 *  removing a workspace can never offer it and they just accumulate. */
export function isDangling(row: Row): boolean {
  return row.branch.hasLocal && !row.branch.hasRemote && row.workspaces.length === 0
}

export const danglingBranches = computed(() =>
  rows.value.filter(isDangling).map((r) => r.branch.name),
)

function isActive(row: Row, activeDays: number): boolean {
  if (row.pr?.state === 'open') return true
  // Anything on disk is active by definition — it's occupying a directory.
  // So is a terminal still open in one that has just been removed.
  if (row.workspaces.length > 0 || row.unlisted.length > 0) return true
  // Finished work with no workspace left is done, however recently it was
  // committed. Without this a branch merged yesterday outranks one that is
  // still open, purely on recency.
  if (row.branch.merged || row.pr?.state === 'merged' || row.pr?.state === 'closed') {
    return false
  }
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
    workspaces: all.filter((r) => r.workspaces.length > 0).length,
    all: all.length,
    // One per Cleanup entry, so each menu item's number is exactly what that
    // sweep would list.
    staleWorkspaces: staleWorkspacePaths.value.length,
    staleBranches: danglingBranches.value.length,
  }
})

export const visibleRows = computed<Row[]>(() => {
  const days = state.config?.activeDays ?? 30
  const term = state.search.trim().toLowerCase()
  let list = rows.value

  // Search always looks at every branch, whatever the filter is set to —
  // otherwise finding an old branch means changing tab first.
  if (term) {
    return inOrder(list.filter((r) => haystack(r).includes(term)))
  }

  switch (state.filter) {
    case 'prs':
      return inOrder(list.filter((r) => r.pr?.state === 'open'))
    case 'assigned':
      return inOrder(list.filter((r) => r.pr?.state === 'open' && assignedToMe(r)))
    case 'workspaces':
      return inOrder(list.filter((r) => r.workspaces.length > 0))
    case 'all':
      return inOrder(list)
    default:
      return inOrder(list.filter((r) => isActive(r, days)))
  }
})

/** The branch each workspace on screen is on, as the list has it. */
const placedOn = computed(() => {
  const byPath = new Map<string, string>()
  for (const row of rows.value) {
    for (const w of [...row.workspaces, ...row.unlisted]) {
      byPath.set(normalizePath(w.path), row.branch.name)
    }
  }
  return byPath
})

export const selectedBranch = computed(() => {
  const s = state.selection
  if (!s) return null
  return (s.path && placedOn.value.get(s.path)) || s.branch
})

export const selectedRow = computed<Row | null>(
  () => rows.value.find((r) => r.branch.name === selectedBranch.value) ?? null,
)

/** The terminal the pane is asked to show, if any. */
export const selectedTerminal = computed(() => {
  const path = state.selection?.path
  return path && shown.has(path) ? path : null
})

/** The workspace to select for a branch: the one whose terminal it showed
 *  last, else the first listed. */
function workspaceOn(branch: string): string | null {
  const row = rows.value.find((r) => r.branch.name === branch)
  const paths = [...(row?.workspaces ?? []), ...(row?.unlisted ?? [])].map((w) => w.path)
  return latestShown(paths) ?? (paths.length ? normalizePath(paths[0]) : null)
}

/** Select a branch of the project on screen, as clicking its row does. */
export function selectBranch(branch: string | null) {
  state.selection = branch ? { path: workspaceOn(branch), branch } : null
}

export async function loadConfig() {
  const config = await api.getConfig()
  // The detail pane used to have the stored width and the branch list the
  // rest. Derived from that, the layout opens as it was left; it is written
  // back with the next save.
  config.listWidth ??= Math.max(
    LIST_MIN,
    Math.round(window.innerWidth - config.sidebarWidth - config.detailWidth),
  )
  state.config = config
}

export async function saveConfig(next: AppConfig) {
  // Cloned to a plain object: the IPC layer structured-clones its payload and
  // chokes on a reactive proxy.
  await api.saveConfig(JSON.parse(JSON.stringify(next)))
  state.config = next
}

export const SIDEBAR_RANGE = { min: 170, max: 380 }
/** Narrowest the branch list gets. Enough for a truncated name and its age;
 *  the folder name and the badges give way first. No ceiling beyond leaving
 *  the detail pane `DETAIL_MIN`. */
export const LIST_MIN = 180
/** The detail pane has no stored width: it takes whatever the window leaves
 *  over, so resizing the window resizes it and the two columns to its left
 *  stay as they were set. A terminal wants all the room it can get. */
export const DETAIL_MIN = 280

export const sidebarWidth = computed(() => state.config?.sidebarWidth ?? 216)
export const listWidth = computed(() => state.config?.listWidth ?? 360)

/** Live update while dragging; nothing is written until the drag ends. */
export function setPaneWidths(sidebar: number, list: number) {
  if (!state.config) return
  state.config.sidebarWidth = sidebar
  state.config.listWidth = list
}

export async function persistPaneWidths() {
  if (!state.config) return
  await saveConfig(JSON.parse(JSON.stringify(state.config)))
}

interface CacheEntry {
  overview: Overview
  overviewAt: number
  aheadBehind: Record<string, AheadBehind>
  selection: Selection | null
  at: number
}

/** Last known state per project. Switching projects shows this immediately —
 *  slightly stale, but present — instead of blanking the list for the several
 *  seconds a full refresh takes. */
const cache = new Map<string, CacheEntry>()

function remember(id: string) {
  // Only a real refresh sets `overviewAt`; a peek is not worth keeping.
  if (!state.overview || state.overviewAt === 0) return
  cache.set(id, {
    overview: state.overview,
    overviewAt: state.overviewAt,
    aheadBehind: state.aheadBehind,
    selection: state.selection,
    at: Date.now(),
  })
}

/** How each project's list was left: its tab and search term. Kept apart from
 *  `cache` because that one only exists once a refresh has landed, and a
 *  search typed while the first one is still loading should survive too. */
const views = new Map<string, { filter: Filter; search: string }>()

export function dropCached(id: string) {
  cache.delete(id)
  views.delete(id)
}

let inFlight = 0

/**
 * Put an overview on screen. The selection needs no moving, since it follows
 * from where its workspace is; what is written down is the name for when
 * that can't say, and the workspace if it has none yet. A terminal's home
 * likewise keeps the branch last seen, for once its workspace is on none.
 */
function land(overview: Overview) {
  state.overview = overview
  for (const w of overview.workspaces) {
    const path = normalizePath(w.path)
    const home = running.get(path)
    if (w.branch && home && home.projectId === overview.project.id && home.branch !== w.branch) {
      running.set(path, { ...home, branch: w.branch })
    }
  }
  const s = state.selection
  if (!s) return
  const placed = s.path && placedOn.value.get(s.path)
  if (placed) s.branch = placed
  // One still asked for stays: a create's, which is listed later.
  else if (!s.path || !shown.has(s.path)) s.path = workspaceOn(s.branch) ?? s.path
}

export async function refresh(options: { fetch?: boolean } = {}) {
  const id = projectId.value
  if (!id) {
    state.overview = null
    return
  }
  const ticket = ++inFlight
  const askedAt = Date.now()
  // The user can switch projects mid-flight; anything that lands afterwards
  // belongs to a project that is no longer on screen.
  const current = () => ticket === inFlight && projectId.value === id
  state.loading = state.overview === null
  if (state.loading) void peek(id, current)
  try {
    // One round-trip: the backend runs the git work and the GitHub queries
    // concurrently. A failed fetch comes back as a warning rather than an
    // error, so the list still renders from local refs.
    const overview = await api.refresh(id, options.fetch === true)
    // Cached even if we've since switched away — that result is exactly what
    // makes switching back instant.
    const previous = cache.get(id)
    cache.set(id, {
      overview,
      overviewAt: askedAt,
      aheadBehind: previous?.aheadBehind ?? {},
      selection: previous?.selection ?? null,
      at: Date.now(),
    })
    if (!current()) return
    land(overview)
    state.overviewAt = askedAt
    state.error = null
    // A branch being created may not exist in any ref yet; its pending card
    // is what the user is looking at, so the selection stays. So does a row
    // no ref has, which is there for a terminal (`branchesOf`).
    if (
      selectedBranch.value &&
      !selectedRow.value &&
      !hasPendingCreate(id, selectedBranch.value)
    ) {
      state.selection = null
    }
    void loadAheadBehind()
  } catch (e) {
    if (current()) state.error = String(e)
  } finally {
    if (current()) state.loading = false
  }
}

/** Nothing on screen for this project yet: show what is on disk while the
 *  full refresh runs. Branches and workspaces only — no PRs, and no status,
 *  which `status.ts` reads as "not asked yet". Discarded if the refresh got
 *  there first, and never cached, so it can't outlive the real thing. */
async function peek(id: string, current: () => boolean) {
  try {
    const overview = await api.peek(id)
    if (!current() || state.overview) return
    land(overview)
    // Zero, which also keeps it out of `cache`: nothing that finished before
    // it is known to be reflected, so create cards hold on until the real
    // refresh lands.
    state.overviewAt = 0
    state.loading = false
  } catch {
    /* the refresh reports whatever went wrong */
  }
}

/** Ahead/behind costs a git process per branch, so only the visible ones. */
async function loadAheadBehind() {
  const id = projectId.value
  const ov = state.overview
  if (!id || !ov) return
  const wanted = visibleRows.value
    .slice(0, 60)
    .map((r) => r.branch.name)
    .filter((name) => !(name in state.aheadBehind))
  if (wanted.length === 0) return
  try {
    const result = await api.aheadBehind(id, ov.baseBranch, wanted)
    if (projectId.value !== id) return
    state.aheadBehind = { ...state.aheadBehind, ...result }
    remember(id)
  } catch {
    /* counts are decoration; failing to get them is not worth a banner */
  }
}

/** Open a project, landing on `selection` if given, else wherever it was
 *  left. */
export async function selectProject(id: string | null, selection?: Selection) {
  const leaving = projectId.value
  if (leaving && leaving !== id) {
    remember(leaving)
    views.set(leaving, { filter: state.filter, search: state.search })
    const view = id ? views.get(id) : undefined
    state.filter = view?.filter ?? 'active'
    state.search = view?.search ?? ''
  }

  if (state.config) state.config.selectedProjectId = id

  // Restore whatever we last saw for this project so the list is populated
  // straight away. The refresh below replaces it; the status line says so.
  const cached = id ? cache.get(id) : undefined
  state.overview = null
  state.overviewAt = cached?.overviewAt ?? 0
  state.aheadBehind = cached?.aheadBehind ?? {}
  state.selection = selection ?? cached?.selection ?? null
  state.error = null
  if (cached) land(cached.overview)

  await api.selectProject(id)
  await refresh({ fetch: true })
}

/** Ask for a project's main workspace and open it. */
export async function addProject() {
  const picked = await open({ directory: true, multiple: false, title: "Choose the project's main workspace" })
  if (typeof picked === 'string') await openFolder(picked)
}

/** Go to the project or workspace a folder is, dropped on the window or
 *  picked with "Add project", adding it as a project if it is neither. A
 *  folder git rejects is reported in `state.error`, as every other failure
 *  is. */
export async function openFolder(path: string) {
  try {
    const landing = await api.openFolder(path)
    if (landing.added) await loadConfig()
    if (landing.branch) goToBranch(landing.projectId, landing.branch)
    else if (landing.projectId !== projectId.value) await selectProject(landing.projectId)
    if (landing.added) {
      const name = state.config?.projects.find((p) => p.id === landing.projectId)?.name
      showToast(`Added ${name ?? 'project'}`)
    }
  } catch (e) {
    state.error = String(e)
  }
}

/** Jump to a branch of any project, as the command palette does. */
export function goToBranch(id: string, branch: string) {
  if (id === projectId.value) selectBranch(branch)
  else void selectProject(id, { path: null, branch })
  revealSelected()
}

/** Jump to a workspace of any project. `branch` is only where it is thought
 *  to be, for until the overview says. */
export function goToWorkspace(id: string, path: string, branch: string) {
  const selection = { path: normalizePath(path), branch }
  if (id === projectId.value) state.selection = selection
  else void selectProject(id, selection)
  revealSelected()
}

/**
 * Make the list show the selected branch: a search that hides it is
 * cleared, then a tab that hides it gives way to All. Waits for the row if
 * it isn't there yet — after switching to a project with nothing cached, it
 * arrives with the peek or the refresh.
 */
function revealSelected() {
  const asked = state.selection
  const reveal = () => {
    // Selected something else meanwhile, or the refresh dropped it.
    if (state.selection !== asked) return true
    if (!selectedRow.value) return false
    const listed = () => visibleRows.value.some((r) => r === selectedRow.value)
    if (!listed()) state.search = ''
    if (!listed()) state.filter = 'all'
    return true
  }
  if (reveal()) return
  const stop = watch(
    () => [selectedRow.value, state.selection] as const,
    () => reveal() && stop(),
  )
}

/** The overview last seen for a project, and when: the one on screen, else
 *  the cached one. Nothing is asked of git. */
export function knownOverview(id: string): { overview: Overview; at: number } | null {
  if (id === projectId.value && state.overview) {
    return { overview: state.overview, at: Date.now() }
  }
  const cached = cache.get(id)
  return cached ? { overview: cached.overview, at: cached.at } : null
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
  // Drives elapsed counters, and how long a finished task lingers in the box.
  timers.push(window.setInterval(() => (state.now = Date.now()), 500))

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

let toastTimer = 0

/** Brief confirmation for actions with no visible result, like copying. */
export function showToast(message: string) {
  state.toast = message
  window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => {
    state.toast = null
  }, 1800)
}

export async function copyToClipboard(text: string, what: string) {
  try {
    await api.copyText(text)
    showToast(`Copied ${what}`)
  } catch (e) {
    state.error = String(e)
  }
}

export const settingsOpen = ref(false)
export const cleanupOpen = ref(false)

/** What the open Cleanup dialog is working on. Snapshots, not live lists: the
 *  refresh that follows a removal must not rewrite the dialog underneath it. */
export const cleanupPaths = ref<string[]>([])
export const cleanupBranches = ref<string[]>([])
export const cleanupTitle = ref('Cleanup')

/** The only way in, so a previous run's targets can't leak into the next one:
 *  every field is set on every call, even when that means clearing it. */
export function openCleanup(paths: string[], branches: string[], title: string) {
  cleanupPaths.value = paths
  cleanupBranches.value = branches
  cleanupTitle.value = title
  cleanupOpen.value = true
}
