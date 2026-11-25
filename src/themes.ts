/**
 * The colour themes. Each has a light and a dark palette, and the OS
 * appearance picks between them as it always has: the palettes are written
 * out as a stylesheet, `:root[data-theme=…]` for light and the same inside
 * `prefers-color-scheme: dark`. The first theme is also plain `:root`, so
 * nothing is uncoloured before a theme is chosen.
 *
 * This file is the only place a colour token gets its value. `styles.css`
 * keeps the tokens that aren't colours (sizes, radii).
 *
 * `--ready` is the mark's blue in every theme, apart from the accent: the
 * marks are orange and blue whatever the theme (the onboarding and the site
 * say so), and an accent that turned the ready mark green would make it
 * read as something else.
 */
import { ref } from 'vue'

export type Scheme = 'light' | 'dark'

export interface Palette {
  bg: string
  'bg-raised': string
  'bg-sidebar': string
  'bg-hover': string
  'bg-active': string
  border: string
  'border-soft': string
  text: string
  'text-dim': string
  'text-faint': string
  /** Hex: the terminal's selection is derived from it. */
  accent: string
  'accent-fg': string
  /** The ready mark and its queue card. Always a blue. */
  ready: string
  green: string
  red: string
  'danger-bg': string
  'danger-fg': string
  amber: string
  orange: string
  purple: string
  skeleton: string
  'skeleton-hi': string
  /** Behind a label laid over other content: away from the text colour. */
  'label-scrim': string
  /** The chosen segment of a segmented control, raised off its track. */
  'control-on': string
  shadow: string
}

export interface Theme {
  id: string
  name: string
  light: Palette
  dark: Palette
}

/** The status colours most themes share, so each theme states only what
 *  makes it different. */
const STATUS_LIGHT = {
  ready: '#2f6fed',
  green: '#1a7f37',
  red: '#c93c37',
  'danger-bg': '#c93c37',
  'danger-fg': '#ffffff',
  amber: '#9a6700',
  orange: '#d0630e',
  purple: '#7b47c4',
} as const

const STATUS_DARK = {
  ready: '#4b8bff',
  green: '#4ac26b',
  red: '#f2695f',
  'danger-bg': '#c4423a',
  'danger-fg': '#ffffff',
  amber: '#d4a72c',
  orange: '#f0883e',
  purple: '#b585f5',
} as const

/** Hover, borders and skeletons are the text's ink at low strength, tinted
 *  as the theme's neutrals are. */
function ink(rgb: string, scheme: Scheme) {
  const a = (light: number, dark: number) => `rgba(${rgb}, ${scheme === 'light' ? light : dark})`
  return {
    'bg-hover': a(0.045, 0.055),
    'bg-active': a(0.075, 0.09),
    border: a(0.11, 0.12),
    'border-soft': a(0.06, 0.07),
    skeleton: a(0.07, 0.07),
    'skeleton-hi': a(0.13, 0.15),
  }
}

const SHADOW = {
  light: '0 10px 30px rgba(0, 0, 0, 0.16)',
  dark: '0 12px 34px rgba(0, 0, 0, 0.5)',
}

