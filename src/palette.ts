import { computed, reactive, ref } from 'vue'
import { api, type BranchInfo, type Overview, type Workspace } from './api'
import { basename } from './format'
import { fuzzyMatch } from './fuzzy'
import { branchesOf, knownOverview, projectId, state } from './store'
import { normalizePath } from './tasks'
import { byTerminalRank, terminalRank, TIER, type TerminalRank, type Tier } from './terminalState'

/**
 * The command palette: every branch of every project, fuzzy-searched.
 *
 * Projects other than the one on screen are read from whatever is already
 * held — the refresh cache — and brought up to date with a peek each: local
 * refs and the workspace list, no fetch and no GitHub, ~0.3s. Nothing here
 * has pull requests, so nothing here searches them.
 */

export const paletteOpen = ref(false)

/**
 * Added to the match score, which runs at roughly a point per well-placed
 * query character. So a bonus decides between comparable matches rather than
 * overruling a better one, and weighs more the shorter the query: two letters
 * lean on context, a whole branch name hardly does.
 *
 * An active terminal outweighs the current project: it is where work is
 * actually going on, in whichever project. One waiting for the user weighs
 * more again — blocked on an answer most of all — so the palette doubles as
 * "go to whatever needs me next". These are the tiers of `terminalRank`,
 * which the branch list sorts by too. With no query the bonuses are the
 * whole score, so it opens on blocked terminals, then ready ones, then the
 * rest of the running ones, then the current project, then everything else
 * by commit. Each tier outweighs the current project on its own, so a
 * blocked terminal elsewhere still beats a ready one here.
 *
 * Within a tier, the terminal active most recently comes first; that is the
 * tie-break after the score, so with a query it never beats a better match.
 * The current project's bonus is left off a branch with a running terminal,
 * or it would split each tier by project before recency had a say.
 */
const CURRENT_PROJECT_BONUS = 1
const TERMINAL_BONUS: Record<Tier, number> = {
  [TIER.none]: 0,
  [TIER.running]: 2,
  [TIER.ready]: 3.25,
  [TIER.blocked]: 4.5,
}

/** A query word found in the project name or a workspace directory, rather
 *  than in the branch name itself, counts for less. */
const SECONDARY_WEIGHT = 0.75

/** A peek younger than this is not repeated on reopening. */
const FRESH_MS = 30_000

export interface Entry {
  projectId: string
  projectName: string
  branch: BranchInfo
  workspaces: Workspace[]
  current: boolean
}

export interface Result {
  entry: Entry
  rank: TerminalRank
  score: number
  /** Matched characters, for highlighting. */
  branchHits: number[]
  projectHits: number[]
}

const peeked = reactive(new Map<string, { overview: Overview; at: number }>())
const peeking = reactive(new Set<string>())

export const paletteLoading = computed(() => peeking.size > 0)

/** The newer of what the store holds and what the palette peeked. */
function latest(id: string) {
  const known = knownOverview(id)
  const own = peeked.get(id)
  if (!known) return own ?? null
  if (!own) return known
  return own.at > known.at ? own : known
}

/** Peek every project not seen recently. What is already held shows at once
 *  and each project's list is replaced as its peek lands. */
export function refreshPalette() {
  for (const p of state.config?.projects ?? []) {
    if (peeking.has(p.id)) continue
    const known = latest(p.id)
    if (known && Date.now() - known.at < FRESH_MS) continue
    peeking.add(p.id)
    api
      .peek(p.id)
      .then((overview) => void peeked.set(p.id, { overview, at: Date.now() }))
      // Gone from disk, or no longer a repository: it lists what it last had.
      .catch(() => {})
      .finally(() => peeking.delete(p.id))
  }
}

/** Every branch of every project, placed as the branch list places them
 *  (`branchesOf`): for the project on screen, from the very overview the
 *  list shows. A project not looked at yet still has its terminals' rows. */
const entries = computed<Entry[]>(() => {
  const out: Entry[] = []
  for (const p of state.config?.projects ?? []) {
    for (const placed of branchesOf(p.id, latest(p.id)?.overview ?? null)) {
      out.push({
        projectId: p.id,
        projectName: p.name,
        branch: placed.branch,
        // A removed workspace's terminal can still be waiting on the user.
        workspaces: [...placed.workspaces, ...placed.unlisted],
        current: p.id === projectId.value,
      })
    }
  }
  return out
})

/** The branch each workspace is on, by normalised path: where the attention
 *  queue finds a terminal's row, so it finds the one the list shows. */
export const entryOf = computed(() => {
  const byPath = new Map<string, Entry>()
  for (const entry of entries.value) {
    for (const w of entry.workspaces) byPath.set(normalizePath(w.path), entry)
  }
  return byPath
})

function weighted(score: number): number {
  return score > 0 ? score * SECONDARY_WEIGHT : score
}

/**
 * The best `limit` branches for `query`. Each space-separated word must
 * match the branch name, the project name or a workspace directory, so
 * `app header` narrows to project `app`. Ties go to the terminal active
 * most recently, then the newer commit.
 */
export function searchPalette(query: string, limit: number): Result[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  // The same for every branch of a project, so worked out once per search.
  const projectMatches = new Map<string, ReturnType<typeof fuzzyMatch>>()

  const results: Result[] = []
  for (const entry of entries.value) {
    // Here rather than in `entries`, which is cached: activity is read as
    // it stands, so it has to be read on every search.
    const rank = terminalRank(entry.workspaces.map((w) => w.path))
    let score =
      rank.tier !== TIER.none
        ? TERMINAL_BONUS[rank.tier]
        : entry.current
          ? CURRENT_PROJECT_BONUS
          : 0
    const branchHits: number[] = []
    const projectHits: number[] = []
    let matched = true

    for (const word of words) {
      const inBranch = fuzzyMatch(word, entry.branch.name)
      const key = `${entry.projectId}\0${word}`
      if (!projectMatches.has(key)) projectMatches.set(key, fuzzyMatch(word, entry.projectName))
      const inProject = projectMatches.get(key)!
      let inFolder: number | null = null
      for (const w of entry.workspaces) {
        const m = fuzzyMatch(word, basename(w.path))
        if (m && (inFolder === null || m.score > inFolder)) inFolder = m.score
      }

      const candidates = [
        inBranch && { score: inBranch.score, hits: branchHits, positions: inBranch.positions },
        inProject && {
          score: weighted(inProject.score),
          hits: projectHits,
          positions: inProject.positions,
        },
        inFolder !== null && { score: weighted(inFolder), hits: null, positions: [] },
      ].filter((c) => !!c)
      if (!candidates.length) {
        matched = false
        break
      }
      const best = candidates.reduce((a, b) => (b.score > a.score ? b : a))
      score += best.score
      best.hits?.push(...best.positions)
    }

    if (matched) results.push({ entry, rank, score, branchHits, projectHits })
  }

  results.sort(
    (a, b) =>
      b.score - a.score ||
      byTerminalRank(a.rank, b.rank) ||
      b.entry.branch.committedAt - a.entry.branch.committedAt,
  )
  return results.slice(0, limit)
}
