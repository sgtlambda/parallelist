import DialogStage from '../views/DialogStage.vue'
import { backend } from '../backend'
import { boot } from '../seed'
import { defineGroup, type Scenario } from '../define'
import cleanup from './cleanupDialog'
import create from './createDialog'
import '../dialogDesigns.css'

/**
 * The dialogs and the popovers in them, in the chosen sheet and then in the
 * looks it was chosen over, each a class on the root element that
 * `dialogDesigns.css` keys on, over a stand-in for the window so a
 * translucent one has something to show through. The same six
 * states under each: the New workspace form, its branch picker open, the
 * quit question with and without a relay, Cleanup and the palette. The components are
 * the ones the app runs; only the stylesheet differs.
 */

const createSetup = create.scenarios[0].setup
const cleanupScenario = cleanup.scenarios[0]

const DESIGNS = [
  [
    'current',
    '0. Current: a sheet',
    'The chosen look. A macOS sheet: drops from under the title bar, the window dims a little, no rules, labels to the left of their fields, AppKit push buttons. The picker is a native menu: translucent, the keyboard row in the accent.',
  ],
  [
    'alert',
    '1. Alert & panel',
    'Floating in the middle in the translucent system material, nothing dimmed. Questions take the NSAlert shape: icon, centred text, stacked full-width buttons with the default first.',
  ],
  [
    'float',
    '2. Float',
    'A keyboard-first panel as Spotlight: thick material, large radius, every dialog high where the palette sits, filled fields, and a toolbar footer whose buttons show ↵ and esc. Menus are a tint, not the accent.',
  ],
] as const

const STATES: {
  id: string
  title: string
  props: Record<string, unknown>
  setup?: () => void | Promise<void>
}[] = [
  {
    id: 'create',
    title: 'New workspace',
    props: { dialog: 'create', open: true, initialBranch: 'feature/search-index' },
    setup: createSetup,
  },
  {
    id: 'picker',
    title: 'Branch picker open',
    props: { dialog: 'picker', open: true, initialBranch: '' },
    setup: createSetup,
  },
  {
    id: 'quit',
    title: 'Quit, with a relay',
    props: { dialog: 'quit', open: true },
    setup: () => backend.handle({ quit_app: () => undefined }),
  },
  {
    id: 'quit-alone',
    title: 'Quit, no relay',
    props: { dialog: 'quit-alone', open: true },
    setup: () => backend.handle({ quit_app: () => undefined }),
  },
  {
    id: 'cleanup',
    title: 'Cleanup',
    props: { dialog: 'cleanup', ...cleanupScenario.props },
    setup: cleanupScenario.setup,
  },
  {
    id: 'palette',
    title: 'Command palette',
    props: { dialog: 'palette' },
    setup: () => boot(),
  },
]

const scenarios: Scenario[] = DESIGNS.flatMap(([design, name, note]) =>
  STATES.map((state) => ({
    id: `${design}-${state.id}`,
    title: `${name} · ${state.title}`,
    note,
    component: DialogStage,
    props: { ...state.props, design: design === 'current' ? undefined : design },
    setup: state.setup,
  })),
)

export default defineGroup('Dialog designs', scenarios)
