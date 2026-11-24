import CreateDialog from '../../components/CreateDialog.vue'
import type { CreatePlan } from '../../api'
import { backend } from '../backend'
import { boot } from '../seed'
import { defineGroup } from '../define'

const WS = '/Users/sandbox/Code'

async function seed() {
  await boot()
  backend.handle({
    list_issues: () => [],
    plan_workspace: (args: { branch: string }): CreatePlan => ({
      suggestedPath: `${WS}/app-${args.branch.split('/').pop()}`,
      conflict:
        args.branch === 'fix/1234-header'
          ? { branch: args.branch, path: `${WS}/app-1234-fix-header` }
          : null,
    }),
    create_workspace: () => null,
  })
}

export default defineGroup('New workspace', [
  {
    id: 'new-branch',
    title: 'A branch with no workspace',
    component: CreateDialog,
    props: { open: true, initialBranch: 'feature/search-index' },
    setup: seed,
  },
  {
    id: 'conflict',
    title: 'A branch that already has a worktree',
    note: 'Git allows a branch in one worktree per clone, so only a clone is offered.',
    component: CreateDialog,
    props: { open: true, initialBranch: 'fix/1234-header' },
    setup: seed,
  },
])
