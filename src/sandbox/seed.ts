import { loadConfig, refresh, state } from '../store'
import { active, projectOf, setStatus } from '../terminalState'
import { normalizePath } from '../tasks'
import type { AppConfig, Overview, TerminalInfo } from '../api'
import { backend } from './backend'
import { config, overview as defaultOverview, peeked } from './fixtures'

/**
 * What most scenarios start from: the config loaded and a project opened,
 * through the same `loadConfig` and `refresh` the app calls, so the
 * store's own bookkeeping runs rather than being poked into place.
 */
export interface Boot {
  config?: AppConfig
  overview?: Overview
  /** What `refresh` never answers — the list is loading — while `peek` does
   *  (`true`) or does not (`'nothing'`), as in a project's first seconds. */
  loading?: boolean | 'nothing'
  selected?: string | null
  filter?: typeof state.filter
}

export async function boot(options: Boot = {}) {
  const cfg = options.config ?? config()
  const full = options.overview ?? defaultOverview()
  const never = new Promise<never>(() => {})
  backend.handle({
    get_config: () => structuredClone(cfg),
    refresh: () => (options.loading ? never : structuredClone(full)),
    peek: () => (options.loading === 'nothing' ? never : structuredClone(options.loading ? peeked(full) : full)),
  })
  state.filter = options.filter ?? 'active'
  await loadConfig()
  state.selectedBranch = options.selected ?? null
  const done = refresh()
  // A loading list is the point: don't wait for what will not come.
  if (!options.loading) await done
}

/** Terminals as the backend reports them: a running shell per entry, with
 *  what its program last said. Keyed and normalised as `setStatus` does. */
export function terminals(
  list: {
    path: string
    status?: TerminalInfo['status']
    ready?: boolean
    minutesAgo?: number
    /** What a working or blocked program says it is doing. */
    message?: string
  }[],
  projectId = 'app',
) {
  for (const t of list) {
    const key = normalizePath(t.path)
    const at = Date.now() - (t.minutesAgo ?? 0) * 60_000
    active.add(key)
    projectOf.set(key, projectId)
    setStatus({
      path: key,
      projectId,
      status: t.status ?? null,
      ready: t.ready ?? false,
      activeAt: at,
      waitingSince: t.status === 'blocked' || t.ready ? at : null,
      message: t.message ?? null,
    })
  }
}
