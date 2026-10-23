import TaskBox from '../../components/TaskBox.vue'
import TasksDialog from '../../components/TasksDialog.vue'
import { backend } from '../backend'
import { boot } from '../seed'
import { defineGroup } from '../define'
import { installLog, line, task } from '../fixtures'
import { STEP_MARK, openTasks, startTaskSync } from '../../tasks'
import { state } from '../../store'
import type { Task, TaskHeader } from '../../api'

/** The app, with these tasks in the registry, as if listed at startup. */
async function withTasks(...list: Task[]) {
  await boot({ selected: 'main' })
  state.now = Date.now()
  backend.handle({ list_tasks: () => structuredClone(list) })
  await startTaskSync(() => {})
}

const header = ({ log: _log, ...rest }: Task): TaskHeader => rest

const creating = (overrides: Partial<Task> = {}) =>
  task({
    kind: 'create',
    title: 'Create fix/1234-header',
    status: 'running',
    branch: 'fix/1234-header',
    path: '/Users/sandbox/Code/app-1234-fix-header',
    mode: 'worktree',
    log: installLog,
    ...overrides,
  })

export default defineGroup('Task box', [
  {
    id: 'running',
    title: 'One running task',
    component: TaskBox,
    setup: () => withTasks(creating()),
  },
  {
    id: 'queued',
    title: 'Running, with others queued (+N)',
    note: 'The oldest running task is featured; the rest are only counted.',
    component: TaskBox,
    setup: () =>
      withTasks(
        creating(),
        task({ kind: 'remove', title: 'Remove app-billing', status: 'queued' }),
        task({ kind: 'remove', title: 'Remove app-search', status: 'queued' }),
      ),
  },
  {
    id: 'failed',
    title: 'Failure takes the box',
    note: 'A failure outranks a running task and stays until dismissed.',
    component: TaskBox,
    setup: () =>
      withTasks(
        creating(),
        task({
          kind: 'remove',
          title: 'Remove app-b',
          status: 'failed',
          attention: true,
          log: [line(0, `${STEP_MARK}Removing the clone`), line(1, 'rm: app-b/vendor: Operation not permitted', true)],
        }),
      ),
  },
  {
    id: 'ready',
    title: 'Ready: a create that finished out of sight',
    note: 'Offers the "Open in" targets, minus any that need a pull request.',
    component: TaskBox,
    setup: () => withTasks(creating({ status: 'done', attention: true })),
  },
  {
    id: 'live',
    title: 'Live: a create that runs, then finishes',
    note: 'Lines and the final header arrive as events, the way the backend sends them. Reset to run it again.',
    component: TaskBox,
    async setup() {
      const running = creating({ log: [] })
      await withTasks(running)
      let n = 0
      const lines = installLog.map((l) => l.text)
      const tick = setInterval(() => {
        if (n < lines.length) {
          backend.emit('task-line', { taskId: running.id, line: line(n, lines[n]) })
          n++
          return
        }
        clearInterval(tick)
        backend.emit('task', { ...header(running), status: 'done', finishedAt: Date.now(), rev: running.rev + 1 })
      }, 700)
    },
  },
  {
    id: 'dialog',
    title: 'Tasks dialog: log of a failed create',
    component: TasksDialog,
    async setup() {
      const failed = task({
        kind: 'create',
        title: 'Create feature/search-index',
        status: 'failed',
        attention: true,
        branch: 'feature/search-index',
        mode: 'worktree',
        log: [...installLog.slice(0, 5), line(5, 'error: pnpm install exited with 1', true)],
      })
      await withTasks(failed, creating({ title: 'Create fix/1234-header' }), task({ kind: 'refresh', title: 'Refresh app', status: 'done' }))
      openTasks(failed.id)
    },
  },
])
