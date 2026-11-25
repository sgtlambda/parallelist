import App from '../../App.vue'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'
import { config } from '../fixtures'
import { state } from '../../store'
import { THEMES } from '../../themes'

const WS = '/Users/sandbox/Code'

/** The window in a theme, with a terminal blocked, one ready and one
 *  working, so the marks and the queue's cards show against its colours.
 *  The width bar's light and dark switch between the theme's palettes. */
async function inTheme(theme: string) {
  await boot({ config: config({ theme }), selected: 'main' })
  state.now = Date.now()
  backend.handle({ terminal_question: () => 'Should the retry move into the client, or stay in the job?' })
  terminals([
    { path: `${WS}/app-b`, status: 'idle', ready: true, minutesAgo: 30 },
    { path: `${WS}/app-1234-fix-header`, status: 'blocked', message: 'Run pnpm test', minutesAgo: 12 },
    { path: `${WS}/app-billing`, status: 'working', message: 'Running the invoice specs', minutesAgo: 3 },
  ])
}

export default defineGroup(
  'Themes',
  THEMES.map((t) => ({
    id: t.id,
    title: t.name,
    component: App,
    setup: () => inTheme(t.id),
  })),
)
