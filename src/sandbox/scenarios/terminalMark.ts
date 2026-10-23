import TerminalMark from '../../components/TerminalMark.vue'
import MarkGallery from '../views/MarkGallery.vue'
import { terminals } from '../seed'
import { defineGroup } from '../define'

const rows = [
  { label: 'No terminal: nothing drawn', path: '/w/none' },
  { label: 'A shell is running', path: '/w/running' },
  { label: 'Working: the glyph blinks', path: '/w/working' },
  { label: 'Blocked on an answer: orange box', path: '/w/blocked' },
  { label: 'Finished out of sight: blue box', path: '/w/ready' },
  { label: 'Blocked and working: orange wins, still blinks', path: '/w/both' },
  { label: 'Idle after a turn, seen', path: '/w/idle' },
]

export default defineGroup('TerminalMark', [
  {
    id: 'states',
    title: 'Every state',
    note: 'terminalMark is the one place this is decided; the list, cards, sidebar and palette all draw it.',
    component: MarkGallery,
    props: { rows, projectId: 'app' },
    setup: () =>
      terminals([
        { path: '/w/running' },
        { path: '/w/working', status: 'working' },
        { path: '/w/blocked', status: 'blocked' },
        { path: '/w/ready', status: 'idle', ready: true },
        { path: '/w/both', status: 'blocked', ready: true },
        { path: '/w/idle', status: 'idle' },
      ]),
  },
  {
    id: 'working',
    title: 'Working, large',
    component: TerminalMark,
    props: { paths: ['/w/working'], size: 48 },
    setup: () => terminals([{ path: '/w/working', status: 'working' }]),
  },
])
