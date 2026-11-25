import { nextTick } from 'vue'
import CommandPalette from '../../components/CommandPalette.vue'
import type { ScreenFound } from '../../api'
import { paletteOpen } from '../../palette'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'
import { config } from '../fixtures'

const WS = '/Users/sandbox/Code'
const header = `${WS}/app-1234-fix-header`
const billing = `${WS}/app-billing`
const search = `${WS}/app-search`

/** Open the palette and type `query`, as the user would. */
async function open(query: string, found: (query: string) => ScreenFound['hits'], screenSearch = true) {
  await boot({ config: config({ screenSearch }) })
  backend.handle({
    screen_search: ({ query }: { query: string }): ScreenFound => ({
      query: query.trim(),
      status: { state: 'ready', model: 'minilm-l6' },
      hits: found(query),
    }),
  })
  terminals([
    { path: header, status: 'working', message: 'Running the tests' },
    { path: billing, status: 'idle', ready: true },
    { path: search, status: 'idle' },
  ])
  paletteOpen.value = true
  await nextTick()
  setTimeout(() => {
    const input = document.querySelector<HTMLInputElement>('.palette-search input')
    if (!input) return
    input.value = query
    input.dispatchEvent(new Event('input'))
  }, 100)
}

export default defineGroup('Palette', [
  {
    id: 'screens',
    title: 'Found by what terminals show',
    note: 'No branch is called anything like "stripe webhook"; two terminals show it, one literally and one by meaning. Each row says what matched.',
    component: CommandPalette,
    setup: () =>
      open('stripe webhook', () => [
        {
          path: billing,
          relevance: 1,
          snippet: 'The controller calls Webhook::constructEvent before checking the Stripe-Signature header · Should I return a 400 early instead?',
        },
        {
          path: search,
          relevance: 0.42,
          snippet: 'POST /payments/events 500 in 31ms · signature verification failed for event evt_1Q…',
        },
      ]),
  },
  {
    id: 'mixed',
    title: 'Names and screens together',
    note: '"header" matches a branch name and what another terminal shows; the name match keeps its highlight.',
    component: CommandPalette,
    setup: () =>
      open('header', () => [
        { path: search, relevance: 1, snippet: '5:12:01 PM [vite] hmr update /src/components/Header.vue' },
      ]),
  },
  {
    id: 'off',
    title: 'Screen search off',
    note: 'As before: branch names only, and the backend is never asked.',
    component: CommandPalette,
    setup: () =>
      open('stripe webhook', () => {
        throw new Error('screen_search must not be called while off')
      }, false),
  },
])
