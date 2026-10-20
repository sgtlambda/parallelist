import { reactive } from 'vue'
import { api, type Issue } from './api'
import { fuzzyMatch } from './fuzzy'
import type { Row } from './store'

/**
 * The New workspace dialog's branch field: what it offers as you type.
 *
 * The text as typed comes first, as the branch to create — or, when a branch
 * by that name exists, as that branch — so Enter does what it always did.
 * Below it, the project's branches and its open issues, fuzzy-matched with
 * the palette's scoring. Picking an issue fills in a branch name for it.
 */

/** Shown per section; the query narrows it long before anyone scrolls. */
const SECTION_LIMIT = 8

/** An issue list younger than this is not fetched again on reopening. */
const FRESH_MS = 60_000

/** A word found in a PR title rather than the branch name counts for less,
 *  as a match outside the branch name does in the palette. */
const SECONDARY_WEIGHT = 0.75

export type Option =
  | { kind: 'create'; name: string }
  | { kind: 'branch'; row: Row; hits: number[] }
  | { kind: 'issue'; issue: Issue; hits: number[]; numberHit: boolean }

/** What the field holds, as a branch name. Spaces separate search words,
 *  and a branch can't have any, so they become dashes. */
export function branchName(text: string): string {
  return text.trim().replace(/\s+/g, '-')
}

// ---------- issues, fetched per project and kept for the session ----------

interface IssueList {
  issues: Issue[]
  at: number
}

const issueLists = reactive(new Map<string, IssueList>())
const issuesLoading = reactive(new Set<string>())
const issueErrors = reactive(new Map<string, string>())

export function issuesOf(id: string): Issue[] {
  return issueLists.get(id)?.issues ?? []
}

export function issuesPending(id: string): boolean {
  return issuesLoading.has(id)
}

export function issuesError(id: string): string | null {
  return issueErrors.get(id) ?? null
}

/** Fetch a project's open issues unless a recent list is held. What is held
 *  shows meanwhile. */
export function loadIssues(id: string) {
  if (issuesLoading.has(id)) return
  const held = issueLists.get(id)
  if (held && Date.now() - held.at < FRESH_MS) return
  issuesLoading.add(id)
  api
    .listIssues(id)
    .then((issues) => {
      issueLists.set(id, { issues, at: Date.now() })
      issueErrors.delete(id)
    })
    // No `gh`, not logged in, not a GitHub repository: the dialog says so in
    // the list and still offers branches.
    .catch((e) => void issueErrors.set(id, String(e)))
    .finally(() => issuesLoading.delete(id))
}

// ---------- matching ----------

function words(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean)
}

/** Where a run of matched characters may start without being a substring
 *  of its own: the start of a word, as `fuzzy.ts` scores them. */
function startsWord(text: string, j: number): boolean {
  if (j === 0) return true
  const prev = text[j - 1]
  if (!/[a-z0-9]/i.test(prev)) return true
  return /[a-z]/.test(prev) && /[A-Z]/.test(text[j])
}

/** A run starting mid-word must be this long to count as meant. */
const MID_WORD_RUN = 3

/**
 * Whether a match reads as meant: every run of matched characters starts a
 * word, or is long enough to be a substring typed on purpose. fzy alone
 * accepts any subsequence, and across a few hundred branches and issue
 * titles that buried the real hits under scattered ones — `sso` found
 * `cla·ss-·o·rder` ahead of the issues about SSO.
 */
function wellPlaced(text: string, positions: number[]): boolean {
  for (let k = 0; k < positions.length; k++) {
    const start = positions[k]
    if (k > 0 && positions[k - 1] === start - 1) continue
    let end = k
    while (end + 1 < positions.length && positions[end + 1] === positions[end] + 1) end++
    if (!startsWord(text, start) && end - k + 1 < MID_WORD_RUN) return false
  }
  return true
}

function match(word: string, text: string) {
  const m = fuzzyMatch(word, text)
  return m && wellPlaced(text, m.positions) ? m : null
}

function matchBranch(row: Row, query: string[]): { score: number; hits: number[] } | null {
  let score = 0
  const hits: number[] = []
  for (const word of query) {
    const inName = match(word, row.branch.name)
    const inPr = row.pr ? match(word, row.pr.title) : null
    const pr = inPr ? inPr.score * SECONDARY_WEIGHT : -Infinity
    if (inName && inName.score >= pr) {
      score += inName.score
      hits.push(...inName.positions)
    } else if (inPr) {
      score += pr
    } else {
      return null
    }
  }
  return { score, hits }
}

