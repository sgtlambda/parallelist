import { createApp, watch } from 'vue'
import App from '../src/App.vue'
import '../src/styles.css'
import './demo.css'
import { startStory } from './backend'
import { state } from '../src/store'
import { showTerminal } from '../src/terminal'

/**
 * The app's own frontend with a scripted backend behind it. This is embedded
 * in the landing page, so two things a window of its own wouldn't need:
 *
 * - A terminal that is shown takes the keyboard (`mayTakeFocus`). Focusing
 *   something inside an iframe scrolls the page around it to the frame, so
 *   until the visitor has pressed or clicked here, the terminal's input is
 *   left unfocused.
 * - What the sessions do on their own begins when the frame scrolls into
 *   view, not when the page loads.
 */

let touched = false
for (const type of ['pointerdown', 'keydown']) {
  window.addEventListener(type, () => (touched = true), { capture: true, once: true })
}
const focus = HTMLElement.prototype.focus
HTMLElement.prototype.focus = function (this: HTMLElement, options?: FocusOptions) {
  if (!touched && this.classList.contains('xterm-helper-textarea')) return
  focus.call(this, options)
}

createApp(App).mount('#app')

// Open on the branch whose terminal is blocked on a question.
const BRANCH = '1234-e2e-flake'
const stop = watch(
  () => state.overview,
  (overview) => {
    if (!overview) return
    stop()
    state.selectedBranch = BRANCH
    const there = overview.workspaces.find((w) => w.branch === BRANCH)
    if (there) showTerminal(overview.project.id, BRANCH, there.path)
  },
  { immediate: true },
)

const root = document.documentElement
if ('IntersectionObserver' in window) {
  const watching = new IntersectionObserver((entries) => {
    if (!entries.some((e) => e.isIntersecting)) return
    watching.disconnect()
    startStory()
  })
  watching.observe(root)
} else startStory()
