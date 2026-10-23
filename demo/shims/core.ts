import { call } from '../backend'

/** `@tauri-apps/api/core`, answered by the page instead of the Rust side. */
export function invoke<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return call(command, args) as Promise<T>
}

/** Rust-to-webview streaming: the backend assigns `onmessage`'s caller. */
export class Channel<T> {
  onmessage: (message: T) => void = () => {}
}
