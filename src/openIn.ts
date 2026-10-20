import { api, type OpenContext, type OpenTarget, type Task } from './api'
import { state } from './store'
import { runInTerminal } from './terminal'
import { terminalActive } from './terminalState'

/**
 * "Open in" targets, for every place that offers them: the workspace cards,
 * the branch menu, and a finished create's buttons.
 *
 * The backend looks the target up and substitutes its placeholders. A
 * `command` target it runs itself; a `terminal` one it hands back, and it is
 * typed into the workspace's built-in terminal here.
 */
export async function openIn(projectId: string, targetId: string, ctx: OpenContext) {
  const command = await api.openIn(targetId, ctx)
  if (command !== null) runInTerminal(projectId, ctx.branch, ctx.path, command)
}

/** A built-in terminal target can't run while that terminal has a shell:
 *  the command would be typed into whatever is running there. */
export function targetBusy(target: OpenTarget, path: string): boolean {
  return target.kind === 'terminal' && terminalActive(path)
}

/** The tooltip for a target's button, saying why it is disabled if it is. */
export function targetTitle(target: OpenTarget, path: string): string | undefined {
  return targetBusy(target, path)
    ? 'A terminal is already running in this workspace'
    : undefined
}

/** A finished create's directory, usable before the list has caught up. */
export async function openTaskIn(targetId: string, task: Task) {
  await openIn(task.projectId, targetId, {
    path: task.path ?? '',
    branch: task.branch ?? '',
    primary: state.config?.projects.find((p) => p.id === task.projectId)?.path ?? '',
    prUrl: task.prUrl ?? '',
  })
}
