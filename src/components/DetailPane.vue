<script setup lang="ts">
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { api, type Workspace } from '../api'
import { WORKSPACE_LABELS, WORKSPACE_TITLES, basename, relativeTime, shortPath } from '../format'
import {
  createTasksFor,
  dismissTask,
  elapsed,
  hasCard,
  normalizePath,
  openTaskIn,
  openTasks,
  removalOf,
  taskProgress,
  type LiveTask,
} from '../tasks'
import { openTargets, project, projectId, selectedRow, state } from '../store'
import { workspaceBadges } from '../status'

const emit = defineEmits<{
  (e: 'create'): void
  (e: 'remove', paths: string[]): void
  (e: 'error', message: string): void
}>()

const showAllTargets = ref(false)
const row = selectedRow

/**
 * One card per directory: the row's real workspaces, then create tasks that
 * have no real card yet — running, failed, or done but not listed until the
 * refresh lands. Keyed by path, so when the refresh swaps a task card for the
 * real one Vue patches the element in place instead of removing and re-adding
 * it. A real workspace carries the removal task it is subject to, if any.
 *
 * The directory is a real worktree as soon as `git worktree add` returns, well
 * before setup steps finish, so a refresh mid-create lists it. Until the task
 * is done its card holds that slot and the real one waits — otherwise the
 * same path shows twice, and the half-installed one offers Open and Remove.
 */
const cards = computed(() => {
  const listed = new Set((row.value?.workspaces ?? []).map((w) => normalizePath(w.path)))
  const pending = createTasksFor(projectId.value, state.selectedBranch).filter(
    (t) => hasCard(t) && !(t.status === 'done' && listed.has(normalizePath(t.path ?? ''))),
  )
  const claimed = new Set(
    pending.filter((t) => t.status !== 'done').map((t) => normalizePath(t.path ?? '')),
  )
  const real = (row.value?.workspaces ?? []).filter((w) => !claimed.has(normalizePath(w.path)))
  return [
    ...real.map((workspace) => ({
      key: normalizePath(workspace.path),
      workspace,
      job: null,
      removal: removalOf(projectId.value, workspace.path),
    })),
    ...pending.map((job) => ({
      key: normalizePath(job.path ?? ''),
      workspace: null,
      job,
      removal: null,
    })),
  ]
})

/** A removal that hasn't failed: the card is on its way out. */
function removing(removal: LiveTask | null): boolean {
  return !!removal && removal.status !== 'failed'
}

/** The pane has something to show: a branch row, or a card for one on its
 *  way. */
const title = computed(
  () => row.value?.branch.name ?? (cards.value.length ? state.selectedBranch : null),
)

const targets = computed(() => {
  const usable = openTargets.value.filter((t) => !t.requiresPr || row.value?.pr)
  return showAllTargets.value ? usable : usable.filter((t) => t.primary)
})

const hiddenCount = computed(() => {
  return openTargets.value.filter((t) => (!t.requiresPr || row.value?.pr) && !t.primary).length
})

const reviewLabel: Record<string, string> = {
  APPROVED: 'approved',
  CHANGES_REQUESTED: 'changes requested',
  REVIEW_REQUIRED: 'review required',
}

async function openIn(targetId: string, workspace: Workspace) {
  try {
    await api.openIn(targetId, {
      path: workspace.path,
      branch: workspace.branch ?? '',
      primary: project.value?.path ?? '',
      prUrl: row.value?.pr?.url ?? '',
    })
  } catch (e) {
    emit('error', String(e))
  }
}

/** A finished create's directory is usable before the list has caught up. */
async function openJob(targetId: string, job: LiveTask) {
  try {
    await openTaskIn(targetId, job)
  } catch (e) {
    emit('error', String(e))
  }
}

/** Which clone a worktree hangs off, when it isn't the main workspace. With
 *  two clones of the same repo around, that distinction is the whole story. */
function ownerLabel(workspace: Workspace): string | null {
  if (!workspace.owner || workspace.owner === project.value?.path) return null
  return basename(workspace.owner)
}

/**
 * How this branch stands with origin, as one label.
 *
 * The raw flags read backwards on their own: "not on origin" sounds like
 * unpushed work, when the usual cause is the opposite — the branch was merged
 * and GitHub deleted it afterwards.
 */
