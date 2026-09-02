import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

export type WorkspaceKind = 'primary' | 'worktree' | 'clone' | 'orphan'
export type CheckState = 'pending' | 'passing' | 'failing' | 'none'

export interface SetupStep {
  id: string
  label: string
  defaultEnabled: boolean
  /** Shell command; `{primary}`, `{path}` and `{branch}` are substituted. */
  script: string
}

export interface ProjectConfig {
  id: string
  name: string
  path: string
  baseBranch: string | null
  pathTemplate: string
  setupSteps: SetupStep[]
  openTargets: OpenTarget[]
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
  projects: ProjectConfig[]
  openTargets: OpenTarget[]
  fetchIntervalSecs: number
  fetchOnFocus: boolean
  prRefreshSecs: number
  activeDays: number
  selectedProjectId: string | null
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
  /** Existed on origin at some point, even if deleted there since. */
  wasPushed: boolean
}

export interface Workspace {
  path: string
  kind: WorkspaceKind
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
  project: ProjectConfig
  baseBranch: string
  slug: string | null
  branches: BranchInfo[]
  workspaces: Workspace[]
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
  projectId: string
  branch: string
  path: string
  mode: 'worktree' | 'clone'
  newBranch: boolean
  from: string | null
  steps: string[]
}

export interface RemovalImpact {
  path: string
  kind: WorkspaceKind
  branch: string | null
  modified: number
  untracked: number
  unpushed: number
  stashes: number
}

export interface RemovalResult {
  path: string
  error: string | null
}

export interface Commit {
  sha: string
  subject: string
  author: string
  committedAt: number
}

/** What deleting a branch would cost. */
export interface BranchImpact {
  branch: string
  /** Newest first, capped for display; `commitCount` is the real total. */
  commits: Commit[]
  /** Commits reachable from this branch and from no other ref. */
  commitCount: number
}

export interface BranchDeleteResult {
  branch: string
  error: string | null
}

export interface OpenContext {
  path: string
  branch: string
  primary: string
  prUrl: string
}

export const api = {
  getConfig: () => invoke<AppConfig>('get_config'),
  saveConfig: (config: AppConfig) => invoke<void>('save_config', { config }),
  addProject: (path: string) => invoke<ProjectConfig>('add_project', { path }),
  removeProject: (id: string) => invoke<void>('remove_project', { id }),
  selectProject: (id: string | null) => invoke<void>('select_project', { id }),
  refresh: (id: string, fetch: boolean) => invoke<Overview>('refresh', { id, fetch }),
  aheadBehind: (id: string, base: string, branches: string[]) =>
    invoke<Record<string, AheadBehind>>('ahead_behind', { id, base, branches }),
  planWorkspace: (id: string, branch: string, pr: number | null) =>
    invoke<CreatePlan>('plan_workspace', { id, branch, pr }),
  createWorkspace: (jobId: string, request: CreateRequest) =>
    invoke<string>('create_workspace', { jobId, request }),
  removalImpacts: (id: string, paths: string[]) =>
    invoke<RemovalImpact[]>('removal_impacts', { id, paths }),
  workspaceSizes: (paths: string[]) =>
    invoke<Record<string, number>>('workspace_sizes', { paths }),
  removeWorkspaces: (
    id: string,
    items: { path: string; kind: WorkspaceKind; deleteBranch: string | null }[],
  ) => invoke<RemovalResult[]>('remove_workspaces', { id, items }),
  branchImpacts: (id: string, branches: string[]) =>
    invoke<BranchImpact[]>('branch_impacts', { id, branches }),
  deleteBranches: (id: string, branches: string[]) =>
    invoke<BranchDeleteResult[]>('delete_branches', { id, branches }),
  openIn: (targetId: string, ctx: OpenContext) => invoke<void>('open_in', { targetId, ctx }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
}

export interface ProgressEvent {
  projectId: string
  phase: string
  detail: string
  /** Phases overlap, so this reports a phase starting or finishing. */
  active: boolean
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
