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
    files: ['README.md', 'composer.json', 'package.json', 'public', 'src', 'tests', 'vendor'],
    branches: [
      branch('main', 3 * HOUR, 'Merge pull request #1249 from acme/fix/checkout-tax', 'dana', {
        hasLocal: true,
      }),
      branch('1234-fix-header', 1 * HOUR, 'Stop the sticky header covering the cart drawer', 'you', {
        hasLocal: true,
      }),
      branch('feature/sso-login', 5 * HOUR, 'Add the SAML callback handler', 'you', {
        hasLocal: true,
      }),
      branch('fix/invoice-rounding', 2 * HOUR, 'Round invoice totals half-even', 'you', {
        hasLocal: true,
      }),
      branch('maintenance/php-8.4', 3 * DAY, 'Bump the runtime to PHP 8.4', 'sam', {
        hasLocal: true,
        hasRemote: false,
        merged: true,
      }),
      branch('feature/gift-cards', 2 * DAY, 'Gift card balance on the order summary', 'dana'),
      branch('feature/wishlist', 6 * DAY, 'Move the wishlist behind its own route', 'sam'),
      branch('fix/cart-total', 9 * DAY, 'Cart total ignores the free-shipping threshold', 'sam'),
      branch('chore/bump-deps', 12 * DAY, 'Bump the front-end dependencies', 'dana'),
      branch('feature/returns-portal', 21 * DAY, 'First pass at self-service returns', 'you'),
      branch('legacy/checkout-v1', 160 * DAY, 'Remove the v1 checkout', 'dana', { merged: true }),
    ],
    workspaces: [
      workspace('primary', SHOP, 'main', null),
      workspace('worktree', `${CODE}/shop-1234-fix-header`, '1234-fix-header', SHOP, {
        modified: 2,
        untracked: 1,
      }),
      workspace('worktree', `${CODE}/shop-sso-login`, 'feature/sso-login', SHOP, { unpushed: 2 }),
      workspace('worktree', `${CODE}/shop-1251-invoice-rounding`, 'fix/invoice-rounding', SHOP),
      workspace('clone', `${CODE}/shop-b`, 'maintenance/php-8.4', null),
    ],
    prs: {
      '1234-fix-header': pr(1234, 'Stop the sticky header covering the cart drawer', '1234-fix-header', 'acme/shop', {
        reviewDecision: 'REVIEW_REQUIRED',
      }),
      'fix/invoice-rounding': pr(1251, 'Round invoice totals half-even', 'fix/invoice-rounding', 'acme/shop', {
        checks: 'pending',
      }),
      'fix/cart-total': pr(1244, 'Cart total ignores the free-shipping threshold', 'fix/cart-total', 'acme/shop', {
        author: 'sam',
        isDraft: true,
        assignees: ['sam'],
        checks: 'failing',
      }),
      'maintenance/php-8.4': pr(1198, 'Bump the runtime to PHP 8.4', 'maintenance/php-8.4', 'acme/shop', {
        state: 'merged',
        author: 'sam',
        assignees: [],
      }),
    },
    issues: [
      issue(1262, 'Checkout total is wrong when a gift card covers the full amount', 'Bug', 1),
      issue(1259, 'Order confirmation email shows the wrong delivery estimate', 'Bug', 3),
      issue(1255, 'Let customers change the delivery address after ordering', 'Feature', 6),
      issue(1241, 'Product images load at full size on mobile', 'Task', 20),
      issue(1230, 'Add a SSO login for the staff dashboard', 'Feature', 30),
    ],
  },
  api: {
    slug: 'acme/api',
    files: ['Dockerfile', 'README.md', 'package.json', 'src', 'test', 'openapi.yaml'],
    branches: [
      branch('main', 5 * HOUR, 'Merge pull request #809 from acme/fix/pagination', 'sam', {
        hasLocal: true,
      }),
      branch('feature/webhooks-retry', 3 * HOUR, 'Retry failed webhooks with backoff and jitter', 'you', {
        hasLocal: true,
      }),
      branch('fix/n-plus-one-orders', 1 * DAY, 'Load order lines in one query', 'sam'),
      branch('feature/rate-limits', 4 * DAY, 'Per-key rate limits', 'dana'),
      branch('chore/openapi-3.1', 8 * DAY, 'Move the spec to OpenAPI 3.1', 'you'),
    ],
    workspaces: [
      workspace('primary', API, 'main', null),
      workspace('worktree', `${CODE}/api-webhooks-retry`, 'feature/webhooks-retry', API),
    ],
    prs: {
      'feature/webhooks-retry': pr(812, 'Retry failed webhooks with backoff and jitter', 'feature/webhooks-retry', 'acme/api'),
      'fix/n-plus-one-orders': pr(820, 'Load order lines in one query', 'fix/n-plus-one-orders', 'acme/api', {
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
      branch('main', 1 * DAY, 'Release 4.2.0', 'dana', { hasLocal: true }),
      branch('feature/button-variants', 2 * HOUR, 'Add ghost and link button variants', 'you', {
        hasLocal: true,
      }),
      branch('fix/focus-ring', 2 * DAY, 'Focus ring clipped inside overflow containers', 'sam'),
      branch('chore/tokens-v2', 7 * DAY, 'Rename the spacing tokens', 'dana'),
    ],
    workspaces: [
      workspace('primary', DS, 'main', null),
      workspace('worktree', `${CODE}/design-system-button-variants`, 'feature/button-variants', DS, {
        modified: 4,
      }),
    ],
    prs: {
      'fix/focus-ring': pr(377, 'Focus ring clipped inside overflow containers', 'fix/focus-ring', 'acme/design-system', {
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
      branch('main', 2 * DAY, 'Merge pull request #204 from acme/chore/cert-rotation', 'dana', {
        hasLocal: true,
      }),
      branch('chore/upgrade-eks', 5 * DAY, 'Upgrade the cluster to 1.32', 'you'),
      branch('fix/alb-timeouts', 11 * DAY, 'Raise the load balancer idle timeout', 'sam'),
    ],
    workspaces: [workspace('primary', INFRA, 'main', null)],
    prs: {
      'chore/upgrade-eks': pr(211, 'Upgrade the cluster to 1.32', 'chore/upgrade-eks', 'acme/infra', {
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