const originState = computed(() => {
  const branch = row.value?.branch
  if (!branch) return null
  const base = state.overview?.baseBranch ?? 'the base branch'

  if (!branch.hasRemote && branch.merged) {
    return {
      text: 'merged · gone from origin',
      cls: 'merged',
      title: `Merged into ${base}, then deleted on origin. The workspace can be removed.`,
    }
  }
  if (!branch.hasRemote && branch.wasPushed) {
    return {
      text: 'gone from origin',
      cls: '',
      title: `Pushed at some point, then deleted on origin, and not merged into ${base}.`,
    }
  }
  if (!branch.hasRemote) {
    return {
      text: 'local only · never pushed',
      cls: 'warn',
      title: 'This branch has never existed on origin.',
    }
  }
  if (branch.merged) {
    return { text: `merged into ${base}`, cls: 'merged', title: '' }
  }
  return null
})

async function openPullRequest() {
  const pr = row.value?.pr
  if (!pr) return
  try {
    await openUrl(pr.url)
  } catch (e) {
    emit('error', String(e))
  }
}


</script>

<template>
  <section class="pane">
    <div class="pane-header" data-tauri-drag-region="deep">
      <span class="truncate" style="font-weight: 600">
        {{ title ?? 'No branch selected' }}
      </span>
      <span class="spacer" />
    </div>

    <div v-if="!title" class="pane-body">
      <p class="empty">
        Pick a branch to see its pull request, how far it has drifted from
        <code>{{ state.overview?.baseBranch ?? 'the base branch' }}</code
        >, and which workspaces hold it.
      </p>
    </div>

    <div v-else class="pane-body detail">
      <div v-if="row" class="card">
        <div class="card-title">Commit</div>
        <div>{{ row.branch.subject || '—' }}</div>
        <div class="muted" style="font-size: 11.5px">
          {{ row.branch.author }} · {{ relativeTime(row.branch.committedAt) }} ago ·
          <code class="selectable">{{ row.branch.sha.slice(0, 8) }}</code>
        </div>
        <div class="row" style="flex-wrap: wrap">
          <span
            v-if="originState"
            class="badge"
            :class="originState.cls"
            :title="originState.title"
          >
            {{ originState.text }}
          </span>
          <span v-if="!row.branch.hasLocal" class="badge">remote only</span>
          <span v-if="row.aheadBehind" class="badge">
            ↑{{ row.aheadBehind.ahead }} ahead · ↓{{ row.aheadBehind.behind }} behind
          </span>
        </div>
      </div>

      <button
        v-if="row?.pr"
        class="card pr-card"
        :title="row.pr.url"
        @click="openPullRequest"
      >
        <div class="row">
          <span class="card-title">Pull request</span>
          <span class="spacer" />
          <span class="open-hint" aria-hidden="true">↗</span>
        </div>
        <div class="row">
          <span class="badge" :class="row.pr.isDraft && row.pr.state === 'open' ? 'draft' : row.pr.state">
            #{{ row.pr.number }} {{ row.pr.isDraft && row.pr.state === 'open' ? 'draft' : row.pr.state }}
          </span>
          <span v-if="row.pr.state === 'open' && row.pr.checks !== 'none'" class="badge">
            <span
              class="dot"
              :class="{
                green: row.pr.checks === 'passing',
                red: row.pr.checks === 'failing',
                amber: row.pr.checks === 'pending',
              }"
            />
            checks {{ row.pr.checks }}
          </span>
          <span v-if="reviewLabel[row.pr.reviewDecision]" class="badge">
            {{ reviewLabel[row.pr.reviewDecision] }}
          </span>
        </div>
        <div>{{ row.pr.title }}</div>
        <div class="muted" style="font-size: 11.5px">by {{ row.pr.author }}</div>
      </button>

      <div>
        <div style="display: flex; flex-direction: column; gap: 14px">
          <div
            v-for="item in cards"
            :key="item.key"
            class="card workspace"
            :class="{
              stale: item.workspace && row?.stale && item.workspace.kind !== 'primary',
              removing: removing(item.removal),
              pending: item.job,
              ready: item.job?.status === 'done',
              failed: item.job?.status === 'failed',
            }"
            :role="item.job ? 'button' : undefined"
            :tabindex="item.job ? 0 : undefined"
            :title="item.job ? 'Show the full log' : undefined"
            @click="item.job && openTasks(item.job.id)"
            @keydown.enter="item.job && openTasks(item.job.id)"
          >
            <template v-if="item.workspace">
              <div class="row card-head">
                <span class="card-title">{{ WORKSPACE_TITLES[item.workspace.kind] }}</span>
                <span class="spacer" />
                <!-- Greyed while it goes; the badge opens the removal's log. -->
                <button
                  v-if="item.removal"
                  class="badge removal"
                  :class="{ closed: item.removal.status === 'failed' }"
                  title="Show the log"
                  @click="openTasks(item.removal.id)"
                >
                  <template v-if="item.removal.status === 'queued'">queued for removal</template>
                  <template v-else-if="item.removal.status === 'failed'">removal failed</template>
                  <template v-else><span class="spin tiny" /> removing</template>
                </button>
              </div>
              <div v-if="workspaceBadges(item.workspace).length" class="row" style="flex-wrap: wrap">
                <span
                  v-for="badge in workspaceBadges(item.workspace)"
                  :key="badge.key"
                  class="badge"
                  :class="{ warn: badge.warn }"
                  :title="badge.title"
                >
                  {{ badge.text }}
                </span>
              </div>
              <div class="path-row">
                <span class="kind-icon" :title="WORKSPACE_LABELS[item.workspace.kind]">
                  <Icon :name="item.workspace.kind" />
                </span>
                <code class="truncate selectable" :title="item.workspace.path">
                  {{ shortPath(item.workspace.path) }}
                </code>
              </div>
              <div v-if="ownerLabel(item.workspace)" class="faint" style="font-size: 11px">
                worktree of {{ ownerLabel(item.workspace) }}
              </div>

              <div class="actions">
                <button
                  v-for="target in targets"
                  :key="target.id"
                  class="btn small"
                  :disabled="removing(item.removal)"
                  @click="openIn(target.id, item.workspace)"
                >
                  {{ target.icon }} {{ target.label }}
                </button>
                <button
                  v-if="hiddenCount && !showAllTargets"
                  class="btn ghost small"
                  @click="showAllTargets = true"
                >
                  +{{ hiddenCount }} more
                </button>
                <span class="spacer" />
                <button
                  v-if="item.removal?.status === 'failed'"
                  class="btn ghost small"
                  @click="dismissTask(item.removal.id)"
                >
                  Dismiss
                </button>
                <button
                  v-if="item.workspace.kind !== 'primary'"
                  class="btn ghost small danger"
                  :disabled="removing(item.removal)"
                  @click="emit('remove', [item.workspace.path])"
                >
                  Remove
                </button>
              </div>
            </template>

            <!-- A workspace on its way, or just arrived. The card is the
                 create task's progress line; clicking it opens the full log.
                 Once the task is done it stays, marked ready, until the
                 refreshed list carries the real card for the same path. -->
            <template v-else-if="item.job">
              <div class="row">
                <span class="card-title">{{ WORKSPACE_TITLES[item.job.mode ?? 'worktree'] }}</span>
                <span class="spacer" />
                <span v-if="item.job.status === 'running'" class="badge">
                  <span class="spin tiny" />
                  creating · {{ elapsed(item.job) }}
                </span>
                <span v-else-if="item.job.status === 'queued'" class="badge">queued</span>
                <span v-else-if="item.job.status === 'done'" class="badge open">ready</span>
                <span v-else class="badge closed">failed</span>
              </div>
              <div class="path-row">
                <span class="kind-icon" :title="WORKSPACE_LABELS[item.job.mode ?? 'worktree']">
                  <Icon :name="item.job.mode ?? 'worktree'" />
                </span>
                <code class="truncate selectable" :title="item.job.path ?? ''">
                  {{ shortPath(item.job.path ?? '') }}
                </code>
              </div>
              <div v-if="item.job.status !== 'done'" class="job-progress">
                <div class="truncate job-step">{{ taskProgress(item.job).step || 'Starting' }}</div>
                <code class="truncate job-line" :class="{ err: item.job.status === 'failed' }">
                  {{ taskProgress(item.job).line }}
                </code>
              </div>
              <div v-if="item.job.status === 'done'" class="actions">
                <button
                  v-for="target in targets.filter((t) => !t.requiresPr)"
                  :key="target.id"
                  class="btn small"
                  @click.stop="openJob(target.id, item.job)"
                >
                  {{ target.icon }} {{ target.label }}
                </button>
              </div>
              <div v-if="item.job.status === 'failed'" class="actions">
                <button class="btn small" @click.stop="openTasks(item.job.id)">Show log</button>
                <span class="spacer" />
                <button class="btn ghost small" @click.stop="dismissTask(item.job.id)">Dismiss</button>
              </div>
            </template>
          </div>

          <button class="new-workspace" @click="emit('create')">+ New workspace</button>
        </div>
      </div>
    </div>
  </section>
</template>
