import ConfirmQuitDialog from '../../components/ConfirmQuitDialog.vue'
import { backend } from '../backend'
import { defineGroup } from '../define'

const setup = () => backend.handle({ quit_app: () => undefined })

export default defineGroup('Quit dialog', [
  {
    id: 'keep',
    title: 'Terminals in a relay',
    note: 'The focused button keeps them running for the next launch; ending them is a click of its own.',
    component: ConfirmQuitDialog,
    props: { ask: { running: 5, waiting: 2, persistent: true } },
    setup,
  },
  {
    id: 'keep-one',
    title: 'One terminal, nothing waiting',
    component: ConfirmQuitDialog,
    props: { ask: { running: 1, waiting: 0, persistent: true } },
    setup,
  },
  {
    id: 'no-relay',
    title: 'No relay',
    note: 'The terminals live in the app (no relay could be started, or an older one was left running), so quitting can only end them.',
    component: ConfirmQuitDialog,
    props: { ask: { running: 3, waiting: 1, persistent: false } },
    setup,
  },
])
