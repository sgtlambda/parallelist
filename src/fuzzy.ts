/**
 * Fuzzy matching for the command palette: fzy's scoring
 * (https://github.com/jhawthorn/fzy, `ALGORITHM.md`).
 *
 * Every character of the query must appear in order. Among the ways it can,
 * the best one is found by dynamic programming: characters that land at the
 * start of a word (after `/`, `-`, `_`, `.`, a space, or a camelCase hump)
 * or right after the previous match score high, and every character skipped
 * costs a little. So `fh` finds `fix-header` well ahead of `refresh`, and
 * among equal matches the shorter name wins.
 *
 * Scores are roughly one point per well-placed query character, which is the
 * scale any bonus added on top should be thought of in. The whole text,
 * typed exactly, scores extra: otherwise `main` barely beats `maintenance/x`
 * and any bonus on the latter overturns it. (fzy makes it infinite, which
 * would leave no room for bonuses at all.)
 */

const GAP_LEADING = -0.005
const GAP_TRAILING = -0.005
const GAP_INNER = -0.01
const MATCH_CONSECUTIVE = 1.0
const MATCH_SLASH = 0.9
const MATCH_WORD = 0.8
const MATCH_CAPITAL = 0.7
const MATCH_DOT = 0.6
const EXACT = 1.5

/** Longer haystacks aren't worth the quadratic table. */
const MAX_LENGTH = 512

export interface Match {
  score: number
  /** Indices into the haystack of the characters that matched. */
  positions: number[]
}

function bonusAt(text: string, j: number): number {
  if (j === 0) return MATCH_SLASH
  const prev = text[j - 1]
  if (prev === '/') return MATCH_SLASH
  if (prev === '-' || prev === '_' || prev === ' ') return MATCH_WORD
  if (prev === '.') return MATCH_DOT
  const c = text[j]
  if (prev >= 'a' && prev <= 'z' && c >= 'A' && c <= 'Z') return MATCH_CAPITAL
  return 0
}

/** Whether `needle` occurs in `hay` as a subsequence. Both lowercased. Cheap,
 *  so it rules out most candidates before the table is built. */
function isSubsequence(needle: string, hay: string): boolean {
  let i = 0
  for (let j = 0; j < hay.length && i < needle.length; j++) {
    if (hay[j] === needle[i]) i++
  }
  return i === needle.length
}

/**
 * Score `text` against `query`, or null when it doesn't match. `query` must
 * already be lowercase and non-empty; case in `text` still counts towards
 * where words start.
 */
export function fuzzyMatch(query: string, text: string): Match | null {
  const lower = text.toLowerCase()
  const n = query.length
  const m = lower.length
  if (n === 0 || n > m || m > MAX_LENGTH || !isSubsequence(query, lower)) return null

  const bonus = new Float64Array(m)
  for (let j = 0; j < m; j++) bonus[j] = bonusAt(text, j)

  // D[i][j]: best score with query[i] matched exactly at text[j].
  // M[i][j]: best score for query[0..i] within text[0..j], matched anywhere.
  const D: Float64Array[] = []
  const M: Float64Array[] = []
  for (let i = 0; i < n; i++) {
    const d = new Float64Array(m).fill(-Infinity)
    const best = new Float64Array(m).fill(-Infinity)
    const gap = i === n - 1 ? GAP_TRAILING : GAP_INNER
    let prev = -Infinity
    for (let j = 0; j < m; j++) {
      if (query[i] === lower[j]) {
        let score = -Infinity
        if (i === 0) score = j * GAP_LEADING + bonus[j]
        else if (j > 0) {
          score = Math.max(M[i - 1][j - 1] + bonus[j], D[i - 1][j - 1] + MATCH_CONSECUTIVE)
        }
        d[j] = score
        prev = Math.max(score, prev + gap)
      } else {
        prev = prev + gap
      }
      best[j] = prev
    }
    D.push(d)
    M.push(best)
  }

  // Walk back through the tables for the characters that gave that score.
  const positions = new Array<number>(n)
  let mustMatch = false
  let j = m - 1
  for (let i = n - 1; i >= 0; i--) {
    for (; j >= 0; j--) {
      if (D[i][j] !== -Infinity && (mustMatch || D[i][j] === M[i][j])) {
        mustMatch = i > 0 && j > 0 && M[i][j] === D[i - 1][j - 1] + MATCH_CONSECUTIVE
        positions[i] = j--
        break
      }
    }
  }

  return { score: M[n - 1][m - 1] + (n === m ? EXACT : 0), positions }
}

/** `text` cut into runs that did and didn't match, for highlighting. */
export function highlight(text: string, positions: Iterable<number>): { text: string; hit: boolean }[] {
  const hits = new Set(positions)
  const parts: { text: string; hit: boolean }[] = []
  for (let i = 0; i < text.length; i++) {
    const hit = hits.has(i)
    const last = parts[parts.length - 1]
    if (last && last.hit === hit) last.text += text[i]
    else parts.push({ text: text[i], hit })
  }
  return parts
}
