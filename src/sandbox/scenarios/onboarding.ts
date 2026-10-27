import Window from '../views/Window.vue'
import { boot } from '../seed'
import { defineGroup } from '../define'
import { config } from '../fixtures'
import { backend } from '../backend'
import { state } from '../../store'

/** The app with no project added, as on a first launch. */
const firstLaunch = () => boot({ config: config({ projects: [], selectedProjectId: null }) })

export default defineGroup('Onboarding', [
  {
    id: 'first-launch',
    title: 'No projects',
    note: 'Replaces the branch list and the detail pane, which have nothing to show; the sidebar and the queue stay. Try narrow widths.',
    component: Window,
    props: { show: 'onboarding' },
    setup: firstLaunch,
  },
  {
    id: 'rejected',
    title: 'A folder git rejected',
    note: 'The error lands in state.error. The branch list would have shown it, and is not there, so the screen carries its own banner.',
    component: Window,
    props: { show: 'onboarding' },
    async setup() {
      await firstLaunch()
      state.error = 'fatal: not a git repository (or any of the parent directories): .git'
    },
  },
  {
    id: 'folder-over',
    title: 'A folder dragged over',
    note: 'A drag from Finder is over the window. The drop goes to Tauri, never the page, so the overlay takes no pointer events.',
    component: Window,
    props: { show: 'onboarding' },
    async setup() {
      await firstLaunch()
      backend.handle({ are_folders: ({ paths }: { paths: string[] }) => paths.map(() => true) })
      // Setup runs before the mount, and the overlay listens once mounted.
      setTimeout(() => {
        backend.emit('drag-drop', { type: 'enter', paths: ['/Users/sandbox/Code/app'], position: { x: 0, y: 0 } })
      }, 100)
    },
  },
])
