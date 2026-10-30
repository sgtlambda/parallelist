import { computed, shallowRef, watch } from 'vue'
import { entryOf, type Entry } from './palette'
import { selectedTerminal } from './store'
import { enterTerminal } from './terminal'
import { running, statuses } from './terminalState'

/**
 * The attention queue: every terminal waiting for the user, blocked on an
 * answer or ready with a turn that ended and got no reply yet, in the order
 * they started waiting. The oldest is first, and is where ⌘↵ goes.
 *
 * First in, first out, and by nothing else: unlike the palette, being
 * blocked doesn't jump the queue. A card stays where it is for as long as
 * its terminal goes on waiting (from blocked to ready is the same wait), and
 * a terminal that starts waiting again joins at the back. The backend keeps
 * when each wait began (`waitingSince`), so a reload keeps the order.
 *
 * Behind the queue, the terminals that are working: not waiting for the
 * user, so never where ⌘↵ goes, but what will join the queue next. They
 * are in the order they started working, so one finishing moves from the
 * head of that line to the back of the queue, right beside it.
 *
 * The terminal on screen keeps its place for as long as it is on screen, even
 * once it has had its reply and is working again, so a card doesn't leave
 * under the user mid-answer. Going elsewhere lets go of it: a terminal still
 * waiting takes its real place, and any other leaves the queue.
 *
 * Each card is on the branch the list has it on, found through the same
 * placement (`branchesOf`), so there is always a row to go to. A terminal
 * of a project no longer in the config has none, and no card either.
 */

export interface QueueItem {
  path: string
  /** `answered`: on screen, kept in its place, with nothing waiting any more. */
  kind: 'blocked' | 'ready' | 'working' | 'answered'
  /** When it started waiting, or working. */
  since: number
  /** What the line is ordered by: `since`, unless kept in its place. */
  place: number
  /** What the program says it is doing, while working or blocked. */
  message: string | null
  /** Its branch, as the branch list has it. */
  entry: Entry
  /** The terminal on screen. Still listed (it waits there until typed
   *  into), but already where ⌘↵ would go, so ⌘↵ goes past it. */
  here: boolean
}

function byAge(a: QueueItem, b: QueueItem) {
  return a.place - b.place || a.path.localeCompare(b.path)
}

/** The terminal on screen, if it has waited since it came on screen, and the
 *  place in the queue that wait gave it. */
const kept = shallowRef<{ path: string; since: number } | null>(null)

watch(
  [selectedTerminal, () => (selectedTerminal.value && statuses.get(selectedTerminal.value)?.waitingSince) || null],
  ([path, since]) => {
    if (kept.value?.path !== path) kept.value = null
    if (path && since && !kept.value) kept.value = { path, since }
  },
  { immediate: true },
)

const items = computed(() => {
  const here = selectedTerminal.value
  const waiting: QueueItem[] = []
  const working: QueueItem[] = []
  for (const path of running.keys()) {
    const s = statuses.get(path)
    const entry = entryOf.value.get(path)
    const place = kept.value?.path === path && path === here ? kept.value.since : null
    if (!entry || (!s && !place)) continue
    const item = { path, message: s?.message ?? null, entry, here: path === here }
    const workingSince = s?.status === 'working' ? s.workingSince : null
    if (s?.waitingSince) {
      // Blocked outweighs ready, as in the terminal mark.
      const kind = s.status === 'blocked' ? 'blocked' : 'ready'
      waiting.push({ ...item, kind, since: s.waitingSince, place: place ?? s.waitingSince })
    } else if (place) {
      const since = workingSince ?? place
      waiting.push({ ...item, kind: workingSince ? 'working' : 'answered', since, place })
    } else if (workingSince) {
      working.push({ ...item, kind: 'working', since: workingSince, place: workingSince })
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
  enterTerminal(item.entry.projectId, item.entry.branch.name, item.path)
}

/** ⌘↵. */
export function goToNextWaiting() {
  if (nextWaiting.value) goToItem(nextWaiting.value)
}
