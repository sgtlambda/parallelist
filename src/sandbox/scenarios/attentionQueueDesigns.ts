import Window from '../views/Window.vue'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'
import { branch, overview, workspace } from '../fixtures'
import { state } from '../../store'
import { normalizePath } from '../../tasks'
import { showTerminal } from '../../terminal'
import '../aqDesigns.css'

/**
 * Other looks for the attention queue, each a class on the window's grid
 * that `aqDesigns.css` keys on, after the chosen one, over the same strip:
 * two ready, one blocked, one of them on screen, then two working past the
 * divider. First two other ways of parting neighbours on the chosen look,
 * then the looks it was chosen over. The component is the one the app runs;
 * only the stylesheet differs.
 */

const WS = '/Users/sandbox/Code'
const header = `${WS}/app-1234-fix-header`
const sso = `${WS}/app-b`
const billing = `${WS}/app-billing`
const search = `${WS}/app-search`
const audit = `${WS}/app-audit-log`

async function sameQueue() {
  const base = overview()
  await boot({
    overview: {
      ...base,
      branches: [...base.branches, branch('feature/audit-log', { hasLocal: true })],
      workspaces: [...base.workspaces, workspace('app-audit-log', 'worktree', 'feature/audit-log')],
    },
    selected: 'fix/billing-rounding',
  })
  state.now = Date.now()
  const asked = new Map(
    Object.entries({
      [sso]: 'Should the SAML assertion be required to be signed, or only the response?',
      [billing]: 'Rounded half-even in the invoice model and updated the spec.',
    }).map(([path, q]) => [normalizePath(path), q]),
  )
  backend.handle({ terminal_question: ({ path }: { path: string }) => asked.get(path) ?? null })
  terminals([
    { path: sso, status: 'idle', ready: true, minutesAgo: 41 },
    { path: header, status: 'blocked', message: 'Run bundle exec rspec spec/models', minutesAgo: 12 },
    { path: billing, status: 'idle', ready: true, minutesAgo: 2 },
    { path: audit, status: 'working', message: 'Run the audit log specs', minutesAgo: 9 },
    { path: search, status: 'working', message: 'Reindexing fixtures', minutesAgo: 3 },
  ])
  showTerminal('app', 'fix/billing-rounding', billing)
}

const design = (name: string) => ({ show: 'queue', design: name })

export default defineGroup('Attention queue designs', [
  {
    id: 'current',
    title: '0. Current: a wash of the state colour',
    note: 'The chosen look. Borderless tiles: blocked sits in a wash of orange, ready in blue; working and the one on screen have no fill, so only what needs you has colour behind it. Two bare cards in a row are parted by a hairline in the gap.',
    component: Window,
    props: { show: 'queue' },
    setup: sameQueue,
  },
  {
    id: 'tile',
    title: '1. Tile: a fill for every card',
    note: 'On the chosen look, in place of its hairline. Working ones and the one on screen get a neutral tile, a step off the strip, so every card is a block and the gap is the strip showing through. The wash still says which need you.',
    component: Window,
    props: design('tile'),
    setup: sameQueue,
  },
  {
    id: 'hollow',
    title: '2. Hollow: an outline for the bare cards',
    note: 'On the chosen look, in place of its hairline. Working ones and the one on screen are outlined by a hairline and left unfilled; waiting ones keep their wash and no outline. Filled is waiting, hollow is not.',
    component: Window,
    props: design('hollow'),
    setup: sameQueue,
  },
  {
    id: 'plain',
    title: '3. Plain: text on the strip',
    note: 'No border and no fill. Each card is its two lines set apart by spacing, the mark alone saying its state. Hover lifts it on the hover colour.',
    component: Window,
    props: design('plain'),
    setup: sameQueue,
  },
  {
    id: 'edge',
    title: '4. Edge: a bar down the left',
    note: 'Flat, square, led by a 3px bar in the state colour (grey for working, dotted where you already are). Reads like a list of messages.',
    component: Window,
    props: design('edge'),
    setup: sameQueue,
  },
  {
    id: 'line',
    title: '5. Line: one line per terminal',
    note: 'Mark, directory, what it said, age, ⌘↵ on one 28px chip, so twice as many fit. Filled, no border; working ones are bare. The strip could be made shorter for this one.',
    component: Window,
    props: design('line'),
    setup: sameQueue,
  },
  {
    id: 'segment',
    title: '6. Segment: one shape around the queue',
    note: 'The waiting cards are one raised segment with hairlines where they meet, like a segmented control: one border for the queue, not one per card. Working ones past the divider are plain text.',
    component: Window,
    props: design('segment'),
    setup: sameQueue,
  },
])
