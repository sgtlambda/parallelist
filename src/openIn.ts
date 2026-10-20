import { api, type OpenContext, type OpenTarget, type Task } from './api'
import { openTargetsFor, state } from './store'
import { runInTerminal } from './terminal'
import { terminalActive } from './terminalState'

/**
 * "Open in" targets, for every place that offers them: the workspace cards,
 * the branch menu, and a finished create's buttons.
 *
 * The backend looks the target up and substitutes its placeholders. A
 * `command` target it runs itself; a `terminal` one it hands back, and it is
 * typed into the workspace's built-in terminal here, shown unless `reveal`
 * is off.
 */
export async function openIn(
  projectId: string,
  targetId: string,
  ctx: OpenContext,
  reveal = true,
) {
  const command = await api.openIn(targetId, ctx)
  if (command !== null) runInTerminal(projectId, ctx.branch, ctx.path, command, reveal)
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
export async function openTaskIn(targetId: string, task: Task, reveal = true) {
  await openIn(
    task.projectId,
    targetId,
    {
      path: task.path ?? '',
      branch: task.branch ?? '',
      primary: state.config?.projects.find((p) => p.id === task.projectId)?.path ?? '',
      prUrl: task.prUrl ?? '',
    },
    reveal,
  )
}

/**
 * Run the targets ticked in the New workspace dialog, now that the create is
 * done. A terminal one is shown only if the user is still on that branch.
 * One that has meanwhile gone from the config, or became busy, is skipped
 * rather than failing the rest.
 */
export async function runCreateTargets(task: Task) {
  if (!task.openTargets.length) return
  const reveal =
    state.config?.selectedProjectId === task.projectId && state.selectedBranch === task.branch
  const known = openTargetsFor(task.projectId)
  for (const id of task.openTargets) {
    const target = known.find((t) => t.id === id)
    if (!target || targetBusy(target, task.path ?? '')) continue
    try {
      await openTaskIn(id, task, reveal)
    } catch (e) {
      state.error = `${target.label}: ${String(e)}`
    }
  }
}
