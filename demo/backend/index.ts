import type {
  AppConfig,
  BranchImpact,
  CreateRequest,
  RemovalImpact,
  WorkspaceKind,
} from '../../src/api'
import { emit } from '../shims/event'
import { seedTerminals } from './seed'
import * as tasks from './tasks'
import * as terminals from './terminals'
import {
  HOME,
  aheadBehind,
  config,
  copy,
  overview,
  project,
  shortHome,
  slugify,
  world,
} from './world'

/**
 * The Rust side, as far as a page can stand in for it: every command
 * `src/api.ts` invokes, answered from the made-up machine in `world.ts`.
 * Nothing leaves the page. Where a command does slow work in the real app
 * (refreshing, creating, removing) it is a task here too, with a short log.
 */

seedTerminals()

const { delay } = tasks

type Handler = (args: any) => unknown | Promise<unknown>

/** Removals and branch deletions run one at a time per project, as in
 *  `tasks.rs`. */
const lanes = new Map<string, Promise<unknown>>()
function inLane(projectId: string, work: () => Promise<void>) {
  const run = (lanes.get(projectId) ?? Promise.resolve()).then(work, work)
  lanes.set(projectId, run)
}

let firstRefresh = true

const handlers: Record<string, Handler> = {
  get_config: () => copy(config),
  save_config: ({ config: next }: { config: AppConfig }) => {
    Object.assign(config, copy(next))
  },
  add_project: () => {
    throw new Error('Choosing a folder is not possible in a web page')
  },
  remove_project: ({ id }: { id: string }) => {
    config.projects = config.projects.filter((p) => p.id !== id)
    if (config.selectedProjectId === id) config.selectedProjectId = config.projects[0]?.id ?? null
  },
  select_project: ({ id }: { id: string | null }) => {
    config.selectedProjectId = id
  },

  refresh: async ({ id }: { id: string }) => {
    const task = tasks.start({ projectId: id, kind: 'refresh', title: `Refreshing ${project(id).name}` })
    // The first one is slower, as a fetch over a real network is.
    const slow = firstRefresh
    firstRefresh = false
    tasks.line(task, '▸ Fetching origin')
    await delay(slow ? 700 : 300)
    tasks.line(task, '▸ Reading workspaces')
    await delay(slow ? 500 : 200)
    tasks.finish(task)
    return overview(id)
  },
  peek: async ({ id }: { id: string }) => {
    await delay(60)
    return overview(id, true)
  },
  ahead_behind: ({ branches }: { branches: string[] }) =>
    Object.fromEntries(branches.map((b) => [b, aheadBehind(b)])),
  list_issues: async ({ id }: { id: string }) => {
    await delay(500)
    return copy(world[id].issues)
  },

  plan_workspace: ({ id, branch, pr }: { id: string; branch: string; pr: number | null }) => {
    const data = world[id]
    const slug = [pr, slugify(branch)].filter(Boolean).join('-')
    const base = `${HOME}/Code/${project(id).name}-${slug}`
    const taken = new Set(data.workspaces.map((w) => w.path))
    let path = base
    for (let suffix = 98; taken.has(path); suffix++) path = `${base}-${String.fromCharCode(suffix)}`
    // A branch can be in one worktree of a clone only.
    const held = data.workspaces.find((w) => w.branch === branch && w.kind !== 'clone')
    return { suggestedPath: path, conflict: held ? { branch, path: held.path } : null }
  },
  create_workspace: ({ request, prUrl }: { request: CreateRequest; prUrl: string | null }) =>
    createWorkspace(request, prUrl),

  removal_impacts: ({ id, paths }: { id: string; paths: string[] }) =>
    paths.map((path): RemovalImpact => {
      const w = world[id].workspaces.find((x) => x.path === path)
      const unpushed = w?.status?.unpushed ?? 0
      return {
        path,
        kind: w?.kind ?? 'orphan',
        branch: w?.branch ?? null,
        status: copy(w?.status ?? null),
        error: null,
        commits: unpushed,
        commitsNeedBranchDelete: w?.kind === 'worktree',
        stashes: 0,
        dependents: [],
      }
    }),
  workspace_sizes: ({ paths }: { paths: string[] }) =>
    Object.fromEntries(paths.map((p, i) => [p, (380 + i * 140) * 1024 * 1024])),
  remove_workspaces: ({
    id,
    items,
  }: {
    id: string
    items: { path: string; kind: WorkspaceKind; deleteBranch: string | null }[]
  }) =>
    items.map((item) => {
      const name = item.path.split('/').pop() ?? item.path
      const task = tasks.start({
        projectId: id,
        kind: 'remove',
        title: `Removing ${name}`,
        path: item.path,
        branch: world[id].workspaces.find((w) => w.path === item.path)?.branch ?? undefined,
      })
      inLane(id, async () => {
        tasks.line(task, `▸ Removing ${shortHome(item.path)}`)
        await delay(900)
        // Closed first, as the real one does.
        const closing = terminals.get(item.path)
        if (closing) {
          closing.channel?.onmessage({ exit: 0 })
          terminals.close(item.path)
          emit('terminal-exit', item.path)
        }
        world[id].workspaces = world[id].workspaces.filter((w) => w.path !== item.path)
        if (item.deleteBranch) dropBranch(id, item.deleteBranch)
        tasks.line(task, item.kind === 'clone' ? 'Deleted the directory' : 'Removed the worktree')
        await delay(300)
        tasks.finish(task)
      })
      return task.id
    }),
  branch_impacts: ({ branches }: { branches: string[] }) =>
    branches.map((branch): BranchImpact => ({ branch, commits: [], commitCount: 0 })),
  delete_branches: ({ id, branches }: { id: string; branches: string[] }) =>
    branches.map((branch) => {
      const task = tasks.start({
        projectId: id,
        kind: 'deleteBranch',
        title: `Deleting branch ${branch}`,
        branch,
      })
      inLane(id, async () => {
        await delay(700)
        dropBranch(id, branch)
        tasks.line(task, `Deleted branch ${branch}`)
        tasks.finish(task)
      })
      return task.id
    }),

  open_in: ({ targetId }: { targetId: string }) => {
    const target = [...config.openTargets, ...config.projects.flatMap((p) => p.openTargets)].find(
      (t) => t.id === targetId,
    )
    return target?.kind === 'terminal' ? target.command : null
  },
  open_link: () => undefined,
  copy_text: async ({ text }: { text: string }) => {
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      /* the toast says copied either way; a page may not be allowed to */
    }
  },

  list_tasks: () => copy(tasks.all()),
  get_task: ({ id }: { id: string }) => copy(tasks.get(id)),
  set_task_attention: ({ id, attention }: { id: string; attention: boolean }) =>
    tasks.setAttention(id, attention),
  // A page has no menu bar to enable anything in.
  set_create_menu: () => {},

  terminal_open: ({ projectId, branch, path, cols, rows, channel }: any) =>
    terminals.open(projectId, branch, path, cols, rows, channel),
  terminal_write: ({ path, data }: { path: string; data: string }) => terminals.write(path, data),
  terminal_resize: ({ path, cols }: { path: string; cols: number }) =>
    terminals.resize(path, cols),
  terminal_close: ({ path }: { path: string }) => terminals.close(path),
  terminal_list: () => terminals.list(),
  terminal_viewing: ({ path, focused }: { path: string | null; focused: boolean }) =>
    terminals.view(path, focused),
  terminal_question: async ({ path }: { path: string }) => {
    // The real one reads Claude Code's transcript, which takes a moment.
    await delay(80)
    return terminals.question(path)
  },
  terminal_mark: ({ path, mark }: { path: string; mark: 'unseen' | 'blocked' }) =>
    terminals.mark(path, mark),
}

