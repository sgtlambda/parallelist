import { invoke, type Channel } from '@tauri-apps/api/core'
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
  /** `command` runs detached through the login shell; `terminal` types it
   *  into the workspace's built-in terminal. */
  kind: 'command' | 'terminal'
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
  /** Null until first set; `loadConfig` derives it from `detailWidth`. */
  listWidth: number | null
  /** Superseded by `listWidth`, which it seeds. */
  detailWidth: number
  /** Every terminal's, so zooming one zooms them all. */
  terminalFontSize: number
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
  /** Null for an orphan, or when git couldn't be asked — then `statusError`
   *  says why. Null with no `statusError` on anything else means not asked
   *  yet (`api.peek`). Never zeros standing in for "unknown". */
  status: WorkspaceStatus | null
  statusError: string | null
}

/** What a workspace holds. `status.rs` defines each count exactly. */
export interface WorkspaceStatus {
  /** Tracked files with changes, staged or not. */
  modified: number
  /** Untracked entries — noise like `.DS_Store`, not work at risk. */
  untracked: number
  /** Commits origin has in no form: no branch, no rebased copy, no squash. */
  unpushed: number
  /** Commits its upstream on origin has that it doesn't. */
  behind: number
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
  /** Workspaces whose directory was deleted while their terminal is still
   *  open — by the program in it, typically — as they last looked. Listed
   *  until the backend closes the terminal, once nothing in it needs you. */
  removed: Workspace[]
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
  status: WorkspaceStatus | null
  /** Something couldn't be measured: unknown, not safe. */
  error: string | null
  /** Commits that exist nowhere else once this is gone. */
  commits: number
  /** A worktree's branch keeps `commits` unless it is deleted too. */
  commitsNeedBranchDelete: boolean
  /** Only a clone's own: a worktree's stashes live in its clone. */
  stashes: number
  /** Worktrees of a clone, stranded by its removal. */
  dependents: string[]
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
  /** What is on disk, without fetching, GitHub or any `git status`: shown
   *  while the first refresh of a project runs. */
  peek: (id: string) => invoke<Overview>('peek', { id }),
  aheadBehind: (id: string, base: string, branches: string[]) =>
    invoke<Record<string, AheadBehind>>('ahead_behind', { id, base, branches }),
  planWorkspace: (id: string, branch: string, pr: number | null) =>
    invoke<CreatePlan>('plan_workspace', { id, branch, pr }),
  /** Returns the task id; the work carries on in the background. */
  createWorkspace: (request: CreateRequest, prUrl: string | null) =>
    invoke<string>('create_workspace', { request, prUrl }),
  removalImpacts: (id: string, paths: string[]) =>
    invoke<RemovalImpact[]>('removal_impacts', { id, paths }),
  workspaceSizes: (paths: string[]) =>
    invoke<Record<string, number>>('workspace_sizes', { paths }),
  removeWorkspaces: (
    id: string,
    items: { path: string; kind: WorkspaceKind; deleteBranch: string | null }[],
  ) => invoke<string[]>('remove_workspaces', { id, items }),
  branchImpacts: (id: string, branches: string[]) =>
    invoke<BranchImpact[]>('branch_impacts', { id, branches }),
  deleteBranches: (id: string, branches: string[]) =>
    invoke<string[]>('delete_branches', { id, branches }),
  /** Resolves to the command to type into the built-in terminal for a
   *  `terminal` target, which the backend leaves to the frontend. */
  openIn: (targetId: string, ctx: OpenContext) =>
    invoke<string | null>('open_in', { targetId, ctx }),
  copyText: (text: string) => invoke<void>('copy_text', { text }),
  listTasks: () => invoke<Task[]>('list_tasks'),
  getTask: (id: string) => invoke<Task | null>('get_task', { id }),
  setTaskAttention: (id: string, attention: boolean) =>
    invoke<void>('set_task_attention', { id, attention }),
  /** Attach to the workspace's terminal, starting a shell if it has none.
   *  Resolves true for a new shell; an existing one replays its recent
   *  output into the channel first. */
  terminalOpen: (
    projectId: string,
    path: string,
    cols: number,
    rows: number,
    channel: Channel<TerminalMessage>,
  ) => invoke<boolean>('terminal_open', { projectId, path, cols, rows, channel }),
  terminalWrite: (path: string, data: string) => invoke<void>('terminal_write', { path, data }),
  terminalResize: (path: string, cols: number, rows: number) =>
    invoke<void>('terminal_resize', { path, cols, rows }),
  terminalClose: (path: string) => invoke<void>('terminal_close', { path }),
  terminalList: () => invoke<TerminalInfo[]>('terminal_list'),
  /** The terminal on screen in the focused window, or null. */
  terminalViewing: (path: string | null, focused: boolean) =>
    invoke<void>('terminal_viewing', { path, focused }),
  /** Mark a terminal by hand, as if its program had reported it. */
  terminalMark: (path: string, mark: 'unseen' | 'blocked') =>
    invoke<void>('terminal_mark', { path, mark }),
}

/** Output as raw bytes, then the exit code once the shell is gone. */
export type TerminalMessage = ArrayBuffer | { exit: number }

export interface TerminalInfo {
  path: string
  projectId: string
  /** What the program in it last reported over OSC 7501 — Claude Code's
   *  `idle`, `working`, `done`, `blocked` — or null. */
  status: string | null
  /** A turn ended while it wasn't being viewed, and it hasn't been since. */
  ready: boolean
}

/** A shell exited, whether or not anything was attached to it. */
export function onTerminalExit(handler: (path: string) => void) {
  return listen<string>('terminal-exit', (e) => handler(e.payload))
}

/** The program in a terminal reported a new status, or stopped having one. */
export function onTerminalStatus(handler: (info: TerminalInfo) => void) {
  return listen<TerminalInfo>('terminal-status', (e) => handler(e.payload))
}

export type TaskKind = 'refresh' | 'create' | 'remove' | 'deleteBranch'
export type TaskStatus = 'queued' | 'running' | 'done' | 'failed'

export interface TaskLine {
  /** Position in the task's log, never reused; a rewritten line keeps it. */
  n: number
  text: string
  error: boolean
}

export interface TaskHeader {
  id: string
  projectId: string
  kind: TaskKind
  title: string
  status: TaskStatus
  /** Failed, or a finished create whose card is not on screen. */
  attention: boolean
  /** Epoch milliseconds. */
  createdAt: number
  startedAt: number | null
  finishedAt: number | null
  branch: string | null
  path: string | null
  mode: 'worktree' | 'clone' | null
  prUrl: string | null
  /** Registry-wide revision; a lower one is an older view of the task. */
  rev: number
}

export interface Task extends TaskHeader {
  log: TaskLine[]
}

/** The backend's task registry, as it changes. */
export function onTaskEvents(handlers: {
  header: (header: TaskHeader) => void
  line: (taskId: string, line: TaskLine) => void
  removed: (taskId: string) => void
}) {
  return Promise.all([
    listen<TaskHeader>('task', (e) => handlers.header(e.payload)),
    listen<{ taskId: string; line: TaskLine }>('task-line', (e) =>
      handlers.line(e.payload.taskId, e.payload.line),
    ),
    listen<string>('task-removed', (e) => handlers.removed(e.payload)),
  ]).then((unlisteners) => () => unlisteners.forEach((u) => u()))
}
