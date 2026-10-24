import { Channel } from '@tauri-apps/api/core'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { Terminal, type ITheme } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { computed, nextTick, reactive, watch } from 'vue'
import { api, onTerminalExit, onTerminalStatus, type TerminalMessage } from './api'
import { goToBranch, projectId as currentProject, saveConfig, state } from './store'
import { normalizePath } from './tasks'
import { activeAt, homeOf, running, setStatus, statuses } from './terminalState'

/**
 * Terminals in the detail pane: a shell per workspace, in the backend
 * (`src-tauri/src/terminal.rs`), drawn here by xterm.
 *
 * Two pieces of state, kept apart because they are keyed differently:
 *
 * - **What each branch shows**: its overview, or the terminal of one of its
 *   workspaces. Per project + branch, so switching branches or projects and
 *   back lands where it was left. In memory only.
 * - **The sessions**, one per workspace directory. A session owns its own
 *   DOM: the xterm is `open()`ed into an element once, and that element is
 *   moved between hosts rather than rebuilt, so scrollback, scroll position
 *   and selection survive any amount of switching.
 *
 * The shells themselves outlive the webview: after a reload `syncTerminals`
 * marks the running ones, and the first time one is shown its session
 * attaches to it and gets its recent output replayed.
 */

/** Branch key → path of the workspace whose terminal it shows. Absent means
 *  the overview. */
const shown = reactive(new Map<string, string>())

/** Where each workspace's terminal was last asked for, which is where its
 *  shell is at home when it starts (`TerminalInfo`). One already running
 *  keeps the home it started with. */
const startOn = new Map<string, { projectId: string; branch: string }>()

function branchKey(projectId: string | null, branch: string | null): string | null {
  return projectId && branch ? `${projectId}\0${branch}` : null
}

/** The workspace whose terminal this branch shows, or null for the overview.
 *  Only a wish: the pane falls back to the overview while that workspace isn't
 *  listed on the row, without forgetting it. */
export function terminalFor(projectId: string | null, branch: string | null): string | null {
  const key = branchKey(projectId, branch)
  return (key && shown.get(key)) ?? null
}

export function showTerminal(projectId: string | null, branch: string | null, path: string) {
  const key = branchKey(projectId, branch)
  if (!key || !projectId || !branch) return
  shown.set(key, normalizePath(path))
  startOn.set(normalizePath(path), { projectId, branch })
}

export function showOverview(projectId: string | null, branch: string | null) {
  const key = branchKey(projectId, branch)
  if (key) shown.delete(key)
}

interface Session {
  path: string
  el: HTMLDivElement
  term: Terminal
  fit: FitAddon
  /** `new` until first shown; `exited` waits for Enter to start another. */
  state: 'new' | 'starting' | 'running' | 'exited'
  /** Typed but not yet sent: while the shell starts, or while the previous
   *  write is still in flight. */
  pending: string
  writing: boolean
  disposed: boolean
}

const sessions = new Map<string, Session>()

const dark = window.matchMedia('(prefers-color-scheme: dark)')

/** The pane's own colours, read from the stylesheet so the terminal sits in
 *  it rather than on it. ANSI colours are VS Code's: its light palette is one
 *  of few where yellow and white stay readable on white. */
function theme(): ITheme {
  const css = getComputedStyle(document.documentElement)
  const v = (name: string) => css.getPropertyValue(name).trim()
  const base: ITheme = {
    background: v('--bg-raised'),
    foreground: v('--text'),
    cursor: v('--text'),
    cursorAccent: v('--bg-raised'),
  }
  return dark.matches
    ? {
        ...base,
        selectionBackground: 'rgba(75, 139, 255, 0.35)',
        black: '#000000', red: '#cd3131', green: '#0dbc79', yellow: '#e5e510',
        blue: '#2472c8', magenta: '#bc3fbc', cyan: '#11a8cd', white: '#e5e5e5',
        brightBlack: '#666666', brightRed: '#f14c4c', brightGreen: '#23d18b',
        brightYellow: '#f5f543', brightBlue: '#3b8eea', brightMagenta: '#d670d6',
        brightCyan: '#29b8db', brightWhite: '#e5e5e5',
      }
    : {
        ...base,
        selectionBackground: 'rgba(47, 111, 237, 0.25)',
        black: '#000000', red: '#cd3131', green: '#00bc00', yellow: '#949800',
        blue: '#0451a5', magenta: '#bc05bc', cyan: '#0598bc', white: '#555555',
        brightBlack: '#666666', brightRed: '#cd3131', brightGreen: '#14ce14',
        brightYellow: '#b5ba00', brightBlue: '#0451a5', brightMagenta: '#bc05bc',
        brightCyan: '#0598bc', brightWhite: '#a5a5a5',
      }
}

