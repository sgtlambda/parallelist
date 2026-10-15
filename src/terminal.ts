import { Channel } from '@tauri-apps/api/core'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal, type ITheme } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'
import { computed, reactive, watch } from 'vue'
import {
  api,
  onTerminalExit,
  onTerminalStatus,
  type TerminalInfo,
  type TerminalMessage,
} from './api'
import { goToBranch, projectId as currentProject, saveConfig, state } from './store'
import { normalizePath } from './tasks'

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
  if (!key || !projectId) return
  shown.set(key, normalizePath(path))
  projectOf.set(normalizePath(path), projectId)
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

/** Which project each workspace's terminal belongs to, so the sidebar can
 *  mark a project without knowing its workspaces. */
const projectOf = new Map<string, string>()

/** Workspaces with a running shell: what the project list, branch list and
 *  cards mark. */
const active = reactive(new Set<string>())

/** What the program in each terminal reports over OSC 7501, when it does,
 *  and whether a turn ended there unseen (`ready`, kept by the backend). */
const statuses = reactive(new Map<string, Pick<TerminalInfo, 'status' | 'ready'>>())

export function terminalActive(path: string): boolean {
  return active.has(normalizePath(path))
}

export interface TerminalMark {
  /** Blinking while Claude Code, or anything speaking OSC 7501, works. */
  icon: 'terminal' | 'terminal-working'
  /** Something waits for the user: `blocked` on an answer (orange), or
   *  `ready`, a turn finished out of sight (blue). */
  box: 'blocked' | 'ready' | null
  title: string
}

/**
 * The mark for the terminals of some workspaces: none unless one has a
 * running shell. Several terminals combine part by part: the icon blinks if
 * any of them works, and the box is orange if any is blocked, else blue if
 * any is ready. The one place it is decided, for the project list, the
 * branch rows, the cards and the palette alike.
 */
export function terminalMark(paths: string[]): TerminalMark | null {
  const keys = paths.map(normalizePath).filter((p) => active.has(p))
  if (!keys.length) return null
  const reported = keys.map((k) => statuses.get(k))
  const working = reported.some((r) => r?.status === 'working')
  const blocked = reported.some((r) => r?.status === 'blocked')
  const ready = reported.some((r) => r?.ready)
  const title = [
    blocked && 'Blocked — waiting for your answer',
    ready && 'Ready — finished while you were away',
    working && 'Working',
  ].filter((t) => !!t)
  return {
    icon: working ? 'terminal-working' : 'terminal',
    box: blocked ? 'blocked' : ready ? 'ready' : null,
    title: title.join('\n') || 'Active terminal',
  }
}

export function projectTerminalMark(projectId: string): TerminalMark | null {
  return terminalMark([...active].filter((path) => projectOf.get(path) === projectId))
}

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

function session(path: string): Session {
  const key = normalizePath(path)
  const found = sessions.get(key)
  if (found) return found

  const el = document.createElement('div')
  el.className = 'terminal-surface'
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
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
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
      await api.terminalWrite(s.path, chunk).catch(() => {})
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
  const projectId = projectOf.get(s.path) ?? ''
  try {
    await api.terminalOpen(projectId, s.path, s.term.cols, s.term.rows, channel)
  } catch (e) {
    if (s.disposed) return
    s.term.write(`\r\n\x1b[31m${String(e)}\x1b[0m\r\n`)
    ended(s, null)
    return
  }
  // A shell can be gone before its open even returns.
  if (s.disposed || s.state !== 'starting') return
  s.state = 'running'
  active.add(s.path)
  send(s, '')
}

function ended(s: Session, code: number | null) {
  s.state = 'exited'
  s.pending = ''
  active.delete(s.path)
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

let viewingSent: string | null | undefined

/** Tell the backend which terminal the user is looking at: the one on
 *  screen, while the window has focus. A turn that ends there isn't
 *  `ready`, and looking at a ready one is what clears it. */
function reportViewing(focused = document.hasFocus()) {
  let viewing: string | null = null
  if (focused) for (const s of sessions.values()) if (s.el.isConnected) viewing = s.path
  if (viewing === viewingSent) return
  viewingSent = viewing
  void api.terminalViewing(viewing).catch(() => {})
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
 */
export function runInTerminal(projectId: string, branch: string, path: string, command: string) {
  const s = session(path)
  projectOf.set(s.path, projectId)
  if (s.state === 'running') send(s, `${command}\r`)
  else s.pending += `${command}\r`
  if (s.state === 'exited') void start(s)
  if (currentProject.value !== projectId || state.selectedBranch !== branch) {
    goToBranch(projectId, branch)
  }
  showTerminal(projectId, branch, path)
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
  active.delete(key)
  statuses.delete(key)
  for (const [branch, shownPath] of shown) if (shownPath === key) shown.delete(branch)
  reportViewing()
}

function setStatus(t: TerminalInfo) {
  const key = normalizePath(t.path)
  if (t.status || t.ready) statuses.set(key, { status: t.status, ready: t.ready })
  else statuses.delete(key)
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
      if (!sessions.has(key)) active.delete(key)
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
      const key = normalizePath(t.path)
      projectOf.set(key, t.projectId)
      active.add(key)
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