function dropBranch(id: string, name: string) {
  const data = world[id]
  data.branches = data.branches.filter((b) => b.name !== name)
  delete data.prs[name]
}

/** A worktree or clone for a branch, made in the background. The directory
 *  appears in the world when the task is done, which is when the real one
 *  would be listed too. */
function createWorkspace(request: CreateRequest, prUrl: string | null): string {
  const data = world[request.projectId]
  const steps = project(request.projectId).setupSteps.filter((s) => request.steps.includes(s.id))
  const task = tasks.start({
    projectId: request.projectId,
    kind: 'create',
    title: `Creating ${request.branch}`,
    branch: request.branch,
    path: request.path,
    mode: request.mode,
    prUrl,
    openTargets: request.openTargets,
  })
  void (async () => {
    tasks.line(task, `▸ Creating ${request.mode} at ${shortHome(request.path)}`)
    await delay(500)
    tasks.line(
      task,
      request.newBranch
        ? `Preparing ${request.mode} (new branch '${request.branch}')`
        : `Preparing ${request.mode} (checking out '${request.branch}')`,
    )
    await delay(600)
    for (const step of steps) {
      tasks.line(task, `▸ ${step.label}`)
      await delay(step.id === 'install' ? 1100 : 300)
      if (step.id === 'install') tasks.line(task, 'Packages: +412 (reused 412, downloaded 0)')
    }
    if (request.newBranch) {
      data.branches.unshift({
        name: request.branch,
        hasRemote: false,
        hasLocal: true,
        sha: '0000000000000000000000000000000000000000',
        committedAt: Math.floor(Date.now() / 1000),
        author: 'you',
        subject: `Start ${request.branch} from ${request.from ?? 'main'}`,
        merged: false,
        wasPushed: false,
      })
    }
    data.workspaces.push({
      kind: request.mode,
      path: request.path,
      branch: request.branch,
      owner: request.mode === 'worktree' ? project(request.projectId).path : null,
      status: { modified: 0, untracked: 0, unpushed: 0, behind: 0 },
      statusError: null,
    })
    await delay(200)
    tasks.finish(task)
  })()
  return task.id
}

/** The one door: `invoke` in the Tauri shim. */
export async function call(command: string, args: Record<string, unknown>): Promise<unknown> {
  const handler = handlers[command]
  if (!handler) throw new Error(`${command} is not part of the demo`)
  return handler(args)
}

export const startStory = terminals.startStory
