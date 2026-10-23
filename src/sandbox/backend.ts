/**
 * The sandbox's stand-in for the Rust backend. Every Tauri module the app
 * imports is aliased (vite.config.ts, `--mode sandbox`) to a stub in
 * `./tauri`, and each of those lands here: `invoke` is answered by a handler
 * a scenario registered, and `listen` hooks into `emit`.
 */

type Handler = (args: any) => unknown

const handlers = new Map<string, Handler>()
const listeners = new Map<string, Set<(event: { event: string; id: number; payload: unknown }) => void>>()

/** Commands that are fire-and-forget or whose empty answer is the honest one,
 *  so a scenario need not mention them. Anything else with no handler fails
 *  loudly: a silent default would hide a fixture the scenario is missing. */
const quiet: Record<string, Handler> = {
  list_tasks: () => [],
  get_task: () => null,
  set_task_attention: () => undefined,
  select_project: () => undefined,
  save_config: () => undefined,
  ahead_behind: () => ({}),
  terminal_list: () => [],
  terminal_viewing: () => undefined,
  terminal_mark: () => undefined,
  terminal_close: () => undefined,
  copy_text: () => undefined,
  open_link: () => undefined,
}

/** Arguments as the log shows them: what JSON can say, and no more. */
function plain(value: unknown): unknown {
  try {
    return JSON.parse(JSON.stringify(value ?? null, (_, v) => (typeof v === 'function' ? undefined : v)))
  } catch {
    return String(value)
  }
}

/** Tell the page around the frame, if there is one, what just happened. */
export function log(kind: 'call' | 'emit' | 'open', name: string, detail?: unknown) {
  if (window.parent === window) return
  window.parent.postMessage({ type: 'sandbox:log', kind, name, detail: plain(detail) }, location.origin)
}

export const backend = {
  /** Answer these commands, replacing any earlier answer. */
  handle(table: Record<string, Handler>) {
    for (const [cmd, handler] of Object.entries(table)) handlers.set(cmd, handler)
  },

  async call(cmd: string, args: unknown): Promise<unknown> {
    log('call', cmd, args)
    const handler = handlers.get(cmd) ?? quiet[cmd]
    if (!handler) {
      throw new Error(`sandbox: no handler for backend command "${cmd}"`)
    }
    return handler(args ?? {})
  },

  /** Deliver an event, as the backend's `emit` would. */
  emit(event: string, payload: unknown) {
    listeners.get(event)?.forEach((l) => l({ event, id: 0, payload }))
  },

  listen(event: string, callback: (e: { event: string; id: number; payload: unknown }) => void) {
    const set = listeners.get(event) ?? new Set()
    set.add(callback)
    listeners.set(event, set)
    return () => void set.delete(callback)
  },
}