export const THEMES: Theme[] = [
  {
    id: 'graphite',
    name: 'Graphite',
    light: {
      bg: '#f6f6f7',
      'bg-raised': '#ffffff',
      'bg-sidebar': '#ececee',
      ...ink('0, 0, 0', 'light'),
      text: '#1c1c1e',
      'text-dim': '#6d6d72',
      'text-faint': '#96969c',
      accent: '#2f6fed',
      'accent-fg': '#ffffff',
      ...STATUS_LIGHT,
      'label-scrim': 'rgba(255, 255, 255, 0.6)',
      'control-on': '#ffffff',
      shadow: SHADOW.light,
    },
    dark: {
      bg: '#1c1c1e',
      'bg-raised': '#262629',
      'bg-sidebar': '#202022',
      ...ink('255, 255, 255', 'dark'),
      text: '#f2f2f4',
      'text-dim': '#9e9ea5',
      'text-faint': '#6f6f77',
      accent: '#4b8bff',
      'accent-fg': '#ffffff',
      ...STATUS_DARK,
      'label-scrim': 'rgba(0, 0, 0, 0.2)',
      'control-on': '#4a4a4f',
      shadow: SHADOW.dark,
    },
  },
  {
    id: 'sand',
    name: 'Sand',
    light: {
      bg: '#f4f0e8',
      'bg-raised': '#fffcf6',
      'bg-sidebar': '#e9e3d7',
      ...ink('70, 50, 20', 'light'),
      text: '#2a251f',
      'text-dim': '#73695c',
      'text-faint': '#9f9585',
      accent: '#b03d68',
      'accent-fg': '#ffffff',
      ...STATUS_LIGHT,
      green: '#2f7a34',
      red: '#c0392b',
      'danger-bg': '#c0392b',
      amber: '#95650a',
      'label-scrim': 'rgba(255, 252, 246, 0.6)',
      'control-on': '#fffcf6',
      shadow: '0 10px 30px rgba(60, 40, 10, 0.16)',
    },
    dark: {
      bg: '#1f1c18',
      'bg-raised': '#29251f',
      'bg-sidebar': '#231f1b',
      ...ink('255, 240, 215', 'dark'),
      text: '#f3ede3',
      'text-dim': '#a89e8f',
      'text-faint': '#776e62',
      accent: '#d4588a',
      'accent-fg': '#ffffff',
      ...STATUS_DARK,
      green: '#6cbf5a',
      amber: '#d9a93a',
      'label-scrim': 'rgba(0, 0, 0, 0.2)',
      'control-on': '#4d463d',
      shadow: SHADOW.dark,
    },
  },
  {
    id: 'moss',
    name: 'Moss',
    light: {
      bg: '#f2f5f1',
      'bg-raised': '#fcfdfb',
      'bg-sidebar': '#e5ebe3',
      ...ink('20, 50, 25', 'light'),
      text: '#1d241e',
      'text-dim': '#637066',
      'text-faint': '#8f9a91',
      accent: '#2e7a52',
      'accent-fg': '#ffffff',
      ...STATUS_LIGHT,
      green: '#2a8a3a',
      'label-scrim': 'rgba(252, 253, 251, 0.6)',
      'control-on': '#fcfdfb',
      shadow: SHADOW.light,
    },
    dark: {
      bg: '#171c18',
      'bg-raised': '#1f2620',
      'bg-sidebar': '#1a201b',
      ...ink('220, 255, 225', 'dark'),
      text: '#edf3ee',
      'text-dim': '#98a69b',
      'text-faint': '#687569',
      accent: '#3f9a63',
      'accent-fg': '#ffffff',
      ...STATUS_DARK,
      green: '#5cc77a',
      'label-scrim': 'rgba(0, 0, 0, 0.2)',
      'control-on': '#3f4a41',
      shadow: SHADOW.dark,
    },
  },
  {
    id: 'tide',
    name: 'Tide',
    light: {
      bg: '#f1f4f7',
      'bg-raised': '#fbfcfe',
      'bg-sidebar': '#e3e9ef',
      ...ink('15, 35, 60', 'light'),
      text: '#18212b',
      'text-dim': '#5e6b78',
      'text-faint': '#8c98a4',
      accent: '#0e7c86',
      'accent-fg': '#ffffff',
      ...STATUS_LIGHT,
      'label-scrim': 'rgba(251, 252, 254, 0.6)',
      'control-on': '#fbfcfe',
      shadow: '0 10px 30px rgba(10, 30, 60, 0.16)',
    },
    dark: {
      bg: '#141a21',
      'bg-raised': '#1c242d',
      'bg-sidebar': '#171e26',
      ...ink('210, 230, 255', 'dark'),
      text: '#edf2f7',
      'text-dim': '#95a2b0',
      'text-faint': '#64707d',
      accent: '#1a96a1',
      'accent-fg': '#ffffff',
      ...STATUS_DARK,
      'label-scrim': 'rgba(0, 0, 0, 0.2)',
      'control-on': '#36424f',
      shadow: SHADOW.dark,
    },
  },
  {
    id: 'plum',
    name: 'Plum',
    light: {
      bg: '#f6f3f8',
      'bg-raised': '#fefcff',
      'bg-sidebar': '#ebe5f0',
      ...ink('45, 20, 65', 'light'),
      text: '#241c2b',
      'text-dim': '#6c6276',
      'text-faint': '#998fa3',
      accent: '#7445c7',
      'accent-fg': '#ffffff',
      ...STATUS_LIGHT,
      // Apart from the accent, which has taken the violet.
      purple: '#a03fb0',
      'label-scrim': 'rgba(254, 252, 255, 0.6)',
      'control-on': '#fefcff',
      shadow: '0 10px 30px rgba(40, 10, 60, 0.16)',
    },
    dark: {
      bg: '#1c1820',
      'bg-raised': '#25202b',
      'bg-sidebar': '#201b25',
      ...ink('240, 220, 255', 'dark'),
      text: '#f3eff7',
      'text-dim': '#a59cae',
      'text-faint': '#746b7d',
      accent: '#8f63e0',
      'accent-fg': '#ffffff',
      ...STATUS_DARK,
      purple: '#d77ee6',
      'label-scrim': 'rgba(0, 0, 0, 0.2)',
      'control-on': '#4a4152',
      shadow: SHADOW.dark,
    },
  },
]

