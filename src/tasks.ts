import { computed, markRaw, reactive, ref } from 'vue'
import { api, onTaskEvents, type Task, type TaskHeader, type TaskLine } from './api'
import { WORKSPACE_TITLES } from './format'
import { projectId, refresh, selectProject, selectedRow, state } from './store'

/**
 * The window's view of the backend's task registry: refreshes, creates,
 * removals and branch deletions. The backend owns the tasks — they outlive a
 * webview reload — and this mirrors them from one listing plus the events
 * that follow it. Tasks cannot be cancelled.
 */

/** The bound the backend keeps, so both sides trim the same lines. */
const MAX_LINES = 5000

/** How long a task that worked stays in the box after finishing: long
 *  enough for its checkmark to draw in and be seen, then it lifts away. */
const LINGER_MS = 1200
/** How long the box keeps a task that asks for attention. It stays flagged
 *  (cards, the dialog) after that; the box just stops showing it. */
const ATTENTION_MS = 10_000
/** The "Ready" notice matters less than a failure, so it goes sooner. */
const READY_MS = 6_000

export interface LiveTask extends Task {
  /** The most recently written line, rewritten or new. For a refresh that is
   *  the counter ticking, which is not necessarily the last line. */
  lastLine: string
}

export const tasks = ref<LiveTask[]>([])

/** Finished creates still being checked against a refresh. They stay in the
 *  box until that settles, so a create that turns out to need a "Ready"
 *  notice doesn't vanish and reappear. */
const checking = reactive(new Set<string>())

/** Whether the task dialog is open, and on which task. */
export const tasksOpen = ref(false)
export const selectedTaskId = ref<string | null>(null)

export function openTasks(id: string | null = null) {
  selectedTaskId.value = id ?? boxTasks.value[0]?.id ?? tasks.value[tasks.value.length - 1]?.id ?? null
  tasksOpen.value = true
}

export function taskById(id: string | null): LiveTask | null {
  return tasks.value.find((t) => t.id === id) ?? null
}

function live(task: Task): LiveTask {
  task.log.forEach(markRaw)
  const last = [...task.log].reverse().find((l) => l.text.trim())
  return reactive({ ...task, lastLine: last?.text ?? '' }) as LiveTask
}

/** Called with each create that finishes while the window is open. Handed
 *  in by `startTaskSync` rather than imported: what it runs reaches
 *  `terminal.ts`, which imports the store, which imports this file. */
let onCreated: (task: LiveTask) => void = () => {}

/**
 * Subscribe, then list. Events that land before the listing are held and
 * replayed over it: revisions make a stale header a no-op, and line numbers
 * make a repeated line harmless.
 */
export async function startTaskSync(created: (task: LiveTask) => void): Promise<() => void> {
  onCreated = created
  let listed = false
  const held: (() => void)[] = []
  const whenListed = (apply: () => void) => (listed ? apply() : held.push(apply))

  const unlisten = await onTaskEvents({
    header: (header) => whenListed(() => applyHeader(header)),
    line: (id, line) => whenListed(() => applyLine(id, line)),
    removed: (id) => whenListed(() => (tasks.value = tasks.value.filter((t) => t.id !== id))),
  })

  tasks.value = (await api.listTasks()).map(live)
  listed = true
  held.splice(0).forEach((apply) => apply())
  return unlisten
}

function applyHeader(header: TaskHeader) {
  const task = taskById(header.id)
  if (!task) {
    tasks.value.push(live({ ...header, log: [] }))
    return
  }
  if (header.rev < task.rev) return
  const was = task.status
  Object.assign(task, header)
  if (was !== task.status && (task.status === 'done' || task.status === 'failed')) {
    void settle(task)
  }
}

