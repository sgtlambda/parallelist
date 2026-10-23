import CleanupDialog from '../../components/CleanupDialog.vue'
import type { BranchImpact, RemovalImpact } from '../../api'
import { backend } from '../backend'
import { boot } from '../seed'
import { defineGroup } from '../define'
import { status } from '../fixtures'

const WS = '/Users/sandbox/Code'

const impact = (dir: string, overrides: Partial<RemovalImpact>): RemovalImpact => ({
  path: `${WS}/${dir}`,
  kind: 'worktree',
  branch: null,
  status: status(),
  error: null,
  commits: 0,
  commitsNeedBranchDelete: false,
  stashes: 0,
  dependents: [],
  ...overrides,
})

const impacts: RemovalImpact[] = [
  impact('app-billing', { branch: 'fix/billing-rounding' }),
  impact('app-1234-fix-header', {
    branch: 'fix/1234-header',
    status: status({ modified: 2, unpushed: 1 }),
    commits: 1,
    commitsNeedBranchDelete: true,
  }),
  impact('app-b', {
    kind: 'clone',
    branch: 'feature/sso-login',
    status: status({ modified: 1 }),
    commits: 14,
    stashes: 3,
    dependents: [`${WS}/app-b-hotfix`, `${WS}/app-b-review`],
  }),
  impact('app-search', { branch: 'feature/search-index', status: null, error: 'git status timed out after 30s' }),
]

const branchImpacts: BranchImpact[] = [
  { branch: 'fix/old-typo', commits: [], commitCount: 0 },
  {
    branch: 'spike/websocket-presence',
    commitCount: 2,
    commits: [
      { sha: 'a1b2c3d', subject: 'Try presence over websockets', author: 'Sam Rivera', committedAt: Date.now() / 1000 - 40 * 86_400 },
      { sha: 'e4f5a6b', subject: 'Sketch the heartbeat', author: 'Sam Rivera', committedAt: Date.now() / 1000 - 41 * 86_400 },
    ],
  },
]

async function seed() {
  await boot()
  backend.handle({
    removal_impacts: (args: { paths: string[] }) => impacts.filter((i) => args.paths.includes(i.path)),
    branch_impacts: (args: { branches: string[] }) => branchImpacts.filter((b) => args.branches.includes(b.branch)),
    workspace_sizes: () => ({
      [`${WS}/app-billing`]: 1_800_000_000,
      [`${WS}/app-1234-fix-header`]: 2_400_000_000,
      [`${WS}/app-b`]: 3_100_000_000,
      [`${WS}/app-search`]: 900_000_000,
    }),
    remove_workspaces: () => [],
    delete_branches: () => [],
  })
}

export default defineGroup('Cleanup dialog', [
  {
    id: 'workspaces',
    title: 'Workspaces at different costs',
    note: 'Clean, uncommitted work, a clone with stashes and stranded worktrees, and one that could not be read. Only the free one starts ticked.',
    component: CleanupDialog,
    props: {
      open: true,
      title: 'Cleanup workspaces',
      paths: impacts.map((i) => i.path),
      branches: [],
    },
    setup: seed,
  },
  {
    id: 'branches',
    title: 'Branches',
    component: CleanupDialog,
    props: { open: true, title: 'Cleanup branches', paths: [], branches: branchImpacts.map((b) => b.branch) },
    setup: seed,
  },
  {
    id: 'single',
    title: 'One item, named on its own',
    note: 'Ticked even though it holds work: it is what the user asked for.',
    component: CleanupDialog,
    props: { open: true, title: 'Remove workspace', paths: [impacts[1].path], branches: [] },
    setup: seed,
  },
])
