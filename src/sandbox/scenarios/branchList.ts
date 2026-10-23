import BranchList from '../../components/BranchList.vue'
import Steps, { type Step } from '../views/Steps.vue'
import { backend } from '../backend'
import { boot, stopTerminals, terminals } from '../seed'
import { defineGroup } from '../define'
import { branch, overview, workspace } from '../fixtures'
import { refresh } from '../../store'
import type { Overview } from '../../api'

const box = { layout: 'box' as const, size: { width: 420, height: 640 } }
const WS = '/Users/sandbox/Code'

const sso = `${WS}/app-b`
const billing = `${WS}/app-billing`
const search = `${WS}/app-search`

/** Terminals starting, coming to need the user and going: each a change of
 *  tier, which is what re-sorts the list on its own. Ends where it began. */
const tierSteps: Step[] = [
  { label: 'A shell starts in app-search', run: () => terminals([{ path: search }]) },
  { label: 'The program in app-billing is blocked', run: () => terminals([{ path: billing, status: 'blocked' }]) },
  { label: 'A turn ends unseen in app-b', run: () => terminals([{ path: sso, status: 'idle', ready: true }]) },
  { label: 'app-billing is answered and works on', run: () => terminals([{ path: billing, status: 'working' }]) },
  { label: 'The program in app-search is blocked', run: () => terminals([{ path: search, status: 'blocked' }]) },
  { label: 'app-search is answered, and its shell exits', run: () => stopTerminals([search]) },
  { label: 'Every shell exits', run: () => stopTerminals([billing, sso]) },
]

/** The overview with these changes, its branches newest commit first, as
 *  the backend sorts them. */
function changed(changes: { committed?: Record<string, number>; add?: string[]; drop?: string[] }): Overview {
  const base = overview()
  const now = Math.floor(Date.now() / 1000)
  const branches = [
    ...base.branches
      .filter((b) => !changes.drop?.includes(b.name))
      .map((b) => (b.name in (changes.committed ?? {}) ? { ...b, committedAt: now - changes.committed![b.name] } : b)),
    ...(changes.add ?? []).map((name) => branch(name, { committedAt: now - 30, subject: 'First pass' })),
  ].sort((a, b) => b.committedAt - a.committedAt)
  return { ...base, branches }
}

/** Answer the next refresh with this, and run one. */
async function refreshTo(next: Overview) {
  backend.handle({ refresh: () => structuredClone(next) })
  await refresh()
}

const fresh = { 'maintenance/bump-deps': 60, 'feature/search-index': 120 }
const refreshSteps: Step[] = [
  { label: 'New commits on maintenance/bump-deps and feature/search-index', run: () => refreshTo(changed({ committed: fresh })) },
  { label: 'feature/csv-export is pushed', run: () => refreshTo(changed({ committed: fresh, add: ['feature/csv-export'] })) },
  {
    label: 'spike/websocket-presence is deleted',
    run: () => refreshTo(changed({ committed: fresh, add: ['feature/csv-export'], drop: ['spike/websocket-presence'] })),
  },
  { label: 'Back to how it began', run: () => refreshTo(overview()) },
]

/** Enough branches to scroll, and a workspace on one near the end. */
function long(): Overview {
  const base = overview()
  const now = Math.floor(Date.now() / 1000)
  const topics = [
    'ci-cache', 'avatar-upload', 'invite-flow', 'pdf-receipts', 'locale-nl', 'slow-query-log',
    'retry-webhooks', 'dark-mode', 'csv-import', 'audit-trail', 'seat-billing', 'oauth-github',
    'mobile-nav', 'trial-banner', 'bulk-archive', 'rate-limits', 'search-facets', 'email-digest',
    'calendar-sync', 'pagination', 'feature-flags', 'cookie-consent', 'two-factor', 'sitemap',
  ]
  const extra = topics.map((t, i) =>
    branch(`feature/${t}`, { committedAt: now - (10 + i) * 86_400, subject: `Work on ${t.replace(/-/g, ' ')}` }),
  )
  return {
    ...base,
    branches: [
      ...base.branches,
      ...extra,
      branch('feature/legacy-export', { hasLocal: true, committedAt: now - 80 * 86_400, subject: 'Export to the old format' }),
    ],
    workspaces: [...base.workspaces, workspace('app-legacy-export', 'worktree', 'feature/legacy-export')],
  }
}

