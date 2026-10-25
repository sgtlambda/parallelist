import type {
  AppConfig,
  BranchInfo,
  Issue,
  OpenTarget,
  Overview,
  ProjectConfig,
  PullRequest,
  SetupStep,
  Workspace,
  WorkspaceKind,
  WorkspaceStatus,
} from '../../src/api'

/**
 * The made-up machine the demo runs on: four projects with their branches,
 * workspaces and pull requests. Times are relative to when the page loads,
 * so "2h" is always two hours.
 */

export const HOME = '/Users/you'
const CODE = `${HOME}/Code`

const HOUR = 3600
const DAY = 86_400
const ago = (seconds: number) => Math.floor(Date.now() / 1000 - seconds)

function sha(seed: string): string {
  let h = 2166136261
  let out = ''
  for (let i = 0; out.length < 40; i++) {
    h = Math.imul(h ^ seed.charCodeAt(i % seed.length) ^ i, 16777619)
    out += (h >>> 0).toString(16).padStart(8, '0')
  }
  return out.slice(0, 40)
}

export interface ProjectData {
  branches: BranchInfo[]
  workspaces: Workspace[]
  prs: Record<string, PullRequest>
  slug: string
  /** What `ls` shows in its main workspace. */
  files: string[]
  issues: Issue[]
}

function branch(
  name: string,
  age: number,
  subject: string,
  author = 'you',
  extra: Partial<BranchInfo> = {},
): BranchInfo {
  return {
    name,
    hasRemote: true,
    hasLocal: false,
    sha: sha(name),
    committedAt: ago(age),
    author,
    subject,
    merged: false,
    wasPushed: true,
    ...extra,
  }
}

const CLEAN: WorkspaceStatus = { modified: 0, untracked: 0, unpushed: 0, behind: 0 }

function workspace(
  kind: WorkspaceKind,
  path: string,
  branchName: string,
  owner: string | null,
  status: Partial<WorkspaceStatus> = {},
): Workspace {
  return { kind, path, branch: branchName, owner, status: { ...CLEAN, ...status }, statusError: null }
}

function pr(
  number: number,
  title: string,
  branchName: string,
  slug: string,
  extra: Partial<PullRequest> = {},
): PullRequest {
  return {
    number,
    title,
    branch: branchName,
    state: 'open',
    isDraft: false,
    url: `https://github.com/${slug}/pull/${number}`,
    author: 'you',
    reviewDecision: '',
    checks: 'passing',
    assignees: ['you'],
    ...extra,
  }
}

const SHOP = `${CODE}/shop`
const API = `${CODE}/api`
const DS = `${CODE}/design-system`
const INFRA = `${CODE}/infra`

