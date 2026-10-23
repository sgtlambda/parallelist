/**
 * Light or dark on demand. `styles.css` only has the `prefers-color-scheme`
 * block, which a page cannot flip, so a forced theme copies that block's
 * `:root` values (or the plain ones) onto the root element, where they win.
 * The app itself stays as it is.
 */

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

let applied: string[] = []

function rootRule(dark: boolean): CSSStyleRule | null {
  for (const sheet of document.styleSheets) {
    let rules: CSSRuleList
    try {
      rules = sheet.cssRules
    } catch {
      continue
    }
    for (const rule of rules) {
      if (dark && rule instanceof CSSMediaRule && rule.conditionText.includes('prefers-color-scheme: dark')) {
        for (const inner of rule.cssRules) {
          if (inner instanceof CSSStyleRule && inner.selectorText === ':root') return inner
        }
      }
      if (!dark && rule instanceof CSSStyleRule && rule.selectorText === ':root' && rule.style.getPropertyValue('--bg')) {
        return rule
      }
    }
  }
  return null
}

export function applyTheme(theme: Theme) {
  const root = document.documentElement
  applied.forEach((name) => root.style.removeProperty(name))
  applied = []
  if (theme === 'system') return
  const rule = rootRule(theme === 'dark')
  if (!rule) return
  for (const name of Array.from(rule.style)) {
    root.style.setProperty(name, rule.style.getPropertyValue(name))
    applied.push(name)
  }
}