function applyLine(id: string, line: TaskLine) {
  const task = taskById(id)
  if (!task) return
  const log = task.log
  const last = log.length ? log[log.length - 1].n : -1
  if (line.n === last + 1) {
    log.push(markRaw(line))
    if (log.length > MAX_LINES) log.splice(0, log.length - MAX_LINES)
  } else if (line.n <= last) {
    const index = line.n - log[0].n
    if (index < 0) return
    log[index] = markRaw(line)
  } else {
    // A gap: something was missed. Take the backend's copy.
    void resync(id)
    return
  }
  if (line.text.trim()) task.lastLine = line.text
}

async function resync(id: string) {
  const fresh = await api.getTask(id)
  const task = taskById(id)
  if (!fresh || !task || fresh.rev < task.rev) return
  Object.assign(task, live(fresh))
}

/**
 * After a task that changes what is on disk: refresh, so the list agrees with
 * it. A finished create runs the targets its dialog ticked, and if its real
 * card is then not on screen asks for attention — the box's "Ready" notice.
 */
async function settle(task: LiveTask) {
  if (task.kind === 'refresh') return
  const created = task.kind === 'create' && task.status === 'done'
  if (created) {
    checking.add(task.id)
    // Not after the refresh: that takes seconds, and the terminal waits
    // for its workspace to be listed anyway.
    onCreated(task)
  }
  try {
    // Only the project on screen can be refreshed; another picks it up when
    // selected.
    if (projectId.value === task.projectId) await refresh()
    if (created && !onScreen(task)) {
      // Locally first: the confirming event can trail the command's reply,
      // and the box would drop the task for that moment.
      task.attention = true
      await api.setTaskAttention(task.id, true)
    }
  } finally {
    checking.delete(task.id)
  }
}

/** Where the real card would be. Only when the workspace is actually listed
 *  there — a refresh that missed it must still produce the notice. */
function onScreen(task: LiveTask): boolean {
  const target = normalizePath(task.path ?? '')
  return (
    projectId.value === task.projectId &&
    state.selectedBranch === task.branch &&
    (selectedRow.value?.workspaces.some((w) => normalizePath(w.path) === target) ?? false)
  )
}

/** Clear a task's claim on attention. It stays in the history. */
export async function dismissTask(id: string) {
  const task = taskById(id)
  if (task) task.attention = false
  await api.setTaskAttention(id, false)
}

export function isActive(task: Task): boolean {
  return task.status === 'queued' || task.status === 'running'
}

const oldest = (key: (t: LiveTask) => number) => (a: LiveTask, b: LiveTask) => key(a) - key(b)

/**
 * Everything the box has to show, the featured task first: tasks asking for
 * attention (for `ATTENTION_MS`, then they leave the box), so a failure can't
 * hide behind a long install; then running,
 * then queued, each oldest first; then tasks that just finished.
 */
export const boxTasks = computed<LiveTask[]>(() => {
  const all = tasks.value
  const attention = all
    .filter(
      (t) =>
        t.attention &&
        state.now - (t.finishedAt ?? t.createdAt) < (t.status === 'failed' ? ATTENTION_MS : READY_MS),
    )
    .sort(oldest((t) => t.finishedAt ?? t.createdAt))
  const running = all
    .filter((t) => t.status === 'running' && !t.attention)
    .sort(oldest((t) => t.startedAt ?? t.createdAt))
  const queued = all.filter((t) => t.status === 'queued').sort(oldest((t) => t.createdAt))
  const recent = all
    .filter(
      (t) =>
        t.status === 'done' &&
        !t.attention &&
        (checking.has(t.id) || state.now - (t.finishedAt ?? 0) < LINGER_MS),
    )
    .sort(oldest((t) => -(t.finishedAt ?? 0)))
  return [...attention, ...running, ...queued, ...recent]
})

/** The dialog's list, in three groups. */
export const taskGroups = computed(() => {
  const all = tasks.value
  const newest = oldest((t: LiveTask) => -(t.finishedAt ?? t.createdAt))
  return [
    { label: 'Needs attention', tasks: all.filter((t) => t.attention).sort(newest) },
    {
      label: 'Running',
      tasks: all.filter((t) => isActive(t) && !t.attention).sort(oldest((t) => t.createdAt)),
    },
    { label: 'Recent', tasks: all.filter((t) => !isActive(t) && !t.attention).sort(newest) },
  ].filter((group) => group.tasks.length)
})

