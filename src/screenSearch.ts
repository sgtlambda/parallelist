import { ref, shallowRef } from 'vue'
import {
  api,
  onScreenSearchStatus,
  type ScreenFound,
  type ScreenHit,
  type ScreenSearchStatus,
} from './api'
import { state } from './store'
import { normalizePath } from './tasks'

/**
 * Screen search: what the terminals show, matched against the palette's
 * query by the backend (`screen_search.rs`), literally and by meaning. Only
 * asked while the palette has a query, and only with the setting on. Until
 * the model is ready nothing is found, and the palette says where it is
 * (`modelNote`).
 *
 * One search is in flight at a time. Keystrokes typed meanwhile replace
 * each other, and only the last is searched once it returns, so a burst of
 * typing costs two searches, not one per key. The model's worker starting
 * up, or a model finishing its download, searches the query again, since
 * what came back before was literal only.
 */

/** Where the model is, when that is worth saying: being downloaded or
 *  started, or failed and why. Null while it is off, idle or ready. */
export function modelNote(status: ScreenSearchStatus): { text: string; error: boolean } | null {
  if (status.state === 'loading') {
    if (status.downloaded === null || status.size === null) {
      return { text: 'Starting the search model…', error: false }
    }
    const mb = (bytes: number) => Math.round(bytes / 1_000_000)
    return {
      text: `Downloading the search model · ${mb(status.downloaded)} of ${mb(status.size)} MB`,
      error: false,
    }
  }
  if (status.state === 'failed') {
    return { text: `The search model couldn't start: ${status.error}`, error: true }
  }
  return null
}

/** Shorter queries are left to branch names, as in the backend. */
export const MIN_SCREEN_QUERY = 3

export const screenStatus = ref<ScreenSearchStatus>({ state: 'off' })

/** The last answer, for the query it answers. */
export const screenFound = shallowRef<ScreenFound | null>(null)

export const screenSearching = ref(false)

/** The query the palette shows, or null while it is closed. */
let wanted: string | null = null
/** `wanted` hasn't been searched since it was last asked for. */
let dirty = false
let inFlight = false

export async function startScreenSearchSync(): Promise<() => void> {
  const stop = await onScreenSearchStatus((status) => {
    screenStatus.value = status
    if (status.state === 'ready' && wanted) searchScreens(wanted)
  })
  try {
    screenStatus.value = await api.screenSearchStatus()
  } catch {
    /* stays off */
  }
  return stop
}

/** Search the screens for `query`, as the palette's query changes. */
export function searchScreens(query: string) {
  const q = query.trim()
  if (!state.config?.screenSearch || q.length < MIN_SCREEN_QUERY) {
    wanted = null
    screenFound.value = null
    return
  }
  wanted = q
  dirty = true
  if (!inFlight) void run()
}

/** The palette closed: forget what was found, and search nothing more. */
export function stopScreenSearch() {
  wanted = null
  dirty = false
  screenFound.value = null
}

async function run() {
  inFlight = true
  screenSearching.value = true
  while (dirty && wanted !== null) {
    dirty = false
    const q = wanted
    try {
      const found = await api.screenSearch(q)
      screenStatus.value = found.status
      // Not if the palette closed or the query was cleared meanwhile.
      if (wanted === null) continue
      screenFound.value = found
      // Then which words matched, for the matches by meaning on screen,
      // unless the query has moved on already.
      if (!dirty) await markMeanings(found)
    } catch {
      // The palette goes on with branch names.
    }
  }
  inFlight = false
  screenSearching.value = false
}

/** The rows the palette shows at most, of which the matches by meaning get
 *  their marks. */
const MARKED = 12

async function markMeanings(found: ScreenFound) {
  const unmarked = found.hits.slice(0, MARKED).filter((h) => h.matchedBy === 'meaning' && !h.marks.length)
  if (!unmarked.length) return
  const marks = await api.screenSearchMarks(
    found.query,
    unmarked.map((h) => h.snippet),
  )
  if (screenFound.value?.query !== found.query) return
  const byPath = new Map(unmarked.map((h, i) => [h.path, marks[i] ?? []]))
  screenFound.value = {
    ...found,
    hits: found.hits.map((h) => (byPath.has(h.path) ? { ...h, marks: byPath.get(h.path)! } : h)),
  }
}

/**
 * The hits that apply to `query`, by normalised path: those for the query
 * itself, or, while its search is still on the way, for what it was before
 * more was typed. Hits for a longer query don't apply to a shorter one.
 */
export function screenHits(query: string): Map<string, ScreenHit> {
  const found = screenFound.value
  const q = query.trim()
  const hits = new Map<string, ScreenHit>()
  if (!found || q.length < MIN_SCREEN_QUERY || !q.toLowerCase().startsWith(found.query.toLowerCase())) {
    return hits
  }
  for (const hit of found.hits) hits.set(normalizePath(hit.path), hit)
  return hits
}

/** A hit's marks as the positions `highlight` takes. */
export function markPositions(marks: [number, number][]): number[] {
  return marks.flatMap(([start, end]) => Array.from({ length: end - start }, (_, i) => start + i))
}
