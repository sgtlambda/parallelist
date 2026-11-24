import OutdatedRelayDialog from '../../components/OutdatedRelayDialog.vue'
import { backend } from '../backend'
import { defineGroup } from '../define'

export default defineGroup('Outdated relay dialog', [
  {
    id: 'launch',
    title: 'At launch',
    note: "An older version's relay holds this build's terminals. Ending them restarts the app; the call is only logged here.",
    component: OutdatedRelayDialog,
    props: { build: 'main' },
    setup: () => backend.handle({ replace_outdated_relay: () => undefined }),
  },
])
