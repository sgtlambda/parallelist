/**
 * Light or dark on demand. The themes' stylesheet picks a palette by
 * `prefers-color-scheme`, which a page cannot flip, so a forced appearance
 * sets the shown theme's palette for it on the root element, where it wins.
 * The app itself stays as it is. (Here "theme" is the appearance; the
 * app's colour themes are `themes.ts`.)
 */
import { watch } from 'vue'
import { THEMES, shownTheme, themeById } from '../themes'


export type Theme = 'system' | 'light' | 'dark'

const KEY = 'sandbox:theme'

export function storedTheme(): Theme {
  try {
    const value = localStorage.getItem(KEY)
    if (value === 'light' || value === 'dark') return value
  } catch {
    /* private window: the default will do */
  }
  return 'system'
}

export function saveTheme(theme: Theme) {
  try {
    localStorage.setItem(KEY, theme)
  } catch {
    /* as above */
  }
}

let forced: Theme = 'system'

/** Sets the shown theme's palette for the forced appearance on the root
 *  element, where it wins over the stylesheet's `prefers-color-scheme`. */
function apply() {
  const root = document.documentElement
  for (const name of Object.keys(THEMES[0].light)) root.style.removeProperty(`--${name}`)
  root.style.removeProperty('color-scheme')
  if (forced === 'system') return
  const palette = themeById(shownTheme.value)[forced]
  for (const [name, value] of Object.entries(palette)) root.style.setProperty(`--${name}`, value)
  root.style.setProperty('color-scheme', forced)
}

watch(shownTheme, apply)

export function applyTheme(theme: Theme) {
  forced = theme
  apply()
}
