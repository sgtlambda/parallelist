import Window from '../views/Window.vue'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'
import { branch, installLog, overview, task, workspace } from '../fixtures'
import { state } from '../../store'
import { normalizePath, startTaskSync } from '../../tasks'
import { showTerminal } from '../../terminal'
import type { Overview, Task } from '../../api'

const WS = '/Users/sandbox/Code'
const queue = { show: 'queue' }

type Seed = Parameters<typeof terminals>[0]

/**
 * The window with these terminals in it. `asked` is what each waiting
 * terminal's transcript ends on (`terminal_question`), which the card shows
 * for a wait, since `done` carries no message of its own. `here` is the
 * branch whose terminal is on screen.
 */
async function withQueue(
  list: Seed,
  options: { asked?: Record<string, string>; here?: { branch: string; path: string }; overview?: Overview } = {},
) {
  await boot({ overview: options.overview, selected: options.here?.branch ?? 'main' })
  state.now = Date.now()
  const asked = new Map(Object.entries(options.asked ?? {}).map(([path, q]) => [normalizePath(path), q]))
  backend.handle({ terminal_question: ({ path }: { path: string }) => asked.get(path) ?? null })
  terminals(list)
  if (options.here) showTerminal('app', options.here.branch, options.here.path)
}

/** `withQueue`, with these tasks in the registry as well. */
async function withTasks(list: Seed, tasks: Task[]) {
  await withQueue(list)
  backend.handle({ list_tasks: () => structuredClone(tasks) })
  await startTaskSync(() => {})
}

const creating = () =>
  task({
    kind: 'create',
    title: 'Create fix/1234-header',
    status: 'running',
    branch: 'fix/1234-header',
    path: `${WS}/app-1234-fix-header`,
    mode: 'worktree',
    log: installLog,
  })

const header = `${WS}/app-1234-fix-header`
const sso = `${WS}/app-b`
const billing = `${WS}/app-billing`
const search = `${WS}/app-search`

/** Enough workspaces to outrun the strip, each on a branch of its own. */
function crowded(): Overview {
  const base = overview()
  const names = ['dark-mode', 'csv-export', 'rate-limits', 'audit-log', 'webhooks', 'sso-scim']
  return {
    ...base,
    branches: [...base.branches, ...names.map((n) => branch(`feature/${n}`, { hasLocal: true }))],
    workspaces: [...base.workspaces, ...names.map((n) => workspace(`app-${n}`, 'worktree', `feature/${n}`))],
  }
}

