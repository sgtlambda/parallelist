import { computed } from 'vue'
import { entryOf, type Entry } from './palette'
import { goToBranch, projectId, state } from './store'
import { showTerminal, terminalFor } from './terminal'
import { running, statuses } from './terminalState'

/**
 * The attention queue: every terminal waiting for the user, blocked on an
 * answer or ready with a turn that ended unseen, in the order they started
 * waiting. The oldest is first, and is where ⌘↵ goes.
 *
 * First in, first out, and by nothing else: unlike the palette, being
 * blocked doesn't jump the queue. A card stays where it is for as long as
 * its terminal goes on waiting (from blocked to ready is the same wait), and
 * a terminal that starts waiting again joins at the back. The backend keeps
 * when each wait began (`waitingSince`), so a reload keeps the order.
 *
 * Behind the queue, the terminals that are working: not waiting for the
 * user, so never where ⌘↵ goes, but what will join the queue next. They
 * are in the order they started working, so one finishing unseen moves
 * from the head of that line to the back of the queue, right beside it.
 *
 * Each card is on the branch the list has it on, found through the same
 * placement (`branchesOf`), so there is always a row to go to. A terminal
 * of a project no longer in the config has none, and no card either.
 */

export interface QueueItem {
  path: string
  kind: 'blocked' | 'ready' | 'working'
  /** When it started waiting, or working. */
  since: number
  /** What the program says it is doing, while working or blocked. */
  message: string | null
  /** Its branch, as the branch list has it. */
  entry: Entry
  /** The terminal on screen. Still listed (one blocked waits there until
   *  answered), but already where ⌘↵ would go, so ⌘↵ goes past it. */
  here: boolean
}

function byAge(a: QueueItem, b: QueueItem) {
  return a.since - b.since || a.path.localeCompare(b.path)
}

const items = computed(() => {
  const here = terminalFor(projectId.value, state.selectedBranch)
  const waiting: QueueItem[] = []
  const working: QueueItem[] = []
  for (const path of running.keys()) {
    const s = statuses.get(path)
    const entry = entryOf.value.get(path)
    if (!s || !entry) continue
    const item = { path, message: s.message, entry, here: path === here }
    if (s.waitingSince) {
      // Blocked outweighs ready, as in the terminal mark.
      const kind = s.status === 'blocked' ? 'blocked' : 'ready'
      waiting.push({ ...item, kind, since: s.waitingSince })
    } else if (s.status === 'working' && s.workingSince) {
      working.push({ ...item, kind: 'working', since: s.workingSince })
    }
  }
  return { waiting: waiting.sort(byAge), working: working.sort(byAge) }
})

/** Waiting for the user, oldest first. */
export const queue = computed(() => items.value.waiting)

/** Working, longest-running first. */
export const working = computed(() => items.value.working)

/** Where ⌘↵ goes: the terminal that has waited longest, other than the one
 *  on screen. */
export const nextWaiting = computed(() => queue.value.find((w) => !w.here) ?? null)

/** Show a terminal on its branch, and give it the keyboard. */
export function goToItem(item: QueueItem) {
  const { projectId, branch } = item.entry
  // Asked for outright, so a search field that had focus lets go of it, or
  // the terminal would leave the keyboard there (`mayTakeFocus`).
  const focused = document.activeElement
  if (focused instanceof HTMLElement && focused.closest('input, textarea')) focused.blur()
  goToBranch(projectId, branch.name)
  showTerminal(projectId, branch.name, item.path)
}

/** ⌘↵. */
export function goToNextWaiting() {
  if (nextWaiting.value) goToItem(nextWaiting.value)
}
