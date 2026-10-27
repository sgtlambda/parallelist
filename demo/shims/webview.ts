/** `@tauri-apps/api/webview`: nothing is dragged onto a web page from Finder. */
export function getCurrentWebview() {
  return {
    onDragDropEvent: async (): Promise<() => void> => () => {},
  }
}
