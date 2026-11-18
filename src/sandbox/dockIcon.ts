/**
 * The Dock icon while terminals are working: the prompt's cursor blinks, as
 * the terminal glyph's does in the app (`.icon-blink`, 1.1 s, on then off),
 * while anything is working.
 */

/** Seconds per blink, the app's `cursor-blink`. */
export const CURSOR_PERIOD = 1.1

/** Whether the cursor is drawn at `t` seconds. Each period starts lit. */
export function cursorOn(working: boolean, t: number, period = CURSOR_PERIOD): boolean {
  if (!working) return true
  const phase = t / period
  return phase - Math.floor(phase) < 0.5
}
