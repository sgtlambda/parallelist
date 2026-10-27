import type { DragDropEvent } from '@tauri-apps/api/webview'
import { backend } from '../backend'

/** A drag from Finder is a `drag-drop` event a scenario emits, with the
 *  payload Tauri's `onDragDropEvent` would hand over. */
export function getCurrentWebview() {
  return {
    onDragDropEvent: (handler: (e: { payload: DragDropEvent }) => void): Promise<() => void> =>
      Promise.resolve(backend.listen('drag-drop', (e) => handler({ payload: e.payload as DragDropEvent }))),
  }
}
