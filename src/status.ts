/**
 * Every badge that describes what a workspace holds, in one place.
 *
 * The branch list, the workspace card and Cleanup each used to spell these
 * out for themselves, and they drifted: the list called unpushed commits
 * "uncommitted" while the card beside it said "unpushed". Anything shown about
 * a workspace's state is built here, and the numbers behind it come from
 * `status.rs`, which says exactly what each one counts.
 */
import type { RemovalImpact, Workspace } from './api'
import { basename } from './format'

export interface Badge {
  /** Stable, for `v-for` keys. */
  key: string
  text: string
  /** `warn` is work that removal would lose; plain is information. */
  warn: boolean
  title: string
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`
}

/** Not asked yet, as opposed to unknown: the list is showing `api.peek`
 *  while the refresh that measures it runs. */
function unchecked(w: Workspace): boolean {
  return w.kind !== 'orphan' && !w.status && !w.statusError
}

/** Something here would be lost, or we couldn't tell. Untracked files and
 *  being behind don't count: neither is work you can lose by removing it.
 *  Not having asked yet counts as couldn't tell. */
export function atRisk(w: Workspace): boolean {
  if (w.kind === 'orphan') return false
  if (!w.status) return true
  return w.status.modified > 0 || w.status.unpushed > 0
}

/** The badges on one workspace's card. */
export function workspaceBadges(w: Workspace): Badge[] {
  const s = w.status
  if (unchecked(w)) {
    return [{ key: 'checking', text: 'checking…', warn: false, title: 'Reading its working tree' }]
  }
  if (!s) {
    return w.statusError
      ? [
          {
            key: 'unknown',
            text: 'status unknown',
            warn: true,
            title: `Couldn't read this workspace's state, so it may hold work: ${w.statusError}`,
          },
        ]
      : []
  }
  const badges: Badge[] = []
  if (s.modified) {
    badges.push({
      key: 'modified',
      text: `${s.modified} changed`,
      warn: true,
      title: `${plural(s.modified, 'tracked file')} with uncommitted changes, staged or not`,
    })
  }
  if (s.untracked) {
    badges.push({
      key: 'untracked',
      text: `${s.untracked} untracked`,
      warn: false,
      title: 'Untracked files — not counted as changes',
    })
  }
  if (s.unpushed) {
    badges.push({
      key: 'unpushed',
      text: `${s.unpushed} unpushed`,
      warn: true,
      title: `${plural(s.unpushed, 'commit')} that origin has in no form — not on any branch there, not as a rebased copy, not squashed into the base branch`,
    })
  }
  if (s.behind) {
    badges.push({
      key: 'behind',
      text: `${s.behind} behind`,
      warn: false,
      title: `Origin has ${plural(s.behind, 'commit')} on this branch that this workspace doesn't`,
    })
  }
  return badges
}

/**
 * The badges on a branch row: one per concern, whichever of its workspaces
 * it comes from. The tooltip names them, since a branch can sit in several.
 */
export function rowBadges(workspaces: Workspace[]): Badge[] {
  const named = (list: Workspace[], detail: (w: Workspace) => string) =>
    list.map((w) => `${basename(w.path)}: ${detail(w)}`).join('\n')

  const changed = workspaces.filter((w) => w.status?.modified)
  const unpushed = workspaces.filter((w) => w.status?.unpushed)
  // Not yet asked is left out: every row would flash "status unknown" for
  // the few seconds until the first refresh lands.
  const unknown = workspaces.filter((w) => w.kind !== 'orphan' && !w.status && !unchecked(w))

  const badges: Badge[] = []
  if (changed.length) {
    badges.push({
      key: 'modified',
      text: 'uncommitted',
      warn: true,
      title: named(changed, (w) => plural(w.status!.modified, 'changed file')),
    })
  }
  if (unpushed.length) {
    badges.push({
      key: 'unpushed',
      text: 'unpushed',
      warn: true,
      title: named(unpushed, (w) => `${plural(w.status!.unpushed, 'commit')} origin doesn't have`),
    })
  }
  if (unknown.length) {
    badges.push({
      key: 'unknown',
      text: 'status unknown',
      warn: true,
      title: named(unknown, (w) => w.statusError ?? 'not readable'),
    })
  }
  return badges
}

/**
 * What one removal would destroy, given whether the branch goes with it.
 * Empty means nothing; `impactSummary` says so in words.
 */
export function impactBadges(i: RemovalImpact, deletingBranch: boolean): Badge[] {
  const badges: Badge[] = []
  if (i.error) {
    badges.push({
      key: 'unknown',
      text: "couldn't check",
      warn: true,
      title: `Some of this workspace couldn't be measured, so it may hold work: ${i.error}`,
    })
  }
  if (i.status?.modified) {
    badges.push({
      key: 'modified',
      text: `${i.status.modified} changed`,
      warn: true,
      title: `${plural(i.status.modified, 'tracked file')} with uncommitted changes`,
    })
  }
  if (commitsLost(i, deletingBranch)) {
    badges.push({
      key: 'commits',
      text: plural(i.commits, 'commit'),
      warn: true,
      title:
        i.kind === 'clone'
          ? 'Commits on any of this clone\'s branches that origin has in no form'
          : 'Commits origin has in no form, held by no other branch',
    })
  }
  if (i.stashes) {
    badges.push({ key: 'stashes', text: `${i.stashes} stashed`, warn: true, title: 'Stashes kept in this clone' })
  }
  if (i.dependents.length) {
    badges.push({
      key: 'dependents',
      text: `${plural(i.dependents.length, 'worktree')} break`,
      warn: true,
      title: `Registered against this clone, and unusable without it:\n${i.dependents.join('\n')}`,
    })
  }
  return badges
}

/** Commits this removal destroys: a worktree's branch keeps its own unless
 *  that branch is being deleted too. */
export function commitsLost(i: RemovalImpact, deletingBranch: boolean): number {
  return i.commitsNeedBranchDelete && !deletingBranch ? 0 : i.commits
}

/** The words shown when `impactBadges` is empty. */
export function impactSummary(i: RemovalImpact, deletingBranch: boolean): string {
  if (i.kind === 'orphan') return 'not a git workspace'
  const kept = i.commitsNeedBranchDelete && !deletingBranch && i.commits
  const parts = ['nothing']
  if (i.status?.untracked) parts.push(`${i.status.untracked} untracked`)
  if (kept) parts.push(`${plural(i.commits, 'commit')} kept on the branch`)
  return parts.length > 1 ? `${parts[0]} (${parts.slice(1).join(', ')})` : parts[0]
}
