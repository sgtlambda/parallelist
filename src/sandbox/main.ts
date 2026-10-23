import { createApp } from 'vue'
import '../styles.css'
import { applyTheme, storedTheme, type Theme } from './theme'

// One page, two modes: the shell with its list of scenarios, and — inside
// that shell's iframe, so each scenario starts from a clean slate and the
// window can be any width — a single scenario.
const params = new URLSearchParams(location.search)
const frame = params.get('frame')

// `?theme=light|dark` wins over the saved choice, for a link that has to look
// the same wherever it is opened.
const forced = params.get('theme')
applyTheme(forced === 'light' || forced === 'dark' ? (forced as Theme) : storedTheme())

if (frame) {
  const { default: Frame } = await import('./Frame.vue')
  createApp(Frame, { id: frame }).mount('#app')
} else {
  const { default: Sandbox } = await import('./Sandbox.vue')
  createApp(Sandbox).mount('#app')
}
