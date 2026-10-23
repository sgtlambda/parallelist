import Window from '../views/Window.vue'
import { boot } from '../seed'
import { defineGroup } from '../define'
import { config } from '../fixtures'
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
])