/** Zoom is the font size, one for every terminal. 12 is 100%. */
export const FONT_SIZE = { min: 8, base: 12, max: 24 }

export const fontSize = computed(() => state.config?.terminalFontSize ?? FONT_SIZE.base)
export const zoomPercent = computed(() => Math.round((fontSize.value / FONT_SIZE.base) * 100))

let saveTimer = 0

/** A point bigger or smaller, or back to 100% with `0`. Written to the config
 *  once the presses stop, rather than once per press. */
export function zoomTerminals(step: 1 | -1 | 0) {
  const config = state.config
  if (!config) return
  const next = step === 0 ? FONT_SIZE.base : fontSize.value + step
  config.terminalFontSize = Math.min(FONT_SIZE.max, Math.max(FONT_SIZE.min, next))
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(() => {
    if (state.config) void saveConfig(JSON.parse(JSON.stringify(state.config)))
  }, 500)
}

/** Whether a terminal is on screen, which is when zoom keys mean it. */
export function terminalShown(): boolean {
  for (const s of sessions.values()) if (s.el.isConnected) return true
  return false
}

// Every session, shown or not. The one on screen refits at once; the others
// refit when next attached, as their element measures zero until then.
watch(fontSize, (size) => {
  for (const s of sessions.values()) {
    s.term.options.fontSize = size
    if (s.el.isConnected && s.el.clientWidth) s.fit.fit()
  }
})

dark.addEventListener('change', () => {
  for (const s of sessions.values()) s.term.options.theme = theme()
})

/**
 * ⌘-clicking a link follows it: an OSC 8 one (Claude Code's `[Image #1]`,
 * once `FORCE_HYPERLINK` tells it to link) or a bare URL in the text. A plain
 * click is left to focusing and selecting, as in iTerm and VS Code. The
 * backend decides what following means, and refuses what it shouldn't
 * follow. Hovering shows where it goes, since an OSC 8 link's text needn't
 * say.
 */
function links(el: HTMLElement) {
  return {
    activate: (event: MouseEvent, url: string) => {
      if (event.metaKey) void api.openLink(url).catch(() => {})
    },
    hover: (_event: MouseEvent, url: string) => void (el.title = `${url}\n⌘-click to open`),
    leave: () => void el.removeAttribute('title'),
  }
}

function session(path: string): Session {
  const key = normalizePath(path)
  const found = sessions.get(key)
  if (found) return found

  const el = document.createElement('div')
  el.className = 'terminal-surface'
  const link = links(el)
  const term = new Terminal({
    fontFamily: "'SF Mono', ui-monospace, Menlo, monospace",
    fontSize: fontSize.value,
    lineHeight: 1.15,
    scrollback: 5000,
    cursorBlink: true,
    // ⌥ types the characters macOS puts on it (€, #, |) instead of acting
    // as Meta, as in Terminal.app by default.
    macOptionIsMeta: false,
    theme: theme(),
    linkHandler: { ...link, allowNonHttpProtocols: true },
  })
  // ⌘↵ is the app's (the attention queue). xterm would send it as a plain
  // Return, so in a Claude Code prompt it would also submit what was typed.
  term.attachCustomKeyEventHandler((event) => !(event.metaKey && event.key === 'Enter'))
  const fit = new FitAddon()
  term.loadAddon(fit)
  term.loadAddon(new WebLinksAddon(link.activate, link))
  const s: Session = {
    path: key,
    el,
    term,
    fit,
    state: 'new',
    pending: '',
    writing: false,
    disposed: false,
  }

  term.onData((data) => {
    if (s.state === 'running' || s.state === 'starting') send(s, data)
    else if (s.state === 'exited' && data === '\r') void start(s)
  })
  term.onResize(({ cols, rows }) => {
    if (s.state === 'running') void api.terminalResize(key, cols, rows).catch(() => {})
  })
  // The element moves between hosts and the pane is resizable; whenever it
  // has a size, the terminal follows it. A detached element measures zero.
  new ResizeObserver(() => {
    if (s.disposed || !el.isConnected || !el.clientWidth || !el.clientHeight) return
    s.fit.fit()
  }).observe(el)

  sessions.set(key, s)
  return s
}

