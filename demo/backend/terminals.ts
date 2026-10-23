import type { Channel } from '@tauri-apps/api/core'
import type { TerminalInfo, TerminalMessage } from '../../src/api'
import { emit } from '../shims/event'
import { locate, shortHome } from './world'

/**
 * Stand-ins for `terminal.rs`: the shells in the detail pane, and the
 * programs in them, are scripted. A shell understands a handful of commands;
 * `agent` starts a made-up coding-agent session that reports its status the
 * way the real one does over OSC 7501 (here straight to the frontend, which
 * is what the Rust side would have parsed it into), so the marks, the
 * attention queue and the palette have something true to follow.
 */

export const C = {
  r: '\x1b[0m',
  b: '\x1b[1m',
  d: '\x1b[2m',
  orange: '\x1b[38;5;209m',
  green: '\x1b[32m',
  red: '\x1b[31m',
  blue: '\x1b[34m',
  gray: '\x1b[90m',
  accent: '\x1b[38;5;69m',
}

type Mode = 'shell' | 'claude' | 'dev'

interface Step {
  /** Seconds into the turn. */
  at: number
  name: string
  detail: string
  result?: string
  /** What the card in the attention queue says while this one runs. */
  message: string
}

interface Turn {
  verb: string
  steps: Step[]
  /** Seconds into the turn at which it ends. */
  endAt: number
  finish: () => void
  t0: number
  next: number
  timer: number
}

interface Ask {
  options: string[]
  selected: number
  choose: (index: number) => void
}

export interface Session {
  path: string
  projectId: string
  branch: string
  dir: string
  /** Columns, as last reported. */
  cols: number
  mode: Mode
  line: string
  history: string[]
  historyAt: number
  backlog: string
  channel: Channel<TerminalMessage> | null
  status: string | null
  ready: boolean
  activeAt: number
  waitingSince: number | null
  message: string | null
  /** What the turn that ended asked, which Claude Code's transcript would say. */
  question: string | null
  /** The line being redrawn while a turn works. Not in the backlog. */
  spinner: string | null
  turn: Turn | null
  ask: Ask | null
  turns: number
  devTimer: number
  /** Starts what the session was seeded in the middle of, once the demo is
   *  on screen. */
  armed: (() => void) | null
}

const sessions = new Map<string, Session>()
const encoder = new TextEncoder()

/** The terminal on screen in the focused window, as `terminal_viewing` says. */
let viewing: string | null = null

export const get = (path: string) => sessions.get(path)

// ---------------------------------------------------------------- output

export function out(s: Session, text: string, store = true) {
  const data = resolve(text, s.cols).replace(/\r?\n/g, '\r\n')
  if (store) {
    s.backlog += data
    if (s.backlog.length > 150_000) s.backlog = s.backlog.slice(-100_000)
  }
  s.channel?.onmessage(encoder.encode(data).buffer as ArrayBuffer)
}

function clearSpinner(s: Session) {
  if (s.spinner) out(s, '\r\x1b[2K', false)
}

/** Printed above the spinner, which is redrawn under it. */
function print(s: Session, text: string) {
  clearSpinner(s)
  out(s, text)
  if (s.spinner) out(s, s.spinner, false)
}

/**
 * Prose is wrapped when it is drawn, to the width the terminal has then: a
 * pane is as wide as the window leaves it, and xterm would break a long line
 * mid-word with no indent. `fill` marks a stretch of text to wrap, `resolve`
 * wraps the marked stretches, so a seeded backlog can be written as plain
 * strings and still fit.
 */
export const fill = (text: string, first = '  ', rest = '  ') =>
  `\u0001${first}\u0003${rest}\u0003${text}\u0002`

