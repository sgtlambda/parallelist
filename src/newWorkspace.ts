import { api, type CreateRequest, type OpenTarget } from './api'
import { openTargetsFor, rows, selectBranch, state, type Row } from './store'

/**
 * How a new workspace is made from a few choices: what the New workspace
 * dialog starts with, and what it sends. Double-clicking a branch with no
 * workspace makes one exactly as the dialog would if it were opened on that
 * branch and submitted at once, so both go through here and can't drift.
 */

/** Setup steps ticked to begin with. */
export function defaultSteps(projectId: string | null): string[] {
  const project = state.config?.projects.find((p) => p.id === projectId)
  return (project?.setupSteps ?? []).filter((s) => s.defaultEnabled).map((s) => s.id)
}

/** "Open in" targets ticked to begin with: those set to run on create, but
 *  at most one built-in terminal target. */
export function defaultTargets(projectId: string | null): string[] {
  const ticked: string[] = []
  for (const t of openTargetsFor(projectId)) {
    if (t.runOnCreate && !ticked.some((id) => targetsClash(projectId, t, id))) ticked.push(t.id)
  }
  return ticked
}

/** At most one built-in terminal target is ticked: a second would be typed
 *  into whatever the first started. */
export function targetsClash(projectId: string | null, target: OpenTarget, id: string): boolean {
  return (
    target.kind === 'terminal' &&
    openTargetsFor(projectId).find((t) => t.id === id)?.kind === 'terminal'
  )
}

/** "Open in" targets on offer for a branch. One that needs a PR only when
 *  the branch has one. */
export function offeredTargets(projectId: string | null, row: Row | null): OpenTarget[] {
  return openTargetsFor(projectId).filter((t) => !t.requiresPr || row?.pr)
}

/** The row for a branch of the project on screen, if it has one: a branch
 *  that doesn't exist yet has none, and so no PR. */
export function rowFor(branch: string): Row | null {
  return rows.value.find((r) => r.branch.name === branch) ?? null
}

/** The branch a new branch is cut from. */
export function baseBranch(): string {
  return state.overview?.baseBranch ?? 'main'
}

/** A branch no ref has has to be created, not just placed in a workspace. */
export function isNewBranch(branch: string): boolean {
  return branch.length > 0 && !(state.overview?.branches ?? []).some((b) => b.name === branch)
}

/** Where a workspace for `branch` would go, and as which kind: a clone when
 *  git won't put the branch in another worktree. `number` leads the
 *  directory name. */
export async function planWorkspace(projectId: string, branch: string, number: number | null) {
  const plan = await api.planWorkspace(projectId, branch, number)
  return {
    path: plan.suggestedPath,
    conflict: plan.conflict,
    mode: plan.conflict ? ('clone' as const) : ('worktree' as const),
  }
}

/** The request for what has been chosen. Only the targets still on offer go,
 *  in the order they are listed. */
export function createRequest(choice: {
  projectId: string
  branch: string
  path: string
  mode: CreateRequest['mode']
  steps: string[]
  targets: string[]
}): CreateRequest {
  const newBranch = isNewBranch(choice.branch)
  return {
    projectId: choice.projectId,
    branch: choice.branch,
    path: choice.path.trim(),
    mode: choice.mode,
    newBranch,
    from: newBranch ? `origin/${baseBranch()}` : null,
    steps: choice.steps,
    openTargets: offeredTargets(choice.projectId, rowFor(choice.branch))
      .filter((t) => choice.targets.includes(t.id))
      .map((t) => t.id),
  }
}

/** Start the create and select its branch, which puts its card on screen.
 *  False if it couldn't be started; `state.error` says why. */
export async function startCreate(request: CreateRequest): Promise<boolean> {
  try {
    await api.createWorkspace(request, rowFor(request.branch)?.pr?.url ?? null)
  } catch (e) {
    state.error = String(e)
    return false
  }
  selectBranch(request.branch)
  return true
}

/** A workspace for a branch of the project on screen, as the dialog would
 *  make it with nothing changed. The request, or null if it didn't start. */
export async function createWithDefaults(
  projectId: string,
  branch: string,
): Promise<CreateRequest | null> {
  let plan
  try {
    plan = await planWorkspace(projectId, branch, rowFor(branch)?.pr?.number ?? null)
  } catch (e) {
    state.error = String(e)
    return null
  }
  const request = createRequest({
    projectId,
    branch,
    path: plan.path,
    mode: plan.mode,
    steps: defaultSteps(projectId),
    targets: defaultTargets(projectId),
  })
  return (await startCreate(request)) ? request : null
}