/**
 * One write in flight per terminal; whatever is typed meanwhile goes out as
 * the next one. Separate invokes are not guaranteed to arrive in order, and
 * fast typing over a write-per-keystroke came out scrambled (`$TERM` arrived
 * as `$TREM`).
 */
function send(s: Session, data: string) {
  s.pending += data
  if (s.writing || s.state !== 'running') return
  s.writing = true
  void (async () => {
    while (s.pending && s.state === 'running') {
      const chunk = s.pending
      s.pending = ''
      await api
        .terminalWrite(s.path, chunk)
        .then((at) => void activeAt.set(s.path, at))
        .catch(() => {})
    }
    s.writing = false
  })()
}

async function start(s: Session) {
  s.state = 'starting'
  const channel = new Channel<TerminalMessage>()
  channel.onmessage = (message) => {
    if (s.disposed) return
    if (message instanceof ArrayBuffer) s.term.write(new Uint8Array(message))
    else ended(s, message.exit)
  }
  const at = startOn.get(s.path) ?? running.get(s.path)
  let opened
  try {
    if (!at) throw new Error('No branch to open a terminal on')
    opened = await api.terminalOpen(at.projectId, at.branch, s.path, s.term.cols, s.term.rows, channel)
    // One already running, after a reload, keeps the time it was listed with.
    if (opened.started) activeAt.set(s.path, Date.now())
  } catch (e) {
    if (s.disposed) return
    s.term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`)
    ended(s, null)
    return
  }
  // A shell can be gone before its open even returns.
  if (s.disposed || s.state !== 'starting') return
  s.state = 'running'
  running.set(s.path, homeOf(opened.terminal))
  send(s, '')
}

function ended(s: Session, code: number | null) {
  s.state = 'exited'
  s.pending = ''
  running.delete(s.path)
  statuses.delete(s.path)
  const why = code === null ? 'No shell' : `Shell exited${code ? ` with ${code}` : ''}`
  s.term.write(`\r\n\x1b[2m${why}. Press Return to start a new one.\x1b[0m\r\n`)
}

/** Put a workspace's terminal into `host`, starting its shell the first time,
 *  and give it the keyboard when nothing else is using it. */
export function attach(path: string, host: HTMLElement) {
  const s = session(path)
  host.appendChild(s.el)
  if (!s.term.element) s.term.open(s.el)
  // Sized before the shell starts, so it begins at the right width rather
  // than at 80 columns and then reflows.
  if (s.el.clientWidth && s.el.clientHeight) s.fit.fit()
  if (s.state === 'new') void start(s)
  if (mayTakeFocus()) s.term.focus()
  reportViewing()
}

let viewingSent: string | undefined

/** Tell the backend which terminal is on screen, and whether the window has
 *  focus: together, whether the user is looking at it. A turn that ends
 *  there isn't `ready`, and looking at a ready one is what clears it.
 *  Leaving one whose workspace was removed lets it close. */
function reportViewing(focused = document.hasFocus()) {
  let shown: string | null = null
  for (const s of sessions.values()) if (s.el.isConnected) shown = s.path
  const sent = `${shown}\0${focused}`
  if (sent === viewingSent) return
  viewingSent = sent
  void api.terminalViewing(shown, focused).catch(() => {})
}

/**
 * Showing a terminal — asked for, or come back to by selecting its branch —
 * means carrying on typing in it, without clicking into it first. Not while
 * the keyboard is busy elsewhere: a search being typed, or a dialog up.
 * Typing with nothing focused goes to the branch search instead, so with
 * the terminal focused, ⌘F is the way there.
 */
function mayTakeFocus(): boolean {
  if (document.querySelector('.scrim')) return false
  const active = document.activeElement
  if (!active || active.closest('.xterm')) return true
  return !active.closest('input, textarea, select, [contenteditable]')
}

/**
 * Type `command` into a workspace's terminal and show it, going to its
 * branch first if that isn't on screen. A shell not yet started gets it as
 * typeahead, sent the moment it is; one that exited is started again for it.
 *
 * With `reveal` off the shell starts unseen instead, and the terminal waits
 * to be opened from its card: a create finishing while the user is elsewhere
 * must not pull them over, nor take the keyboard from what they are typing.
 */
export function runInTerminal(
  projectId: string,
  branch: string,
  path: string,
  command: string,
  reveal = true,
) {
  const s = session(path)
  startOn.set(s.path, { projectId, branch })
  if (s.state === 'running') send(s, `${command}\r`)
  else s.pending += `${command}\r`
  // A new one is otherwise started by being shown.
  if (s.state === 'exited' || (!reveal && s.state === 'new')) void start(s)
  if (!reveal) return
  if (currentProject.value !== projectId || state.selectedBranch !== branch) {
    goToBranch(projectId, branch)
  }
  showTerminal(projectId, branch, path)
}

/**
 * Show a workspace's terminal on its branch and give it the keyboard, as
 * asked for outright: ⌘↵, or double-clicking a branch. Even one already on
 * screen takes it back, and a search field that had it lets go. A terminal
 * whose workspace isn't ready yet takes it once the pane shows it (`attach`).
 */
export function enterTerminal(projectId: string, branch: string, path: string) {
  const focused = document.activeElement
  if (focused instanceof HTMLElement && focused.closest('input, textarea')) focused.blur()
  goToBranch(projectId, branch)
  showTerminal(projectId, branch, path)
  void nextTick(() => {
    const s = sessions.get(normalizePath(path))
    if (s?.el.isConnected && mayTakeFocus()) s.term.focus()
  })
}

/** Take a workspace's terminal out of the document, keeping it for later. */
export function detach(path: string) {
  sessions.get(normalizePath(path))?.el.remove()
  reportViewing()
}

/** Hang up on the workspace's shell and forget its session. Every branch
 *  showing it goes back to its overview. */
export function closeTerminal(path: string) {
  const key = normalizePath(path)
  void api.terminalClose(key).catch(() => {})
  const s = sessions.get(key)
  if (s) {
    s.disposed = true
    s.term.dispose()
    s.el.remove()
    sessions.delete(key)
  }
  running.delete(key)
  statuses.delete(key)
  for (const [branch, shownPath] of shown) if (shownPath === key) shown.delete(branch)
  reportViewing()
}

/** Mark the shells that are already running — after a webview reload, all of
 *  them — and follow the ones that exit with nothing attached, and what the
 *  programs in all of them report. */
export async function syncTerminals(): Promise<() => void> {
  const unlisteners = await Promise.all([
    onTerminalExit((path) => {
      const key = normalizePath(path)
      statuses.delete(key)
      // An attached session hears it on its own channel, with the exit code.
      if (!sessions.has(key)) running.delete(key)
    }),
    onTerminalStatus(setStatus),
  ])
  // What a previous webview reported viewing is gone with it.
  viewingSent = undefined
  reportViewing()
  const onFocus = () => reportViewing(true)
  const onBlur = () => reportViewing(false)
  window.addEventListener('focus', onFocus)
  window.addEventListener('blur', onBlur)
  try {
    for (const t of await api.terminalList()) {
      running.set(normalizePath(t.path), homeOf(t))
      setStatus(t)
    }
  } catch {
    /* nothing to mark */
  }
  return () => {
    unlisteners.forEach((u) => u())
    window.removeEventListener('focus', onFocus)
    window.removeEventListener('blur', onBlur)
  }
}