const FILL = /\u0001([^\u0003]*)\u0003([^\u0003]*)\u0003([^\u0002]*)\u0002/g
const visible = (text: string) => text.replace(/\x1b\[[0-9;]*m/g, '')

export function resolve(text: string, cols: number): string {
  const width = Math.max(20, cols - 1)
  return text.replace(FILL, (_, first: string, rest: string, body: string) => {
    const lines: string[] = []
    let line = ''
    for (const word of body.split(/\s+/)) {
      if (!line) line = first + word
      else if (visible(`${line} ${word}`).length > width) {
        lines.push(line)
        line = rest + word
      } else line += ` ${word}`
    }
    lines.push(line)
    return lines.join('\n')
  })
}

/** An assistant message: a bullet, then paragraphs. */
export function say(text: string): string {
  return (
    text
      .split('\n\n')
      .map((p, i) => fill(p, i === 0 ? '● ' : '  ', '  '))
      .join('\n\n') + '\n\n'
  )
}

export function tool(name: string, detail: string, result?: string): string {
  return (
    `${C.green}●${C.r} ${C.b}${name}${C.r}(${C.d}${detail}${C.r})\n` +
    (result ? `  ${C.d}⎿  ${result}${C.r}\n` : '') +
    '\n'
  )
}

export const you = (text: string) => fill(text, `${C.gray}❯${C.r} `, '  ') + '\n\n'

export function banner(dir: string): string {
  return (
    `${C.orange}✻${C.r} ${C.b}agent${C.r}\n\n` +
    `${C.d}  cwd: ${dir}\n  /help for help, /exit to leave${C.r}\n\n`
  )
}

// ---------------------------------------------------------------- status

function info(s: Session): TerminalInfo {
  return {
    path: s.path,
    projectId: s.projectId,
    status: s.status,
    ready: s.ready,
    activeAt: s.activeAt,
    waitingSince: s.waitingSince,
    message: s.message,
  }
}

const publish = (s: Session) => emit('terminal-status', info(s))

/**
 * What the backend does with each status the program reports. `done` while
 * nobody is looking leaves the terminal `ready`; blocked waits whether seen
 * or not; looking at a ready one clears it (`view`).
 */
export function setStatus(s: Session, status: string | null, message: string | null = null) {
  const now = Date.now()
  s.activeAt = now
  s.message = message
  if (status === 'blocked') {
    s.status = 'blocked'
    s.waitingSince ??= now
  } else if (status === 'done') {
    const seen = viewing === s.path
    s.status = seen ? 'idle' : 'done'
    s.ready = !seen
    s.waitingSince = seen ? null : (s.waitingSince ?? now)
  } else {
    s.status = status
    s.ready = false
    s.waitingSince = null
    s.question = null
  }
  publish(s)
}

export function view(path: string | null, focused: boolean) {
  viewing = focused ? path : null
  const s = viewing ? sessions.get(viewing) : null
  if (s?.ready) {
    s.ready = false
    s.status = 'idle'
    s.waitingSince = null
    publish(s)
  }
}

// ---------------------------------------------------------------- sessions

export function create(
  projectId: string,
  path: string,
  seed: Partial<Session> = {},
): Session {
  const found = locate(path)
  const s: Session = {
    path,
    projectId: found?.projectId ?? projectId,
    branch: found?.workspace.branch ?? path.split('/').pop() ?? '',
    dir: shortHome(path),
    cols: 60,
    mode: 'shell',
    line: '',
    history: [],
    historyAt: 0,
    backlog: '',
    channel: null,
    status: null,
    ready: false,
    activeAt: Date.now(),
    waitingSince: null,
    message: null,
    question: null,
    spinner: null,
    turn: null,
    ask: null,
    turns: 0,
    devTimer: 0,
    armed: null,
    ...seed,
  }
  sessions.set(path, s)
  return s
}

export const shellPrompt = (s: Session) => `${C.b}${C.accent}${s.dir}${C.r} ❯ `

function prompt(s: Session) {
  s.line = ''
  s.historyAt = s.history.length
  out(s, s.mode === 'claude' ? `${C.gray}❯${C.r} ` : shellPrompt(s))
}

export function open(
  projectId: string,
  path: string,
  cols: number,
  _rows: number,
  channel: Channel<TerminalMessage>,
): boolean {
  const found = sessions.get(path)
  const s = found ?? create(projectId, path)
  s.channel = channel
  s.cols = cols
  if (!found) {
    out(s, `${C.d}Last login: ${new Date().toDateString()} on ttys001${C.r}\n`)
    prompt(s)
    return true
  }
  // Attaching to a running one replays what it printed.
  // Seeded backlogs are written with plain newlines.
  s.backlog = resolve(s.backlog, cols).replace(/\r?\n/g, '\r\n')
  const replay = s.backlog + (s.spinner ? `\r\x1b[2K${s.spinner}` : '')
  channel.onmessage(encoder.encode(replay).buffer as ArrayBuffer)
  return false
}

export function resize(path: string, cols: number) {
  const s = sessions.get(path)
  if (s) s.cols = cols
}

export function close(path: string) {
  const s = sessions.get(path)
  if (!s) return
  stopTurn(s)
  window.clearInterval(s.devTimer)
  sessions.delete(path)
  if (viewing === path) viewing = null
}

/** The program ends the shell, as `exit` does. */
function exitShell(s: Session) {
  s.channel?.onmessage({ exit: 0 })
  close(s.path)
  emit('terminal-exit', s.path)
}

export function list(): TerminalInfo[] {
  return [...sessions.values()].map(info)
}

export const question = (path: string) => sessions.get(path)?.question ?? null

/** "Mark unseen" and "Mark blocked" in a row's menu, as if the program had
 *  reported them. */
export function mark(path: string, what: 'unseen' | 'blocked') {
  const s = sessions.get(path)
  if (!s) return
  if (what === 'blocked') setStatus(s, 'blocked', s.message)
  else {
    s.ready = true
    s.waitingSince ??= Date.now()
    publish(s)
  }
}

// ---------------------------------------------------------------- input

const KEYS = /\x1b\[[0-9;?]*[A-Za-z~]|\x1b[\s\S]?|[\s\S]/gu

export function write(path: string, data: string): number {
  const s = sessions.get(path)
  if (!s) return Date.now()
  s.activeAt = Date.now()
  for (const key of data.match(KEYS) ?? []) {
    if (!sessions.has(path)) break
    if (s.ask) askKey(s, key)
    else if (s.turn) turnKey(s, key)
    else if (s.mode === 'dev') devKey(s, key)
    else lineKey(s, key)
  }
  return s.activeAt
}

function erase(s: Session, count: number) {
  out(s, '\b \b'.repeat(count))
}

function lineKey(s: Session, key: string) {
  switch (key) {
    case '\r':
      out(s, s.mode === 'claude' ? '\n\n' : '\n')
      return submit(s)
    case '\x7f':
      if (s.line) {
        s.line = s.line.slice(0, -1)
        erase(s, 1)
      }
      return
    case '\x15':
      erase(s, s.line.length)
      s.line = ''
      return
    case '\x03':
      out(s, '^C\n')
      if (s.mode === 'claude') s.line = ''
      return prompt(s)
    case '\x04':
      if (!s.line && s.mode === 'claude') return leaveClaude(s)
      return
    case '\x0c':
      return clearScreen(s)
    case '\x1b[A':
    case '\x1b[B': {
      const next = s.historyAt + (key === '\x1b[A' ? -1 : 1)
      if (next < 0 || next > s.history.length) return
      erase(s, s.line.length)
      s.historyAt = next
      s.line = s.history[next] ?? ''
      out(s, s.line)
      return
    }
    default:
      if (key.length === 1 && key >= ' ') {
        s.line += key
        out(s, key)
      }
  }
}

function clearScreen(s: Session) {
  s.backlog = ''
  out(s, '\x1b[2J\x1b[H\x1b[3J')
  out(s, s.mode === 'claude' ? `${C.gray}❯${C.r} ${s.line}` : shellPrompt(s) + s.line)
}

function submit(s: Session) {
  const text = s.line.trim()
  s.line = ''
  if (text) {
    s.history.push(text)
  }
  s.historyAt = s.history.length
  if (s.mode === 'claude') return text ? claudeInput(s, text) : prompt(s)
  if (!text) return prompt(s)
  shellCommand(s, text)
}

// ---------------------------------------------------------------- the shell

const STATUS_FILES = [
  'src/components/AppHeader.vue',
  'src/components/CartDrawer.vue',
  'src/routes.ts',
  'tests/header.spec.ts',
]

function shellCommand(s: Session, text: string) {
  const [cmd, ...args] = text.split(/\s+/)
  const found = locate(s.path)
  const reply = (body: string) => {
    out(s, body.endsWith('\n') ? body : body + '\n')
    prompt(s)
  }
  switch (cmd) {
    case 'agent':
      return startClaude(s)
    case 'pnpm':
    case 'npm':
      if (args.join(' ').replace('run ', '') === 'dev') return startDev(s)
      return reply(`${C.d}Nothing to run for that in the demo.${C.r}`)
    case 'clear':
      s.backlog = ''
      out(s, '\x1b[2J\x1b[H\x1b[3J')
      return prompt(s)
    case 'pwd':
      return reply(s.path)
    case 'whoami':
      return reply('you')
    case 'date':
      return reply(new Date().toString())
    case 'echo':
      return reply(args.join(' '))
    case 'ls':
      return reply(found?.data.files.join('  ') ?? '')
    case 'git':
      return git(s, args, reply)
    case 'exit':
      return exitShell(s)
    case 'help':
      return reply(
        `${C.d}A scripted shell. Try${C.r} agent${C.d}, ${C.r}pnpm dev${C.d}, ${C.r}git status${C.d}, ${C.r}ls${C.d} or ${C.r}exit${C.d}.${C.r}`,
      )
    default:
      return reply(`zsh: command not found: ${cmd}`)
  }
}

function git(s: Session, args: string[], reply: (body: string) => void) {
  const found = locate(s.path)
  const status = found?.workspace.status
  switch (args[0]) {
    case 'status': {
      const lines = [`On branch ${s.branch}`]
      if (status?.unpushed) {
        lines.push(`Your branch is ahead of 'origin/${s.branch}' by ${status.unpushed} commits.`)
      } else lines.push(`Your branch is up to date with 'origin/${s.branch}'.`)
      lines.push('')
      if (status?.modified) {
        lines.push('Changes not staged for commit:')
        for (const f of STATUS_FILES.slice(0, status.modified)) {
          lines.push(`\t${C.red}modified:   ${f}${C.r}`)
        }
      } else lines.push('nothing to commit, working tree clean')
      return reply(lines.join('\n'))
    }
    case 'branch':
      return reply(`* ${s.branch}\n  main`)
    case 'log':
      return reply(
        `${C.orange}${found?.data.branches.find((b) => b.name === s.branch)?.sha.slice(0, 7) ?? 'a1b2c3d'}${C.r} ${
          found?.data.branches.find((b) => b.name === s.branch)?.subject ?? 'Initial commit'
        }\n${C.orange}9f3e2a1${C.r} Merge pull request #1249 from acme/fix/checkout-tax`,
      )
    default:
      return reply(`${C.d}Only status, branch and log are scripted here.${C.r}`)
  }
}

// ----------------------------------------------------------- a dev server

const HMR = ['src/main.ts', 'src/components/CartDrawer.vue', 'src/styles/app.css', 'src/routes.ts']

export function startDev(s: Session, quiet = false) {
  s.mode = 'dev'
  if (!quiet) {
    out(
      s,
      `\n  ${C.green}${C.b}VITE${C.r} v6.0.3  ${C.d}ready in${C.r} ${C.b}412 ms${C.r}\n\n` +
        `  ${C.green}➜${C.r}  ${C.b}Local${C.r}:   ${C.accent}http://localhost:5173/${C.r}\n` +
        `  ${C.d}➜  press h + enter to show help${C.r}\n`,
    )
  }
  armDev(s)
}

/** A chatty server: output on its own, which never counts as activity. */
export function armDev(s: Session) {
  window.clearInterval(s.devTimer)
  s.devTimer = window.setInterval(() => {
    if (s.mode !== 'dev') return
    const at = new Date().toLocaleTimeString('en-GB')
    out(s, `${C.d}${at}${C.r} ${C.green}[vite]${C.r} hmr update ${HMR[Math.floor(Math.random() * HMR.length)]}\n`)
  }, 6500)
}

function devKey(s: Session, key: string) {
  if (key !== '\x03') return
  window.clearInterval(s.devTimer)
  s.mode = 'shell'
  out(s, '^C\n')
  prompt(s)
}

// ------------------------------------------------------------ Claude Code

export function startClaude(s: Session) {
  s.mode = 'claude'
  s.turns = 0
  out(s, banner(s.dir))
  setStatus(s, 'idle')
  prompt(s)
}

function leaveClaude(s: Session) {
  out(s, `\n${C.d}Goodbye!${C.r}\n`)
  s.mode = 'shell'
  setStatus(s, null)
  prompt(s)
}

function claudeInput(s: Session, text: string) {
  if (text === '/exit' || text === '/quit') return leaveClaude(s)
  if (text.startsWith('/')) {
    out(s, `${C.d}  ⎿  Slash commands aren't part of the demo.${C.r}\n\n`)
    return prompt(s)
  }
  s.turns++
  // The first message of a session ends on a question, to show what that
  // looks like from the queue; the rest end on their own.
  const word =
    (text.match(/[A-Za-z]{5,}/g) ?? ['TODO']).sort((a, b) => b.length - a.length)[0].toLowerCase()
  const lead: Step[] = [
    {
      at: 1,
      name: 'Search',
      detail: `pattern: "${word}"`,
      result: 'Found 6 files',
      message: `Searching for “${word}”`,
    },
    {
      at: 3,
      name: 'Read',
      detail: 'src/routes.ts',
      result: 'Read 118 lines',
      message: 'Reading src/routes.ts',
    },
  ]
  if (s.turns % 2 === 1) {
    runTurn(s, {
      verb: 'Pondering',
      steps: lead,
      endAt: 4.5,
      finish: () => askPermission(s, 'pnpm test', 'Run the test suite'),
    })
  } else {
    runTurn(s, {
      verb: 'Cooking',
      steps: [
        ...lead,
        {
          at: 5,
          name: 'Update',
          detail: 'src/routes.ts',
          result: 'Updated with 6 additions, 2 removals',
          message: 'Editing src/routes.ts',
        },
      ],
      endAt: 7,
      finish: () =>
        done(
          s,
          `That's done on \`${s.branch}\`. The change is in src/routes.ts, and nothing else needed touching.`,
        ),
    })
  }
}

interface TurnSpec {
  verb: string
  steps: Step[]
  endAt: number
  finish: () => void
  /** Seconds already elapsed, for a turn seeded in the middle. */
  elapsed?: number
}

export function runTurn(s: Session, spec: TurnSpec) {
  stopTurn(s)
  const elapsed = spec.elapsed ?? 0
  const upcoming = spec.steps.findIndex((st) => st.at > elapsed)
  s.turn = {
    ...spec,
    t0: Date.now() - elapsed * 1000,
    next: upcoming === -1 ? spec.steps.length : upcoming,
    timer: 0,
  }
  const current = [...spec.steps].reverse().find((st) => st.at <= elapsed)
  setStatus(s, 'working', current?.message ?? `${spec.verb}…`)
  tick(s)
  s.turn.timer = window.setInterval(() => tick(s), 1000)
}

function tick(s: Session) {
  const t = s.turn
  if (!t) return
  const seconds = (Date.now() - t.t0) / 1000
  while (t.next < t.steps.length && t.steps[t.next].at <= seconds) {
    const step = t.steps[t.next++]
    print(s, tool(step.name, step.detail, step.result))
    setStatus(s, 'working', step.message)
  }
  if (seconds >= t.endAt) {
    const finish = t.finish
    stopTurn(s)
    finish()
    return
  }
  const frames = ['✻', '✽', '✶', '✳', '✢']
  s.spinner =
    `${C.orange}${frames[Math.floor(seconds) % frames.length]}${C.r} ${t.verb}… ` +
    `${C.d}(${Math.floor(seconds)}s · esc to interrupt)${C.r}`
  out(s, `\r\x1b[2K${s.spinner}`, false)
}

function stopTurn(s: Session) {
  if (s.turn) window.clearInterval(s.turn.timer)
  s.turn = null
  clearSpinner(s)
  s.spinner = null
}

function turnKey(s: Session, key: string) {
  if (key !== '\x1b' && key !== '\x03') return
  stopTurn(s)
  out(s, `  ${C.red}⎿  Interrupted by user${C.r}\n\n`)
  setStatus(s, 'idle')
  prompt(s)
}

/** What a finished turn asked: the last question, else its last paragraph. */
function lastAsked(text: string): string {
  const paragraphs = text.split('\n\n').map((p) => p.replace(/\s+/g, ' ').trim())
  const asking = [...paragraphs].reverse().find((p) => p.includes('?'))
  if (asking) {
    const sentences = asking.match(/[^.!?]+[.!?]?/g) ?? [asking]
    return [...sentences].reverse().find((x) => x.includes('?'))?.trim() ?? asking
  }
  return paragraphs[paragraphs.length - 1]
}

export function done(s: Session, summary: string) {
  stopTurn(s)
  out(s, say(summary))
  s.question = lastAsked(summary)
  setStatus(s, 'done')
  prompt(s)
}

export function renderOptions(a: Ask): string {
  return a.options
    .map((o, i) =>
      i === a.selected ? `${C.accent} ❯ ${i + 1}. ${o}${C.r}\n` : `   ${i + 1}. ${o}\n`,
    )
    .join('')
}

/** Waits for a choice. Keys are the real prompt's: the digit, or arrows and
 *  Return. */
export function ask(
  s: Session,
  heading: string,
  options: string[],
  choose: (index: number) => void,
  question: string,
  message: string,
) {
  stopTurn(s)
  s.ask = { options, selected: 0, choose }
  out(s, `${heading}\n${renderOptions(s.ask)}`)
  s.question = question
  setStatus(s, 'blocked', message)
}

function askKey(s: Session, key: string) {
  const a = s.ask
  if (!a) return
  let pick = -1
  if (key === '\r') pick = a.selected
  else if (/^[1-9]$/.test(key) && Number(key) <= a.options.length) pick = Number(key) - 1
  else if (key === '\x1b[A' || key === '\x1b[B') {
    const next = a.selected + (key === '\x1b[A' ? -1 : 1)
    if (next < 0 || next >= a.options.length) return
    a.selected = next
    out(s, `\x1b[${a.options.length}A\r\x1b[J${renderOptions(a)}`)
    return
  }
  if (pick < 0) return
  s.ask = null
  out(s, `\n  ${C.d}⎿  ${a.options[pick]}${C.r}\n\n`)
  a.choose(pick)
}

export function askPermission(s: Session, command: string, why: string) {
  ask(
    s,
    ` ${C.b}Bash command${C.r}\n\n   ${command}\n   ${C.d}${why}${C.r}\n\n Do you want to proceed?`,
    ['Yes', "Yes, and don't ask again", 'No, and say what to do instead'],
    (choice) => {
      if (choice === 2) {
        out(s, `  ${C.red}⎿  Stopped${C.r}\n\n`)
        setStatus(s, 'idle')
        return prompt(s)
      }
      runTurn(s, {
        verb: 'Running',
        steps: [
          { at: 1, name: 'Bash', detail: command, result: '41 passed', message: `Running ${command}` },
        ],
        endAt: 3.5,
        finish: () =>
          done(s, `The tests pass, so the change on \`${s.branch}\` is ready for you to look at.`),
      })
    },
    `Do you want to run \`${command}\`?`,
    `Run ${command}?`,
  )
}

// ----------------------------------------------------------------- story

/** Called once the demo scrolls into view: what was seeded mid-turn carries
 *  on, so the queue changes while the visitor is looking. */
export function startStory() {
  for (const s of sessions.values()) {
    s.armed?.()
    s.armed = null
  }
}