function matchIssue(
  issue: Issue,
  query: string[],
): { score: number; hits: number[]; numberHit: boolean } | null {
  let score = 0
  const hits: number[] = []
  let numberHit = false
  const number = String(issue.number)
  for (const word of query) {
    // `1234` or `#1234`, from the start: issue numbers are typed, not
    // fuzzed. Scored like a run of consecutive matches, plus the exact bonus
    // when it is the whole number.
    const digits = /^#?(\d+)$/.exec(word)?.[1]
    if (digits && number.startsWith(digits)) {
      score += digits.length + (digits === number ? 1.5 : 0)
      numberHit = true
      continue
    }
    const m = match(word, issue.title)
    if (!m) return null
    score += m.score
    hits.push(...m.positions)
  }
  return { score, hits, numberHit }
}

/**
 * Everything to offer for `text`. The first option is the text itself —
 * the branch to create, or the existing branch it names — whenever there is
 * text; that is the default Enter takes. Branches keep their list order
 * with no query, issues their most-recently-updated one.
 */
export function search(text: string, rows: Row[], issues: Issue[]): Option[] {
  const name = branchName(text)
  const query = words(text)
  const exact = name ? rows.find((r) => r.branch.name === name) : undefined

  const options: Option[] = []
  if (exact) options.push({ kind: 'branch', row: exact, hits: [] })
  else if (name) options.push({ kind: 'create', name })

  const branches = rows
    .filter((r) => r !== exact)
    .map((row) => ({ row, m: query.length ? matchBranch(row, query) : { score: 0, hits: [] } }))
    .filter((x) => x.m !== null)
    .sort((a, b) => b.m!.score - a.m!.score)
    .slice(0, SECTION_LIMIT)
  for (const { row, m } of branches) options.push({ kind: 'branch', row, hits: m!.hits })

  const matched = issues
    .map((issue) => ({
      issue,
      m: query.length ? matchIssue(issue, query) : { score: 0, hits: [], numberHit: false },
    }))
    .filter((x) => x.m !== null)
    .sort((a, b) => b.m!.score - a.m!.score)
    .slice(0, SECTION_LIMIT)
  for (const { issue, m } of matched) options.push({ kind: 'issue', issue, ...m! })

  return options
}

// ---------- a branch name for an issue ----------

/** Prefixes by issue type, in order of preference. Only one the project's
 *  branches already use is taken: a repository that doesn't prefix its
 *  branches doesn't get `fix/` from us. */
const PREFIXES: Record<string, string[]> = {
  bug: ['fix', 'bugfix', 'bug', 'hotfix'],
  feature: ['feature', 'feat'],
  other: ['maintenance', 'chore', 'task'],
}

/** Words that only lengthen a branch name. */
const FILLER = new Set(
  'a an the to of for in on at by with and or is are be it its this that from as into when'.split(
    ' ',
  ),
)

/** Words of the title kept. Enough to tell branches apart, few enough to
 *  type; the name is selected after picking, to be renamed in place. */
const TITLE_WORDS = 5

export function prefixFor(issue: Issue, rows: Row[]): string {
  const type = issue.issueType?.toLowerCase()
  const candidates = PREFIXES[type === 'bug' || type === 'feature' ? type : 'other']
  const used = new Set(rows.map((r) => r.branch.name.split('/')[0]))
  const prefix = candidates.find((p) => used.has(p))
  return prefix ? `${prefix}/` : ''
}

/**
 * The branch to put an issue in: the one GitHub has linked to it, else its
 * title cut down to a few words, behind the prefix the project's convention
 * gives that type of issue (`fix/`, `feature/`, …).
 */
export function branchForIssue(issue: Issue, rows: Row[]): { name: string; linked: boolean } {
  const linked = issue.linkedBranches[0]
  if (linked) return { name: linked, linked: true }
  const slug = issue.title
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((w) => w && !FILLER.has(w))
    .slice(0, TITLE_WORDS)
    .join('-')
  return { name: prefixFor(issue, rows) + (slug || `issue-${issue.number}`), linked: false }
}