export const world: Record<string, ProjectData> = {
  shop: {
    slug: 'acme/shop',
    files: ['README.md', 'composer.json', 'package.json', 'playwright.config.ts', 'public', 'src', 'tests', 'vendor'],
    branches: [
      branch('main', 3 * HOUR, 'Merge pull request #1249 from acme/opcache-preload', 'dana', {
        hasLocal: true,
      }),
      branch('1234-e2e-flake', 1 * HOUR, 'wait for iframe ready before pay', 'you', { hasLocal: true }),
      branch('idem-keys', 5 * HOUR, 'idempotency key middleware', 'you', { hasLocal: true }),
      branch('reserve-lock', 2 * HOUR, 'lock skus in id order', 'you', { hasLocal: true }),
      branch('php84', 3 * DAY, 'php 8.4', 'sam', { hasLocal: true, hasRemote: false, merged: true }),
      branch('csp-nonce', 2 * DAY, 'nonce inline scripts', 'dana'),
      branch('cron-tz', 4 * DAY, 'digest at 08:00 local', 'sam'),
      branch('rm-moment', 6 * DAY, 'moment -> Intl', 'sam'),
      branch('sessions-db2', 9 * DAY, 'move sessions to redis db 2', 'sam'),
      branch('eslint9', 12 * DAY, 'flat config', 'dana'),
      branch('sam/tmp-debug', 15 * DAY, 'debug logging, do not merge', 'sam'),
      branch('logical-repl-spike', 21 * DAY, 'wip', 'you'),
      branch('no-webpack', 160 * DAY, 'rm webpack', 'dana', { merged: true }),
    ],
    workspaces: [
      workspace('primary', SHOP, 'main', null),
      workspace('worktree', `${CODE}/shop-1234-e2e-flake`, '1234-e2e-flake', SHOP, {
        modified: 2,
        untracked: 1,
      }),
      workspace('worktree', `${CODE}/shop-idem-keys`, 'idem-keys', SHOP, { unpushed: 2 }),
      workspace('worktree', `${CODE}/shop-reserve-lock`, 'reserve-lock', SHOP),
      workspace('clone', `${CODE}/shop-b`, 'php84', null),
    ],
    prs: {
      '1234-e2e-flake': pr(1258, 'Wait for the payment iframe before clicking Pay', '1234-e2e-flake', 'acme/shop', {
        reviewDecision: 'REVIEW_REQUIRED',
      }),
      'reserve-lock': pr(1251, 'Lock SKUs in id order', 'reserve-lock', 'acme/shop', {
        checks: 'pending',
      }),
      'sessions-db2': pr(1244, 'Move sessions to redis db 2', 'sessions-db2', 'acme/shop', {
        author: 'sam',
        isDraft: true,
        assignees: ['sam'],
        checks: 'failing',
      }),
      php84: pr(1198, 'PHP 8.4', 'php84', 'acme/shop', {
        state: 'merged',
        author: 'sam',
        assignees: [],
      }),
    },
    issues: [
      issue(1262, 'OPcache stale for ~1 min after deploy', 'Bug', 1),
      issue(1259, 'Webhook signature check fails on a BOM', 'Bug', 3),
      issue(1255, 'Soft delete customers', 'Feature', 6),
      issue(1241, 'srcset on product images', 'Task', 20),
      issue(1230, 'SAML for the staff dashboard', 'Feature', 30),
    ],
  },
  api: {
    slug: 'acme/api',
    files: ['Dockerfile', 'README.md', 'package.json', 'src', 'test', 'openapi.yaml'],
    branches: [
      branch('main', 5 * HOUR, 'Merge pull request #809 from acme/cursor-off-by-one', 'sam', {
        hasLocal: true,
      }),
      branch('retry-backoff', 3 * HOUR, 'retry with backoff', 'you', { hasLocal: true }),
      branch('n+1-orders', 1 * DAY, 'one query for order lines', 'sam'),
      branch('econnreset', 2 * DAY, 'retry idempotent reqs on ECONNRESET', 'sam'),
      branch('ratelimit-tb', 4 * DAY, 'token bucket', 'dana'),
      branch('node22', 6 * DAY, 'node:22-alpine', 'you'),
      branch('oas31', 8 * DAY, 'openapi 3.1', 'you'),
    ],
    workspaces: [
      workspace('primary', API, 'main', null),
      workspace('worktree', `${CODE}/api-retry-backoff`, 'retry-backoff', API),
    ],
    prs: {
      'retry-backoff': pr(812, 'Webhook retries', 'retry-backoff', 'acme/api'),
      'n+1-orders': pr(820, 'Order lines in one query', 'n+1-orders', 'acme/api', {
        author: 'sam',
        assignees: ['sam'],
      }),
    },
    issues: [],
  },
  'design-system': {
    slug: 'acme/design-system',
    files: ['README.md', 'package.json', 'packages', 'stories', 'tokens'],
    branches: [
      branch('main', 1 * DAY, 'v4.2.0', 'dana', { hasLocal: true }),
      branch('css-layers', 2 * HOUR, '@layer components', 'you', { hasLocal: true }),
      branch('focus-ring-clip', 2 * DAY, 'focus ring clipped in overflow', 'sam'),
      branch('popover-toplayer', 3 * DAY, 'dialog + popover in top layer', 'you'),
      branch('oklch', 7 * DAY, 'tokens in oklch', 'dana'),
    ],
    workspaces: [
      workspace('primary', DS, 'main', null),
      workspace('worktree', `${CODE}/design-system-css-layers`, 'css-layers', DS, {
        modified: 4,
      }),
    ],
    prs: {
      'focus-ring-clip': pr(377, 'Focus ring clipped in overflow containers', 'focus-ring-clip', 'acme/design-system', {
        author: 'sam',
        assignees: [],
        reviewDecision: 'APPROVED',
      }),
    },
    issues: [],
  },
  infra: {
    slug: 'acme/infra',
    files: ['README.md', 'modules', 'envs', 'Makefile'],
    branches: [
      branch('main', 2 * DAY, 'Merge pull request #204 from acme/cert-rotation', 'dana', {
        hasLocal: true,
      }),
      branch('spot-drain', 1 * DAY, 'drain on interruption notice', 'you'),
      branch('tf-ephemeral', 4 * DAY, 'ephemeral db password', 'dana'),
      branch('eks-1.32', 5 * DAY, '1.32', 'you'),
      branch('alb-timeout', 11 * DAY, 'idle timeout 120s', 'sam'),
    ],
    workspaces: [workspace('primary', INFRA, 'main', null)],
    prs: {
      'eks-1.32': pr(211, 'EKS 1.32', 'eks-1.32', 'acme/infra', {
        checks: 'pending',
        isDraft: true,
      }),
    },
    issues: [],
  },
}