const legacy = `${WS}/app-legacy-export`
const farSteps: Step[] = [
  { label: 'app-legacy-export, far down the list, is blocked', run: () => terminals([{ path: legacy, status: 'blocked' }]) },
  { label: 'It is answered, and its shell exits', run: () => stopTerminals([legacy]) },
]

export default defineGroup('Branch list', [
  {
    id: 'mixed',
    title: 'Mixed',
    note: 'A main workspace, a worktree with work in it, a clone, finished work, a workspace git could not read.',
    component: BranchList,
    ...box,
    setup: () => boot({ filter: 'all', selected: 'fix/1234-header' }),
  },
  {
    id: 'terminals',
    title: 'Running terminals sort first',
    note: 'Blocked, then ready, then the rest, after the pinned main workspace.',
    component: BranchList,
    ...box,
    async setup() {
      await boot({ filter: 'workspaces' })
      terminals([
        { path: `${WS}/app-billing`, minutesAgo: 1 },
        { path: `${WS}/app-b`, status: 'working', minutesAgo: 3 },
        { path: `${WS}/app-1234-fix-header`, status: 'blocked', minutesAgo: 9 },
        { path: `${WS}/app-search`, status: 'idle', ready: true, minutesAgo: 30 },
      ])
    },
  },
  {
    id: 'removed',
    title: 'A removed workspace whose terminal is still open',
    note: 'Not in the overview, so listed on the branch its terminal was opened on until nothing in it needs the user.',
    component: BranchList,
    ...box,
    async setup() {
      const gone = workspace('app-billing', 'worktree', 'fix/billing-rounding')
      const base = overview()
      await boot({
        filter: 'workspaces',
        overview: { ...base, workspaces: base.workspaces.filter((w) => w.path !== gone.path) },
      })
      terminals([{ path: gone.path, branch: 'fix/billing-rounding', status: 'idle', ready: true }])
    },
  },
  {
    id: 'removed-branch',
    title: 'Removed along with its branch',
    note: 'The branch is on no ref any more, so the terminal brings its row back, as deleted.',
    component: BranchList,
    ...box,
    async setup() {
      const gone = workspace('app-billing', 'worktree', 'fix/billing-rounding')
      const base = overview()
      await boot({
        filter: 'active',
        overview: {
          ...base,
          branches: base.branches.filter((b) => b.name !== gone.branch),
          workspaces: base.workspaces.filter((w) => w.path !== gone.path),
        },
      })
      terminals([{ path: gone.path, branch: 'fix/billing-rounding', status: 'blocked' }])
    },
  },
  {
    id: 'reorder-terminals',
    title: 'Reordering: terminals change tier',
    note: 'Rows slide to their new place, and the one that moves furthest passes over the others. Switching tab or typing a search is another list, so it doesn\'t slide.',
    component: Steps,
    props: { steps: tierSteps },
    ...box,
    setup: () => boot({ filter: 'all', selected: 'fix/billing-rounding' }),
  },
  {
    id: 'reorder-refresh',
    title: 'Reordering: a refresh lands',
    note: 'New commits move branches up; a new branch fades in, a deleted one goes and the rows after it close up.',
    component: Steps,
    props: { steps: refreshSteps },
    ...box,
    setup: () => boot({ filter: 'all' }),
  },
  {
    id: 'reorder-far',
    title: 'Reordering: a row from out of sight',
    note: 'The row comes in from the bottom edge rather than from where it was. Scroll down to watch it leave from where it was.',
    component: Steps,
    props: { steps: farSteps, every: 3000 },
    ...box,
    setup: () => boot({ filter: 'all', overview: long() }),
  },
  {
    id: 'reorder-rapid',
    title: 'Reordering: faster than a slide',
    note: 'A step every 180ms: each slide starts from wherever the last one had got to.',
    component: Steps,
    props: { steps: tierSteps, every: 180 },
    ...box,
    setup: () => boot({ filter: 'all', selected: 'fix/billing-rounding' }),
  },
  {
    id: 'checking',
    title: 'First seconds: "checking…"',
    note: 'The peek has landed — local refs and workspaces — and the refresh has not.',
    component: BranchList,
    ...box,
    setup: () => boot({ loading: true, filter: 'workspaces' }),
  },
  {
    id: 'loading',
    title: 'Nothing yet: skeleton',
    component: BranchList,
    ...box,
    setup: () => boot({ loading: 'nothing' }),
  },
])
