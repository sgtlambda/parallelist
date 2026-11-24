import FirstRunWindow from '../views/FirstRunWindow.vue'
import FirstRunCard from '../firstRun/FirstRunCard.vue'
import { defineGroup } from '../define'
import { BARE, TYPICAL, firstRun } from '../suggestions'

/**
 * The concepts for the first-run choice of "Open in" targets that were not
 * taken; the one taken, tiles, is the onboarding's first step and has its
 * scenarios there. Both run `src/firstRun.ts`; only the way it is shown
 * differs.
 */

export default defineGroup('First run', [
  {
    id: 'checklist',
    title: '1. Checklist: a dialog of switches',
    note: 'A dialog over the onboarding. Found programs grouped, the agents on.',
    component: FirstRunWindow,
    props: { concept: 'checklist' },
    setup: () => firstRun(TYPICAL),
  },
  {
    id: 'checklist-bare',
    title: '1. Checklist, on a Mac with little installed',
    note: 'Only Terminal and Finder.',
    component: FirstRunWindow,
    props: { concept: 'checklist' },
    setup: () => firstRun(BARE),
  },
  {
    id: 'checklist-loading',
    title: '1. Checklist, while it looks',
    note: 'suggest_targets can take a second.',
    component: FirstRunWindow,
    props: { concept: 'checklist' },
    setup: () => firstRun(TYPICAL, { loading: true }),
  },
  {
    id: 'card',
    title: '3. On the card: no wizard',
    note: 'Found programs as dashed buttons on the cards. Clicking one opens the workspace in it and keeps it.',
    component: FirstRunCard,
    layout: 'box',
    size: { width: 420, height: 520 },
    setup: () => firstRun(TYPICAL, { projects: true }),
  },
  {
    id: 'card-bare',
    title: '3. On the card, on a Mac with little installed',
    component: FirstRunCard,
    layout: 'box',
    size: { width: 420, height: 520 },
    setup: () => firstRun(BARE, { projects: true }),
  },
])
