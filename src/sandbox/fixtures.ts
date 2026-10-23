import type {
  AppConfig,
  BranchInfo,
  OpenTarget,
  Overview,
  ProjectConfig,
  PullRequest,
  Task,
  TaskLine,
  Workspace,
  WorkspaceStatus,
} from '../api'
import { STEP_MARK } from '../tasks'

/**
 * Made-up data in the shapes the backend returns. Small builders with the
 * boring fields filled in, so a scenario says only what it is about.
 */

const HOME = '/Users/sandbox/Code'
const DAY = 86_400
const now = () => Math.floor(Date.now() / 1000)

export const targets: OpenTarget[] = [
  {
    id: 'claude',
    label: 'Claude',
    icon: '✦',
    command: 'claude',
    kind: 'terminal',
    primary: true,
    requiresPr: false,
    runOnCreate: true,
  },
  {
    id: 'code',
    label: 'VS Code',
    icon: '⌨',
    command: 'code {path}',
    kind: 'command',
    primary: true,
    requiresPr: false,
    runOnCreate: false,
  },
  {
    id: 'pr',
    label: 'Pull request',
    icon: '↗',
    command: 'open {prUrl}',
    kind: 'command',
    primary: false,
    requiresPr: true,
    runOnCreate: false,
  },
]

export const project = (name: string, overrides: Partial<ProjectConfig> = {}): ProjectConfig => ({
  id: name,
  name,
  path: `${HOME}/${name}`,
  baseBranch: 'main',
  pathTemplate: '{project}-{branch}',
  setupSteps: [],
  openTargets: [],
  ...overrides,
})

export const config = (overrides: Partial<AppConfig> = {}): AppConfig => ({
  projects: [project('app'), project('parallelist'), project('docs')],
  openTargets: targets,
  fetchIntervalSecs: 300,
  fetchOnFocus: true,
  prRefreshSecs: 120,
  activeDays: 30,
  selectedProjectId: 'app',
  sidebarWidth: 216,
  listWidth: 360,
  detailWidth: 640,
  terminalFontSize: 13,
  ...overrides,
})

export const branch = (name: string, overrides: Partial<BranchInfo> = {}): BranchInfo => ({
  name,
  hasRemote: true,
  hasLocal: false,
  sha: Math.random().toString(16).slice(2, 9),
  committedAt: now() - 2 * DAY,
  author: 'Sam Rivera',
  subject: `Work on ${name}`,
  merged: false,
  wasPushed: true,
  ...overrides,
})

export const status = (overrides: Partial<WorkspaceStatus> = {}): WorkspaceStatus => ({
  modified: 0,
  untracked: 0,
  unpushed: 0,
  behind: 0,
  ...overrides,
})

export const workspace = (
  dir: string,
  kind: Workspace['kind'],
  branchName: string | null,
  overrides: Partial<Workspace> = {},
): Workspace => ({
  path: `${HOME}/${dir}`,
  kind,
  branch: branchName,
  owner: kind === 'worktree' ? `${HOME}/app` : null,
  status: status(),
  statusError: null,
  ...overrides,
})

export const pr = (number: number, branchName: string, overrides: Partial<PullRequest> = {}): PullRequest => ({
  number,
  title: `Work on ${branchName}`,
  branch: branchName,
  state: 'open',
  isDraft: false,
  url: `https://github.com/example/app/pull/${number}`,
  author: 'sam',
  reviewDecision: '',
  checks: 'passing',
  assignees: [],
  ...overrides,
})

/** One project's overview with something of everything: a main workspace, a
 *  worktree with work in it, a clone, finished work still on disk, a
 *  workspace git could not read, and branches with nothing on disk. */
