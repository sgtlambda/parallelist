import BranchList from '../../components/BranchList.vue'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'
import { overview, workspace } from '../fixtures'

const box = { layout: 'box' as const, size: { width: 420, height: 640 } }
const WS = '/Users/sandbox/Code'

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