export const DEFAULT_THEME = THEMES[0].id

export function themeById(id: string | null | undefined): Theme {
  return THEMES.find((t) => t.id === id) ?? THEMES[0]
}

function declarations(palette: Palette, scheme: Scheme): string {
  const lines = Object.entries(palette).map(([name, value]) => `  --${name}: ${value};`)
  return `${lines.join('\n')}\n  color-scheme: ${scheme};`
}

/** Every theme as CSS. A selector of `:root[data-theme]` outranks the plain
 *  `:root`, and each theme's dark rule comes after its light one, so the
 *  shown theme's palette for the OS appearance is the one that applies. */
export function themeCss(): string {
  const rules: string[] = []
  const selector = (t: Theme) =>
    t.id === DEFAULT_THEME ? `:root, :root[data-theme='${t.id}']` : `:root[data-theme='${t.id}']`
  for (const t of THEMES) rules.push(`${selector(t)} {\n${declarations(t.light, 'light')}\n}`)
  const dark = THEMES.map((t) => `${selector(t)} {\n${declarations(t.dark, 'dark')}\n}`)
  rules.push(`@media (prefers-color-scheme: dark) {\n${dark.join('\n')}\n}`)
  return rules.join('\n')
}

const STORAGE_KEY = 'parallelist:theme'

/** The theme on screen: the saved one, or the one Settings is previewing. */
export const shownTheme = ref<string>(DEFAULT_THEME)

export function showTheme(id: string | null | undefined) {
  const theme = themeById(id)
  document.documentElement.dataset.theme = theme.id
  shownTheme.value = theme.id
  try {
    localStorage.setItem(STORAGE_KEY, theme.id)
  } catch {
    /* no storage: the config still has it, a moment later */
  }
}

/** Writes the themes' stylesheet, and shows the theme last shown, before the
 *  config has loaded, so the window doesn't open in the default and then
 *  change colour. */
export function installThemes() {
  const style = document.createElement('style')
  style.id = 'themes'
  style.textContent = themeCss()
  document.head.prepend(style)
  let last: string | null = null
  try {
    last = localStorage.getItem(STORAGE_KEY)
  } catch {
    /* as above */
  }
  showTheme(last)
}
