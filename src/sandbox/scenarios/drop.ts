import App from '../../App.vue'
import { showTerminal } from '../../terminal'
import TerminalDrop from '../views/TerminalDrop.vue'
import { backend } from '../backend'
import { boot, terminals } from '../seed'
import { defineGroup } from '../define'

const PATH = '/Users/sandbox/Code/app-1234-fix-header'
const SHOT = '/Users/sandbox/Desktop/Screenshot 2026-10-26 at 14.02.11.png'
const FOLDER = '/Users/sandbox/Code/api'

/** Over the branch list's stand-in, and over the terminal, in CSS pixels of
 *  a 1000×600 frame. */
const LEFT = { x: 140, y: 260 }
const RIGHT = { x: 600, y: 260 }

/**
 * A shell in the terminal (unless `shell` is off), and `paths` dragged from
 * Finder to `at`; with `drop`, let go there too. The shell has no pty
 * behind it: it echoes what it is sent, as one in a terminal would.
 */
async function drag(paths: string[], at: { x: number; y: number }, options: { drop?: boolean; shell?: boolean } = {}) {
  await boot({ selected: 'fix/1234-header' })
  if (options.shell !== false) terminals([{ path: PATH, branch: 'fix/1234-header' }])
  let echo = (_: string) => {}
  backend.handle({
    terminal_open: ({ channel, path }: { channel: { onmessage: (m: unknown) => void }; path: string }) => {
      echo = (text) => channel.onmessage(new TextEncoder().encode(text).buffer)
      setTimeout(() => echo('app-1234-fix-header % '), 0)
      return { started: false, terminal: { path, projectId: 'app', branch: 'fix/1234-header', kind: 'worktree' } }
    },
    terminal_resize: () => undefined,
    terminal_write: ({ data }: { data: string }) => {
      echo(data)
      return Date.now()
    },
    are_folders: ({ paths: dragged }: { paths: string[] }) => dragged.map((p) => !p.includes('.')),
    open_folder: () => {
      throw new Error('The sandbox opens no folders')
    },
  })
  // Setup runs before the mount, and the areas listen once mounted.
  setTimeout(() => backend.emit('drag-drop', { type: 'enter', paths, position: at }), 800)
  if (options.drop) setTimeout(() => backend.emit('drag-drop', { type: 'drop', paths, position: at }), 1000)
}

/**
 * The whole window, with a terminal open on `fix/1234-header` and another
 * waiting in the queue, and `paths` dragged in from Finder. The drag then
 * follows the mouse, so the areas can be crossed by hand; Escape ends it.
 * Nothing can really be dropped: a browser hands the page no paths.
 */
async function dragOverApp(paths: string[]) {
  await drag(paths, LEFT)
  backend.handle({ set_create_menu: () => undefined })
  terminals([
    { path: PATH, branch: 'fix/1234-header' },
    { path: '/Users/sandbox/Code/app-b', branch: 'feature/sso-login', ready: true, minutesAgo: 4 },
  ])
  showTerminal('app', 'fix/1234-header', PATH)
  const over = (e: MouseEvent) =>
    backend.emit('drag-drop', { type: 'over', position: { x: e.clientX, y: e.clientY } })
  window.addEventListener('mousemove', over)
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return
    window.removeEventListener('mousemove', over)
    backend.emit('drag-drop', { type: 'leave' })
  })
}

const withShell = { path: PATH }

export default defineGroup('Drop', [
  {
    id: 'app-folder',
    title: 'A folder over the window',
    note: 'The app itself, with a folder dragged in. Move the mouse to cross the areas; Escape ends the drag.',
    component: App,
    setup: () => dragOverApp([FOLDER]),
  },
  {
    id: 'app-file',
    title: 'A file over the window',
    note: 'Only the terminal takes it. Move the mouse over it; Escape ends the drag.',
    component: App,
    setup: () => dragOverApp([SHOT]),
  },
  {
    id: 'folder-left',
    title: 'A folder over the branch list',
    note: 'Two areas down to the queue: the project and branch columns, and the terminal. The one under the pointer is filled.',
    component: TerminalDrop,
    props: withShell,
    setup: () => drag([FOLDER], LEFT),
  },
  {
    id: 'folder-terminal',
    title: 'A folder over the terminal',
    note: 'Letting go here pastes its path instead of opening it.',
    component: TerminalDrop,
    props: withShell,
    setup: () => drag([FOLDER], RIGHT),
  },
  {
    id: 'folder-no-terminal',
    title: 'A folder, with no terminal on screen',
    note: 'The projects area takes the whole width, still stopping at the queue.',
    component: TerminalDrop,
    setup: () => drag([FOLDER], RIGHT, { shell: false }),
  },
  {
    id: 'file-outside',
    title: 'A file outside the terminal',
    note: 'Files only go to a terminal; let go here and nothing happens.',
    component: TerminalDrop,
    props: withShell,
    setup: () => drag([SHOT], LEFT),
  },
  {
    id: 'file-terminal',
    title: 'A file over the terminal',
    component: TerminalDrop,
    props: withShell,
    setup: () => drag([SHOT], RIGHT),
  },
  {
    id: 'dropped',
    title: 'A folder and two files dropped on the terminal',
    note: 'What the shell receives: backslash-escaped as Terminal.app types it, as one bracketed paste. The log has the terminal_write.',
    component: TerminalDrop,
    props: withShell,
    setup: () => drag([FOLDER, SHOT, "/Users/sandbox/Downloads/notes (draft) & Jan's.md"], RIGHT, { drop: true }),
  },
])
