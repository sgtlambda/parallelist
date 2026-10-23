import { computed, reactive } from 'vue'
import type { TerminalInfo } from './api'
import { normalizePath } from './tasks'

/**
 * What is known about the terminals, apart from the xterm sessions that draw
 * them (`src/terminal.ts`): which are running, what their programs report,
 * when each was last active, and what all that makes their marks and their
 * place in the lists. Nothing here needs the store, so the store can sort by
 * it without an import cycle.
 */

/** Where a terminal belongs (`TerminalInfo`): its project, and the branch
 *  and kind of workspace it was opened on. */
export type Home = Pick<TerminalInfo, 'projectId' | 'branch' | 'kind'>

/**
 * Every running shell, by workspace path, with its home: what the project
 * list, branch list, cards and attention queue mark. One map, so a terminal
 * can't be running without somewhere to be listed (`branchesOf`), and the
 * sidebar can mark a project without knowing its workspaces.
 */
export const running = reactive(new Map<string, Home>())

export function homeOf(t: TerminalInfo): Home {
  return { projectId: t.projectId, branch: t.branch, kind: t.kind }
}

/** What the program in each terminal reports over OSC 7501, when it does,
 *  whether a turn ended there unseen (`ready`, kept by the backend), since
 *  when either has kept it waiting for the user, and since when it has been
 *  working (when the status turned `working`, or after a reload, when it
 *  was last active). */
export const statuses = reactive(
  new Map<
    string,
    Pick<TerminalInfo, 'status' | 'ready' | 'waitingSince' | 'message'> & {
      workingSince: number | null
    }
  >(),
)

/**
 * When each terminal was last active, in epoch milliseconds: started, its
 * program's status changed, or typed into (`TerminalInfo.activeAt`).
 * Deliberately not reactive. It changes with every keystroke, and nothing
 * should re-sort on that: the lists read it whenever they re-sort for some
 * other reason, so the branch list doesn't shuffle rows under the pointer.
 */
export const activeAt = new Map<string, number>()

export function setStatus(t: TerminalInfo) {
  const key = normalizePath(t.path)
  if (t.status || t.ready) {
    const was = statuses.get(key)
    const workingSince =
      t.status !== 'working'
        ? null
        : was?.status === 'working'
          ? was.workingSince
          : t.activeAt || Date.now()
    statuses.set(key, {
      status: t.status,
      ready: t.ready,
      waitingSince: t.waitingSince,
      message: t.message,
      workingSince,
    })
  }
  else statuses.delete(key)
  if (t.activeAt) activeAt.set(key, t.activeAt)
}

export function terminalActive(path: string): boolean {
  return running.has(normalizePath(path))
}

export interface TerminalMark {
  /** Blinking while Claude Code, or anything speaking OSC 7501, works. */
  icon: 'terminal' | 'terminal-working'
  /** Something waits for the user: `blocked` on an answer (orange), or
   *  `ready`, a turn finished out of sight (blue). */
  box: 'blocked' | 'ready' | null
  title: string
}

/**
 * The mark for the terminals of some workspaces: none unless one has a
 * running shell. Several terminals combine part by part: the icon blinks if
 * any of them works, and the box is orange if any is blocked, else blue if
 * any is ready. The one place it is decided, for the project list, the
 * branch rows, the cards and the palette alike.
 */
export function terminalMark(paths: string[]): TerminalMark | null {
  const keys = paths.map(normalizePath).filter((p) => running.has(p))
  if (!keys.length) return null
  const reported = keys.map((k) => statuses.get(k))
  const working = reported.some((r) => r?.status === 'working')
  const blocked = reported.some((r) => r?.status === 'blocked')
  const ready = reported.some((r) => r?.ready)
  const title = [
    blocked && 'Blocked — waiting for your answer',
    ready && 'Ready — finished while you were away',
    working && 'Working',
  ].filter((t) => !!t)
  return {
    icon: working ? 'terminal-working' : 'terminal',
    box: blocked ? 'blocked' : ready ? 'ready' : null,
    title: title.join('\n') || 'Active terminal',
  }
}

export function projectTerminalMark(projectId: string): TerminalMark | null {
  const paths = [...running].filter(([, home]) => home.projectId === projectId)
  return terminalMark(paths.map(([path]) => path))
}

/** How much a terminal asks of the user, in the same order as its mark's
 *  box: blocked on an answer, then ready, then merely running. */
export const TIER = { none: 0, running: 1, ready: 2, blocked: 3 } as const
export type Tier = (typeof TIER)[keyof typeof TIER]

/**
 * Each running terminal's tier. The same Map comes back for as long as no
 * tier changes, so whatever sorts by it re-sorts when a terminal starts,
 * stops or comes to need the user, and not on every status in between
 * (`working` and `idle` alternate all through a Claude Code session).
 */
const tiers = computed<Map<string, Tier>>((previous) => {
  const next = new Map<string, Tier>()
  for (const path of running.keys()) {
    const s = statuses.get(path)
    next.set(path, s?.status === 'blocked' ? TIER.blocked : s?.ready ? TIER.ready : TIER.running)
  }
  const same =
    previous?.size === next.size && [...next].every(([path, tier]) => previous.get(path) === tier)
  return same ? previous : next
})

/** Where some workspaces' terminals put their branch, for sorting. */
export interface TerminalRank {
  /** The most any of them asks of the user. */
  tier: Tier
  /** When the most recently active of them was, or 0 with none running. */
  at: number
}

/**
 * The rank of a branch by its workspaces' terminals: the one ordering the
 * palette and the branch list share. Combined like `terminalMark`. Reactive
 * in the tier only; `at` is read as it stands (see `activeAt`).
 */
export function terminalRank(paths: string[]): TerminalRank {
  let tier: Tier = TIER.none
  let at = 0
  for (const path of paths) {
    const key = normalizePath(path)
    const t = tiers.value.get(key)
    if (!t) continue
    if (t > tier) tier = t
    at = Math.max(at, activeAt.get(key) ?? 0)
  }
  return { tier, at }
}

/** Highest tier first, then the most recently active. Branches without a
 *  running terminal compare equal, so a stable sort leaves them be. */
export function byTerminalRank(a: TerminalRank, b: TerminalRank): number {
  return b.tier - a.tier || b.at - a.at
}
