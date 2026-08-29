<script setup lang="ts">
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, ref } from 'vue'
import Icon from './Icon.vue'
import { api, type Checkout } from '../api'
import { CHECKOUT_LABELS, basename, relativeTime, shortPath } from '../format'
import { openTargets, project, selectedRow, state } from '../store'

const emit = defineEmits<{
  (e: 'create'): void
  (e: 'remove', paths: string[]): void
  (e: 'error', message: string): void
}>()

const showAllTargets = ref(false)
const row = selectedRow

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

async function openIn(targetId: string, checkout: Checkout) {
  try {
    await api.openIn(targetId, {
      path: checkout.path,
      branch: checkout.branch ?? '',
      primary: project.value?.path ?? '',
      prUrl: row.value?.pr?.url ?? '',
    })
  } catch (e) {
    emit('error', String(e))
  }
}

/** Which clone a worktree belongs to, when it isn't the primary one. With two
 *  clones of the same repo around, that distinction is the whole story. */
function ownerLabel(checkout: Checkout): string | null {
  if (!checkout.owner || checkout.owner === project.value?.path) return null
  return basename(checkout.owner)
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
      title: `Merged into ${base}, then deleted on origin. The checkout can be removed.`,
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
        {{ row ? row.branch.name : 'No branch selected' }}
      </span>
      <span class="spacer" />
    </div>

    <div v-if="!row" class="pane-body">
      <p class="empty">
        Pick a branch to see its pull request, how far it has drifted from
        <code>{{ state.overview?.baseBranch ?? 'the base branch' }}</code
        >, and where it is checked out.
      </p>
    </div>

    <div v-else class="pane-body detail">
      <div class="card">
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
        v-if="row.pr"
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
        <p v-if="!row.checkouts.length" class="muted" style="font-size: 12px; line-height: 1.5">
          Not checked out anywhere.
        </p>

        <div style="display: flex; flex-direction: column; gap: 14px">
          <div
            v-for="checkout in row.checkouts"
            :key="checkout.path"
            class="card checkout"
            :class="{ stale: checkout.branchGone }"
          >
            <div class="card-title">Checkout</div>
            <div
              v-if="checkout.modified || checkout.untracked || checkout.unpushed"
              class="row"
            >
              <span v-if="checkout.modified" class="badge warn">
                {{ checkout.modified }} changed
              </span>
              <span v-if="checkout.untracked" class="badge" title="Untracked files — not counted as changes">
                {{ checkout.untracked }} untracked
              </span>
              <span v-if="checkout.unpushed" class="badge warn">{{ checkout.unpushed }} unpushed</span>
            </div>
            <div class="path-row">
              <span class="kind-icon" :title="CHECKOUT_LABELS[checkout.kind]">
                <Icon :name="checkout.kind" />
              </span>
              <code class="truncate selectable" :title="checkout.path">
                {{ shortPath(checkout.path) }}
              </code>
            </div>
            <div v-if="ownerLabel(checkout)" class="faint" style="font-size: 11px">
              worktree of {{ ownerLabel(checkout) }}
            </div>

            <div class="actions">
              <button
                v-for="target in targets"
                :key="target.id"
                class="btn small"
                @click="openIn(target.id, checkout)"
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
                v-if="checkout.kind !== 'primary'"
                class="btn ghost small danger"
                @click="emit('remove', [checkout.path])"
              >
                Remove
              </button>
            </div>
          </div>

          <button class="new-checkout" @click="emit('create')">+ New checkout</button>
        </div>
      </div>
    </div>
  </section>
</template>
