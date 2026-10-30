import { createWithDefaults, defaultTargets, offeredTargets } from './newWorkspace'
import { openIn } from './openIn'
import { project, projectId, state, type Row } from './store'
import { createTasksFor, isActive, normalizePath, removalOf } from './tasks'
import { enterTerminal } from './terminal'
import { byTerminalRank, latestShown, terminalActive, terminalRank } from './terminalState'

/**
 * Double-clicking a branch: into a shell on it, whatever it takes.
 *
 * - A shell already running there, on screen or not, takes the keyboard:
 *   the one the branch shows, else the one that asks most of the user, else
 *   the one active most recently.
 * - Otherwise a workspace of the branch gets its shell started: the one the
 *   branch showed last, else the first listed. The built-in terminal target
 *   a create would run (ticked by default) runs in it, if there is one.
 * - With no workspace, one is made as the New workspace dialog would make it
 *   with nothing changed (`createWithDefaults`), and its shell opens once it
 *   is set up. A built-in terminal target ticked by default runs in it then,
 *   as after any create. One already being made is waited for instead.
 */
export async function enterShell(row: Row) {
  const id = projectId.value
  if (!id) return
  const branch = row.branch.name
  const shown = latestShown([...row.workspaces, ...row.unlisted].map((w) => w.path))

  const live = [...row.workspaces, ...row.unlisted].map((w) => w.path).filter(terminalActive)
  if (live.length) {
    const path =
      live.find((p) => normalizePath(p) === shown) ??
      live.map((p) => ({ p, rank: terminalRank([p]) })).sort((a, b) => byTerminalRank(a.rank, b.rank))[0].p
    return enterTerminal(id, branch, path)
  }

  // A workspace on its way out is no place to start a shell.
  const usable = row.workspaces.filter((w) => {
    const removal = removalOf(id, w.path)
    return !removal || removal.status === 'failed'
  })
  if (usable.length) {
    const workspace = usable.find((w) => normalizePath(w.path) === shown) ?? usable[0]
    enterTerminal(id, branch, workspace.path)
    const target = offeredTargets(id, row).find(
      (t) => t.kind === 'terminal' && defaultTargets(id).includes(t.id),
    )
    if (!target) return
    try {
      await openIn(id, target.id, {
        path: workspace.path,
        branch: workspace.branch ?? branch,
        primary: project.value?.path ?? '',
        prUrl: row.pr?.url ?? '',
      })
    } catch (e) {
      state.error = `${target.label}: ${String(e)}`
    }
    return
  }
  if (row.workspaces.length) return

  // The pane shows a create's terminal once the create is done.
  const creating = createTasksFor(id, branch).find((t) => isActive(t) && t.path)
  if (creating) return enterTerminal(id, branch, creating.path!)
  const key = `${id}\0${branch}`
  if (starting.has(key)) return
  starting.add(key)
  try {
    const request = await createWithDefaults(id, branch)
    if (request) enterTerminal(id, branch, request.path)
  } finally {
    starting.delete(key)
  }
}

/** Branches whose create is being planned or started, before its task
 *  exists: another double-click meanwhile would ask for a second. */
const starting = new Set<string>()