function projectName(id: string): string {
  return state.config?.projects.find((p) => p.id === id)?.name ?? ''
}

/** The line under a task's title: what it is doing now, or how it ended. */
export function taskLine(task: LiveTask): string {
  if (task.status === 'queued') return 'Queued'
  if (task.kind === 'create' && task.status === 'done') {
    return `Ready · ${WORKSPACE_TITLES[task.mode ?? 'worktree']} in ${projectName(task.projectId)}`
  }
  const text = task.lastLine.replace(STEP_MARK, '').trim()
  if (text) return text
  return task.status === 'running' ? 'Starting…' : 'Done'
}

export function elapsed(task: Task): string {
  const from = task.startedAt ?? task.createdAt
  const to = task.finishedAt ?? state.now
  const total = Math.max(0, Math.floor((to - from) / 1000))
  const minutes = Math.floor(total / 60)
  const seconds = total % 60
  return minutes ? `${minutes}m ${String(seconds).padStart(2, '0')}s` : `${seconds}s`
}

/** Paths compared as the directory listing reports them: no trailing slash. */
export function normalizePath(path: string): string {
  return path.replace(/\/+$/, '')
}

/** The step marker the backend puts in front of each phase and setup step. */
export const STEP_MARK = '▸ '

/** The step a task is on, and the most recent line that step produced. Read
 *  from the tail: an install step can log thousands of lines, and this runs
 *  on every one of them. */
export function taskProgress(task: Task): { step: string; line: string } {
  let line = ''
  for (let i = task.log.length - 1; i >= 0; i--) {
    const text = task.log[i].text
    if (text.startsWith(STEP_MARK)) {
      return { step: text.slice(STEP_MARK.length), line }
    }
    if (!line && text.trim()) line = text.trim()
  }
  return { step: '', line }
}

/** Create tasks for a branch, whatever their state. The detail pane decides
 *  which still need a card. */
export function createTasksFor(project: string | null, branch: string | null): LiveTask[] {
  if (!project || !branch) return []
  return tasks.value.filter(
    (t) => t.kind === 'create' && t.projectId === project && t.branch === branch,
  )
}

/**
 * Whether a create task still has a card of its own, beside or instead of the
 * real one. Running or queued; failed until dismissed; done until a refresh
 * that began after it finished — that refresh has the real workspace if there
 * is one, and without the cutoff a create from earlier in the session would
 * come back as a "ready" card once its workspace was removed.
 */
export function hasCard(task: Task): boolean {
  if (isActive(task)) return true
  if (task.status === 'failed') return task.attention
  return (task.finishedAt ?? 0) >= state.overviewAt
}

/** True while this branch has a create card that is not a real workspace
 *  yet — the branch may not exist in any ref. A refresh must not drop the
 *  selection out from under it. */
export function hasPendingCreate(project: string | null, branch: string | null): boolean {
  return createTasksFor(project, branch).some((t) => t.status !== 'done' && hasCard(t))
}

/** The removal task a listed workspace is subject to, if any: queued,
 *  running, failed and not yet dismissed, or done and still listed only
 *  because the list predates it. */
export function removalOf(project: string | null, path: string): LiveTask | null {
  const target = normalizePath(path)
  const matching = tasks.value.filter(
    (t) =>
      t.kind === 'remove' &&
      t.projectId === project &&
      normalizePath(t.path ?? '') === target &&
      hasCard(t),
  )
  return matching[matching.length - 1] ?? null
}

/** Put a finished create's real card on screen: its project, then its
 *  branch. */
export async function revealTask(task: Task) {
  if (state.config?.selectedProjectId !== task.projectId) {
    await selectProject(task.projectId)
  }
  state.selectedBranch = task.branch
}

/** A finished create whose card is off screen: the box's "Ready" notice. */
export function isReady(task: Task): boolean {
  return task.kind === 'create' && task.status === 'done' && task.attention
}