export default defineGroup('Attention queue', [
  {
    id: 'empty',
    title: 'Nothing waiting',
    note: 'The strip is there anyway, so the panes above never change height. The logo sits in the middle, its frames lighting up back to front.',
    component: Window,
    props: queue,
    setup: () => withQueue([]),
  },
  {
    id: 'empty-task',
    title: 'Nothing waiting, a task running',
    note: 'The task box takes the middle of the empty strip, and the logo gives way to it.',
    component: Window,
    props: queue,
    setup: () => withTasks([], [creating()]),
  },
  {
    id: 'waiting-task',
    title: 'Cards waiting, a task running',
    note: 'With cards in the strip the box stays at the right end, over their fade.',
    component: Window,
    props: queue,
    setup: () =>
      withTasks([{ path: header, status: 'blocked', message: 'Run pnpm test', minutesAgo: 5 }], [creating()]),
  },
  {
    id: 'waiting',
    title: 'Blocked and ready, oldest first',
    note: 'First in, first out: blocked does not jump ahead of ready. The first card carries ⌘↵. A blocked card shows its own message; a ready one shows what its turn ended on.',
    component: Window,
    props: queue,
    setup: () =>
      withQueue(
        [
          { path: sso, status: 'idle', ready: true, minutesAgo: 41 },
          { path: header, status: 'blocked', message: 'Run bundle exec rspec spec/models', minutesAgo: 12 },
          { path: billing, status: 'idle', ready: true, minutesAgo: 2 },
        ],
        {
          asked: {
            [sso]: 'Should the SAML assertion be required to be signed, or only the response?',
            [billing]: 'Rounded half-even in the invoice model and updated the spec.',
          },
        },
      ),
  },
  {
    id: 'here',
    title: 'The terminal on screen keeps its card',
    note: 'Marked "here", and ⌘↵ goes past it to the next one.',
    component: Window,
    props: queue,
    setup: () =>
      withQueue(
        [
          { path: header, status: 'blocked', message: 'Run pnpm test', minutesAgo: 15 },
          { path: sso, status: 'idle', ready: true, minutesAgo: 6 },
        ],
        {
          here: { branch: 'fix/1234-header', path: header },
          asked: { [sso]: 'Want the login button on the left or the right?' },
        },
      ),
  },
  {
    id: 'working',
    title: 'Working, behind the divider',
    note: 'Never where ⌘↵ goes, but what joins the queue next. Longest-running first.',
    component: Window,
    props: queue,
    setup: () =>
      withQueue(
        [
          { path: sso, status: 'idle', ready: true, minutesAgo: 8 },
          { path: header, status: 'working', message: 'Run the header specs', minutesAgo: 14 },
          { path: search, status: 'working', message: 'Reindexing fixtures', minutesAgo: 3 },
        ],
        { asked: { [sso]: 'Pushed. Anything else for this branch?' } },
      ),
  },
  {
    id: 'working-only',
    title: 'Only working: no divider',
    component: Window,
    props: queue,
    setup: () =>
      withQueue([
        { path: header, status: 'working', message: 'Run the header specs', minutesAgo: 14 },
        { path: search, status: 'working', minutesAgo: 1 },
      ]),
  },
  {
    id: 'crowded',
    title: 'More than fit: scrolls sideways',
    note: 'The strip keeps its height and does not wrap.',
    component: Window,
    props: queue,
    setup: () =>
      withQueue(
        [
          { path: sso, status: 'idle', ready: true, minutesAgo: 90 },
          { path: header, status: 'blocked', minutesAgo: 70 },
          { path: billing, status: 'idle', ready: true, minutesAgo: 55 },
          { path: `${WS}/app-dark-mode`, status: 'idle', ready: true, minutesAgo: 40 },
          { path: `${WS}/app-csv-export`, status: 'blocked', minutesAgo: 30 },
          { path: `${WS}/app-rate-limits`, status: 'idle', ready: true, minutesAgo: 20 },
          { path: `${WS}/app-audit-log`, status: 'working', minutesAgo: 12 },
          { path: `${WS}/app-webhooks`, status: 'working', minutesAgo: 5 },
        ],
        { overview: crowded() },
      ),
  },
  {
    id: 'long',
    title: 'Long lines truncate',
    note: 'Workspace name and message, each cut with an ellipsis rather than growing the card or pushing the age and ⌘↵ out of it.',
    component: Window,
    props: queue,
    async setup() {
      const long = 'feature/a-branch-name-long-enough-that-it-cannot-possibly-fit-in-one-card'
      const dir = 'app-a-workspace-directory-name-long-enough-that-it-cannot-fit-in-one-card'
      const base = overview()
      await withQueue(
        [
          { path: `${WS}/${dir}`, status: 'blocked', message: 'Run bundle exec rspec spec/controllers/admin/billing_reports_controller_spec.rb', minutesAgo: 9 },
          { path: `${WS}/${dir}-2`, status: 'idle', ready: true, minutesAgo: 4 },
        ],
        {
          asked: {
            [`${WS}/${dir}-2`]:
              'Do you want the report to include refunds issued after the period closed, or only those issued within it?',
          },
          overview: {
            ...base,
            branches: [...base.branches, branch(long, { hasLocal: true }), branch(`${long}-2`, { hasLocal: true })],
            workspaces: [
              ...base.workspaces,
              workspace(dir, 'worktree', long),
              workspace(`${dir}-2`, 'worktree', `${long}-2`),
            ],
          },
        },
      )
    },
  },
  {
    id: 'removed',
    title: 'Its workspace removed, and its branch',
    note: 'Removed from inside the terminal, which still waits. The list has the card\'s row too: the same placement puts both on the branch the terminal was opened on.',
    component: Window,
    props: queue,
    async setup() {
      const gone = `${WS}/app-site-copy`
      await withQueue(
        [{ path: gone, branch: 'docs/site-copy', status: 'idle', ready: true, minutesAgo: 4 }],
        { asked: { [gone]: 'Pushed to main and removed the worktree. Anything else?' } },
      )
    },
  },
])
