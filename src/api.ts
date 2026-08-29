import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type CheckoutKind = 'primary' | 'worktree' | 'clone' | 'orphan'
export type CheckState = 'pending' | 'passing' | 'failing' | 'none'

export interface SetupStep {
  id: string
  label: string
  defaultEnabled: boolean
  kind: 'copyFiles' | 'symlink' | 'command'
  files?: string[]
  paths?: string[]
  script?: string
}

export interface RepoConfig {
  id: string
  name: string
  path: string
  baseBranch: string | null
  pathTemplate: string
  setupSteps: SetupStep[]
}

export interface OpenTarget {
  id: string
  label: string
  icon: string
  command: string
  primary: boolean
  requiresPr: boolean
}

export interface AppConfig {
  repos: RepoConfig[]
  openTargets: OpenTarget[]
  fetchIntervalSecs: number
  fetchOnFocus: boolean
  prRefreshSecs: number
  activeDays: number
  selectedRepoId: string | null
  sidebarWidth: number
  detailWidth: number
}

export interface BranchInfo {
  name: string
  hasRemote: boolean
  hasLocal: boolean
  sha: string
  committedAt: number
  author: string
  subject: string
  merged: boolean
}

export interface Checkout {
  path: string
  kind: CheckoutKind
  branch: string | null
  /** Which clone a worktree hangs off. Null for a clone's own root. */
  owner: string | null
  /** Modified or staged tracked files. */
  modified: number
  /** Untracked entries — noise like `.DS_Store`, not work at risk. */
  untracked: number
  unpushed: number
  branchGone: boolean
}

export interface PullRequest {
  number: number
  title: string
  branch: string
  state: string
  isDraft: boolean
  url: string
  author: string
  reviewDecision: string
  checks: CheckState
  assignees: string[]
}

export interface Overview {
  repo: RepoConfig
  baseBranch: string
  slug: string | null
  branches: BranchInfo[]
  checkouts: Checkout[]
  pullRequests: Record<string, PullRequest>
  viewer: string | null
  warnings: string[]
}

export interface AheadBehind {
  ahead: number
  behind: number
}

export interface CreatePlan {
  suggestedPath: string
  conflict: { branch: string; path: string } | null
}

export interface CreateRequest {
  repoId: string
  branch: string
  path: string
  mode: 'worktree' | 'clone'
  newBranch: boolean
  from: string | null
  steps: string[]
}

export interface RemovalImpact {
  path: string
  kind: CheckoutKind
  branch: string | null
  modified: number
  untracked: number
  unpushed: number
  stashes: number
  sizeBytes: number
}

export interface RemovalResult {
  path: string
  error: string | null
}

export interface OpenContext {
  path: string
  branch: string
  repo: string
  prUrl: string
}

export const api = {
  getConfig: () => invoke<AppConfig>('get_config'),
  saveConfig: (config: AppConfig) => invoke<void>('save_config', { config }),
  addRepo: (path: string) => invoke<RepoConfig>('add_repo', { path }),
  removeRepo: (id: string) => invoke<void>('remove_repo', { id }),
  selectRepo: (id: string | null) => invoke<void>('select_repo', { id }),
  fetchRepo: (id: string) => invoke<void>('fetch_repo', { id }),
  overview: (id: string) => invoke<Overview>('overview', { id }),
  aheadBehind: (id: string, base: string, branches: string[]) =>
    invoke<Record<string, AheadBehind>>('ahead_behind', { id, base, branches }),
  planCheckout: (id: string, branch: string, pr: number | null) =>
    invoke<CreatePlan>('plan_checkout', { id, branch, pr }),
  createCheckout: (jobId: string, request: CreateRequest) =>
    invoke<string>('create_checkout', { jobId, request }),
  removalImpacts: (id: string, paths: string[]) =>
    invoke<RemovalImpact[]>('removal_impacts', { id, paths }),
  removeCheckouts: (
    id: string,
    items: { path: string; kind: CheckoutKind; deleteBranch: string | null }[],
  ) => invoke<RemovalResult[]>('remove_checkouts', { id, items }),
  openIn: (targetId: string, ctx: OpenContext) => invoke<void>('open_in', { targetId, ctx }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
}

export interface ProgressEvent {
  repoId: string
  phase: string
  detail: string
}

/** Refresh progress, so a multi-second load can say what it is waiting on. */
export function onProgress(handler: (event: ProgressEvent) => void) {
  return listen<ProgressEvent>('progress', (event) => handler(event.payload))
}

export interface LogEvent {
  jobId: string
  line: string
  error: boolean
}

/** Subscribe to the setup log of one create job. */
export function onSetupLog(jobId: string, handler: (event: LogEvent) => void) {
  return listen<LogEvent>('setup-log', (event) => {
    if (event.payload.jobId === jobId) handler(event.payload)
  })
}
