import DockIcon from '../views/DockIcon.vue'
import { defineGroup } from '../define'

export default defineGroup('Dock icon', [
  {
    id: 'working',
    title: 'Something working: the cursor blinks',
    note: 'On and off every 1.1 s, as the terminal glyph blinks in the app, while anything is working.',
    component: DockIcon,
    props: { working: true },
  },
  {
    id: 'idle',
    title: 'Nothing working',
    note: 'The icon as it ships, cursor steady.',
    component: DockIcon,
    props: { working: false },
  },
  {
    id: 'working-waiting',
    title: 'Working, with two waiting',
    note: 'The blink next to the red badge, to see whether the two compete.',
    component: DockIcon,
    props: { working: true, waiting: 2 },
  },
])
