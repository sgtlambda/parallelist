import { backend } from '../backend'

export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return backend.call(cmd, args) as Promise<T>
}

/** What the backend writes to; a handler calls `onmessage` itself. */
export class Channel<T = unknown> {
  onmessage: (message: T) => void = () => {}
}
