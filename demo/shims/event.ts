type Handler = (event: { payload: any }) => void

const listeners = new Map<string, Set<Handler>>()

/** `@tauri-apps/api/event`: the backend's `emit` reaches whoever listened. */
export async function listen<T>(
  name: string,
  handler: (event: { payload: T }) => void,
): Promise<() => void> {
  const set = listeners.get(name) ?? new Set()
  listeners.set(name, set)
  set.add(handler)
  return () => void set.delete(handler)
}

export function emit(name: string, payload: unknown) {
  for (const handler of listeners.get(name) ?? []) handler({ payload })
}
