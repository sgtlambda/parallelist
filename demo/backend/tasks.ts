import type { Task, TaskHeader, TaskKind, TaskLine } from '../../src/api'
import { emit } from '../shims/event'

/** The task registry `tasks.rs` keeps: a title, a status and a log. */

const registry = new Map<string, Task>()
let revision = 0
let counter = 0

const header = ({ log: _log, ...rest }: Task): TaskHeader => rest

function publish(task: Task) {
  task.rev = ++revision
  emit('task', header(task))
}

export function start(spec: {
  projectId: string
  kind: TaskKind
  title: string
  branch?: string
  path?: string
  mode?: 'worktree' | 'clone'
  prUrl?: string | null
  openTargets?: string[]
}): Task {
  const now = Date.now()
  const task: Task = {
    id: `task-${++counter}`,
    projectId: spec.projectId,
    kind: spec.kind,
    title: spec.title,
    status: 'running',
    attention: false,
    createdAt: now,
    startedAt: now,
    finishedAt: null,
    branch: spec.branch ?? null,
    path: spec.path ?? null,
    mode: spec.mode ?? null,
    prUrl: spec.prUrl ?? null,
    openTargets: spec.openTargets ?? [],
    rev: 0,
    log: [],
  }
  registry.set(task.id, task)
  publish(task)
  return task
}

export function line(task: Task, text: string, error = false) {
  const entry: TaskLine = { n: task.log.length, text, error }
  task.log.push(entry)
  emit('task-line', { taskId: task.id, line: entry })
}

export function finish(task: Task, error?: string) {
  if (error) line(task, error, true)
  task.status = error ? 'failed' : 'done'
  task.attention = !!error
  task.finishedAt = Date.now()
  publish(task)
}

export const all = () => [...registry.values()]
export const get = (id: string) => registry.get(id) ?? null

export function setAttention(id: string, attention: boolean) {
  const task = registry.get(id)
  if (!task) return
  task.attention = attention
  publish(task)
}

export const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms))

/** Lines appearing a beat apart, as a command's output does. */
export async function play(task: Task, lines: [string, number][]) {
  for (const [text, wait] of lines) {
    await delay(wait)
    line(task, text)
  }
}