export function overview(overrides: Partial<Overview> = {}): Overview {
  const workspaces = [
    workspace('app', 'primary', 'main'),
    workspace('app-1234-fix-header', 'worktree', 'fix/1234-header', {
      status: status({ modified: 2, untracked: 1, unpushed: 1 }),
    }),
    workspace('app-b', 'clone', 'feature/sso-login', { status: status({ behind: 3 }) }),
    workspace('app-billing', 'worktree', 'fix/billing-rounding'),
    workspace('app-search', 'worktree', 'feature/search-index', {
      status: null,
      statusError: 'git status timed out after 30s',
    }),
  ]
  const branches = [
    branch('main', { hasLocal: true, committedAt: now() - 3600, subject: 'Merge fix/billing-rounding' }),
    branch('fix/1234-header', { hasLocal: true, committedAt: now() - 1800, subject: 'Align the header on narrow screens' }),
    branch('feature/sso-login', { hasLocal: true, committedAt: now() - DAY, subject: 'Add SAML login' }),
    branch('fix/billing-rounding', { hasLocal: true, merged: true, committedAt: now() - 4 * DAY, subject: 'Round invoices half-even' }),
    branch('feature/search-index', { hasLocal: true, committedAt: now() - 6 * DAY, subject: 'Index issues for search' }),
    branch('maintenance/bump-deps', { committedAt: now() - 9 * DAY, subject: 'Bump Rails to 8.1' }),
    branch('spike/websocket-presence', {
      hasLocal: true,
      hasRemote: false,
      wasPushed: false,
      committedAt: now() - 40 * DAY,
      subject: 'Try presence over websockets',
    }),
    branch('fix/old-typo', { hasLocal: true, hasRemote: false, merged: true, committedAt: now() - 70 * DAY, subject: 'Fix a typo' }),
  ]
  return {
    project: project('app'),
    baseBranch: 'main',
    slug: 'example/app',
    branches,
    workspaces,
    pullRequests: {
      'fix/1234-header': pr(1234, 'fix/1234-header', { assignees: ['sam'] }),
      'feature/sso-login': pr(1301, 'feature/sso-login', { isDraft: true, checks: 'pending' }),
      'fix/billing-rounding': pr(1288, 'fix/billing-rounding', { state: 'merged' }),
      'maintenance/bump-deps': pr(1310, 'maintenance/bump-deps', { checks: 'failing', reviewDecision: 'CHANGES_REQUESTED' }),
    },
    viewer: 'sam',
    warnings: [],
    ...overrides,
  }
}

/** The same overview as `api.peek` gives it: what is on disk, nothing measured. */
export function peeked(source: Overview = overview()): Overview {
  return {
    ...source,
    pullRequests: {},
    workspaces: source.workspaces.map((w) => ({ ...w, status: null, statusError: null })),
  }
}

let nextTask = 1

export const line = (n: number, text: string, error = false): TaskLine => ({ n, text, error })

/** A task as the registry lists it, `ago` seconds old. */
export function task(overrides: Partial<Task> & Pick<Task, 'kind' | 'title' | 'status'>): Task {
  const created = Date.now() - 20_000
  const finished = overrides.status === 'done' || overrides.status === 'failed'
  return {
    id: `task-${nextTask++}`,
    projectId: 'app',
    attention: false,
    createdAt: created,
    startedAt: overrides.status === 'queued' ? null : created + 200,
    finishedAt: finished ? Date.now() - 1000 : null,
    branch: null,
    path: null,
    mode: null,
    prUrl: null,
    openTargets: [],
    rev: nextTask,
    log: [],
    ...overrides,
  }
}

export const installLog: TaskLine[] = [
  line(0, `${STEP_MARK}Creating the worktree`),
  line(1, 'Preparing worktree (checking out \'fix/1234-header\')'),
  line(2, `${STEP_MARK}bundle install`),
  line(3, 'Fetching gem metadata from https://rubygems.org/'),
  line(4, 'Installing rack 3.1.8'),
  line(5, `${STEP_MARK}pnpm install`),
  line(6, 'Progress: resolved 1412, reused 1408, downloaded 4, added 1412, done'),
]
