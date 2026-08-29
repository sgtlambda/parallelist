import { onUnmounted } from 'vue'

/**
 * Close on Escape while `isOpen` reports true.
 *
 * Listens on the window rather than the dialog element: a modal's content is
 * not always focused, so a keydown handler on the element itself would miss
 * most presses.
 */
export function useEscape(isOpen: () => boolean, close: () => void) {
  const onKey = (event: KeyboardEvent) => {
    if (event.key !== 'Escape' || !isOpen()) return
    event.stopPropagation()
    close()
  }
  window.addEventListener('keydown', onKey)
  onUnmounted(() => window.removeEventListener('keydown', onKey))
}