function issue(number: number, title: string, issueType: string, days: number): Issue {
  return {
    number,
    title,
    url: `https://github.com/acme/shop/issues/${number}`,
    updatedAt: new Date(Date.now() - days * DAY * 1000).toISOString(),
    issueType,
    linkedBranches: [],
  }
}

const SETUP: SetupStep[] = [
  {
    id: 'copy-env',
    label: 'Copy untracked env files from the main workspace',
    defaultEnabled: true,
    script: 'for f in .env; do [ -e {primary}/$f ] && cp {primary}/$f .; done',
  },
  { id: 'install', label: 'pnpm install', defaultEnabled: true, script: 'pnpm install' },
]

/** Only targets that type into the built-in terminal: the others would
 *  launch apps on a machine the page doesn't have. */
const TARGETS: OpenTarget[] = [
  {
    id: 'agent',
    label: 'Agent',
    icon: '✳',
    command: 'agent',
    kind: 'terminal',
    primary: true,
    requiresPr: false,
    runOnCreate: true,
  },
  {
    id: 'dev',
    label: 'Dev server',
    icon: '▶',
    command: 'pnpm dev',
    kind: 'terminal',
    primary: true,
    requiresPr: false,
    runOnCreate: false,
  },
]

const PROJECTS: [string, string][] = [
  ['shop', SHOP],
  ['api', API],
  ['design-system', DS],
  ['infra', INFRA],
]

export const config: AppConfig = {
  projects: PROJECTS.map(
    ([id, path]): ProjectConfig => ({
      id,
      name: id,
      path,
      baseBranch: 'main',
      pathTemplate: '{parent}/{project}-{slug}',
      setupSteps: structuredClone(SETUP),
      openTargets: [],
    }),
  ),
  openTargets: TARGETS,
  fetchIntervalSecs: 180,
  fetchOnFocus: true,
  prRefreshSecs: 120,
  activeDays: 30,
  selectedProjectId: 'shop',
  // Narrower than the app's defaults: the page shows it at about 980px.
  sidebarWidth: 184,
  listWidth: 262,
  detailWidth: 360,
  terminalFontSize: 12,
}

/** What the frontend receives is always a copy, so its reactive proxies never
 *  alias the demo's own state. */
const copy = <T>(value: T): T => JSON.parse(JSON.stringify(value))

export function project(id: string): ProjectConfig {
  const found = config.projects.find((p) => p.id === id)
  if (!found) throw new Error(`No project ${id}`)
  return found
}

/** `Overview` as `refresh` returns it, or as `peek` does: only what is on
 *  disk, with every status still to be read. */
export function overview(id: string, peek = false): Overview {
  const data = world[id]
  return copy({
    project: project(id),
    baseBranch: 'main',
    slug: data.slug,
    branches: data.branches,
    workspaces: peek
      ? data.workspaces.map((w) => ({ ...w, status: null, statusError: null }))
      : data.workspaces,
    pullRequests: peek ? {} : data.prs,
    viewer: 'you',
    warnings: [],
  })
}

/** The branch and project a workspace directory belongs to. */
export function locate(path: string) {
  for (const [id, data] of Object.entries(world)) {
    const found = data.workspaces.find((w) => w.path === path)
    if (found) return { projectId: id, workspace: found, data }
  }
  return null
}

export function slugify(branchName: string): string {
  return branchName
    .replace(/^[a-z]+\//, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
}

/** A branch's stand-in for "how far from main", stable per name. */
export function aheadBehind(name: string): { ahead: number; behind: number } {
  if (name === 'main') return { ahead: 0, behind: 0 }
  const h = parseInt(sha(name).slice(0, 6), 16)
  return { ahead: (h % 7) + 1, behind: (h >> 3) % 40 }
}

export function shortHome(path: string): string {
  return path.startsWith(HOME) ? `~${path.slice(HOME.length)}` : path
}

export { copy }
