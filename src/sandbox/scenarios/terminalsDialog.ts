import TerminalsDialog from '../../components/TerminalsDialog.vue'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'

const WS = '/Users/sandbox/Code'

async function seed() {
  await boot()
  terminals([
    { path: `${WS}/app`, branch: 'main', minutesAgo: 240 },
    { path: `${WS}/app-1234-fix-header`, branch: 'fix/1234-header', status: 'blocked', minutesAgo: 3, message: 'Run the migrations' },
    { path: `${WS}/app-billing`, branch: 'fix/billing-rounding', status: 'idle', ready: true, minutesAgo: 12 },
    { path: `${WS}/app-b`, branch: 'feature/sso-login', minutesAgo: 60 * 26 },
    { path: `${WS}/app-search`, branch: 'feature/search-index', status: 'working', minutesAgo: 1 },
  ])
  terminals(
    [
      { path: `${WS}/parallelist`, branch: 'main', minutesAgo: 90 },
      { path: `${WS}/parallelist-session-management`, branch: 'session-management', status: 'working', minutesAgo: 0 },
    ],
    'parallelist',
  )
  terminals([{ path: `${WS}/docs-quickstart`, branch: 'docs/quickstart', minutesAgo: 60 * 24 * 6 }], 'docs')
  backend.handle({
    terminal_programs: () => ({
      [`${WS}/app`]: 'pnpm dev',
      [`${WS}/app-1234-fix-header`]: 'claude',
      [`${WS}/app-billing`]: 'claude',
      [`${WS}/app-search`]: 'claude --resume',
      [`${WS}/parallelist`]: 'pnpm tauri dev',
      [`${WS}/parallelist-session-management`]: 'claude',
    }),
    other_builds: () => [
      {
        build: 'aq-spacer-width (dev)',
        terminals: [
          {
            id: 'a1',
            path: `${WS}/parallelist-aq-spacer-width`,
            projectId: 'parallelist',
            branch: 'aq-spacer-width',
            kind: 'worktree',
            status: 'idle',
            ready: true,
            activeAt: Date.now() - 3 * 86_400_000,
            waitingSince: Date.now() - 3 * 86_400_000,
            message: null,
          },
        ],
        answering: true,
      },
      { build: 'interactive-icon', terminals: null, answering: true },
      { build: 'close-hides', terminals: null, answering: false },
    ],
  })
}

export default defineGroup('Terminals dialog', [
  {
    id: 'several',
    title: 'Across projects',
    note: 'Grouped by project, waiting first. Shells with nothing in the foreground read "shell".',
    component: TerminalsDialog,
    props: { open: true },
    setup: seed,
  },
  {
    id: 'none',
    title: 'Nothing running',
    component: TerminalsDialog,
    props: { open: true },
    setup: async () => {
      await boot()
      backend.handle({ terminal_programs: () => ({}), other_builds: () => [] })
    },
  },
])
