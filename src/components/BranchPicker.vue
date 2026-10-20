<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import Icon from './Icon.vue'
import type { Issue } from '../api'
import {
  branchForIssue,
  branchName,
  issuesError,
  issuesOf,
  issuesPending,
  search,
  type Option,
} from '../branchPicker'
import { basename, relativeTime, WORKSPACE_LABELS } from '../format'
import { highlight } from '../fuzzy'
import { rows } from '../store'

/**
 * The branch field of the New workspace dialog: a text field with a list of
 * branches and open issues under it. Whatever is typed is the branch, as
 * before; the list only offers something better. Its first entry is the
 * text itself, so Enter still submits the dialog, and the list floats over
 * the fields below rather than pushing them down.
 */
const props = defineProps<{
  projectId: string | null
  /** The `origin/…` a new branch starts from. */
  base: string
  /** True when the text names no existing branch. */
  isNew: boolean
}>()

const text = defineModel<string>({ required: true })
/** The issue the branch was picked for, which numbers its directory. */
const issue = defineModel<Issue | null>('issue', { required: true })

const input = ref<HTMLInputElement | null>(null)
const list = ref<HTMLElement | null>(null)
const open = ref(false)
/** -1 is nothing highlighted: only with an empty field, which has no
 *  default to highlight. */
const index = ref(-1)

const issues = computed(() => (props.projectId ? issuesOf(props.projectId) : []))
const options = computed(() => (open.value ? search(text.value, rows.value, issues.value) : []))

/** Shown under the issues heading in place of issues, or alongside held ones
 *  while a fresh list is on its way. */
const issueNote = computed(() => {
  const id = props.projectId
  if (!id) return null
  const error = issuesError(id)
  if (issuesPending(id) && !issues.value.length) return { text: 'Loading issues…', title: '' }
  if (error && !issues.value.length) return { text: 'Issues unavailable', title: error }
  return null
})

const sections = computed(() => {
  const out: { heading: string | null; items: { option: Option; i: number }[] }[] = []
  options.value.forEach((option, i) => {
    const heading =
      i === 0 && hasDefault.value
        ? null
        : option.kind === 'issue'
          ? 'Issues'
          : 'Branches'
    const last = out[out.length - 1]
    if (last && last.heading === heading) last.items.push({ option, i })
    else out.push({ heading, items: [{ option, i }] })
  })
  if (issueNote.value && !out.some((s) => s.heading === 'Issues')) {
    out.push({ heading: 'Issues', items: [] })
  }
  return out
})

/** Whether the first option is the text itself, which Enter submits rather
 *  than picks. An empty field has no such default. */
const hasDefault = computed(() => !!branchName(text.value))

/** Typing re-ranks everything, so the default is highlighted again. */
function reset() {
  index.value = hasDefault.value ? 0 : -1
  list.value?.scrollTo({ top: 0 })
}

watch(text, (value) => {
  // Emptied: whatever issue it was picked for is gone with it.
  if (!value.trim()) issue.value = null
})

function show() {
  if (open.value) return
  open.value = true
  reset()
}

function onInput() {
  open.value = true
  reset()
}

function move(by: number) {
  if (!open.value) return show()
  const count = options.value.length
  if (!count) return
  index.value = index.value < 0 && by < 0 ? count - 1 : (index.value + by + count) % count
  void nextTick(() =>
    list.value?.querySelector('.picker-item.active')?.scrollIntoView({ block: 'nearest' }),
  )
}

function choose(option: Option) {
  open.value = false
  if (option.kind === 'create') return
  if (option.kind === 'branch') {
    text.value = option.row.branch.name
    issue.value = null
    return
  }
  const { name, linked } = branchForIssue(option.issue, rows.value)
  text.value = name
  issue.value = option.issue
  // The name is a guess from the title: selected, past the prefix, so typing
  // replaces it. A linked branch is no guess.
  void nextTick(() => {
    const el = input.value
    if (!el) return
    el.focus()
    if (linked) el.setSelectionRange(name.length, name.length)
    else el.setSelectionRange(name.lastIndexOf('/') + 1, name.length)
  })
}

function onKey(event: KeyboardEvent) {
  if (event.isComposing) return
  const ctrl = event.ctrlKey && !event.metaKey && !event.altKey
  if (event.key === 'ArrowDown' || (ctrl && (event.key === 'n' || event.key === 'j'))) {
    event.preventDefault()
    move(1)
  } else if (event.key === 'ArrowUp' || (ctrl && (event.key === 'p' || event.key === 'k'))) {
    event.preventDefault()
    move(-1)
  } else if (event.key === 'Escape' && open.value) {
    // Closes the list, not the dialog under it.
    event.preventDefault()
    event.stopPropagation()
    open.value = false
  } else if (event.key === 'Enter') {
    // The first entry is the text as typed: Enter on it submits the dialog,
    // as it would with no list. Anything further down is picked instead.
    const first = hasDefault.value ? 1 : 0
    const option = open.value && index.value >= first ? options.value[index.value] : undefined
    open.value = false
    if (option) {
      event.preventDefault()
      choose(option)
    }
  } else if (event.key === 'Tab') {
    open.value = false
  }
}

/** On movement, not on enter: a list scrolled under a resting pointer would
 *  otherwise steal the highlight from the keyboard. */
function hover(i: number) {
  index.value = i
}

function issueAge(issue: Issue): string {
  return relativeTime(Date.parse(issue.updatedAt) / 1000)
}

function focus() {
  input.value?.focus()
  // Nothing to edit yet, so straight to what there is to pick from.
  if (!text.value.trim()) show()
}

defineExpose({ focus })
</script>

<template>
  <div class="picker">
    <div class="picker-field">
      <input
        id="branch"
        ref="input"
        v-model="text"
        spellcheck="false"
        autocomplete="off"
        placeholder="Search branches and issues, or name a new branch"
        role="combobox"
        :aria-expanded="open"
        @input="onInput"
        @keydown="onKey"
        @click="show"
        @blur="open = false"
      />
      <span
        v-if="issue"
        class="picker-tag mono"
        :title="`For issue #${issue.number}: ${issue.title}`"
        >#{{ issue.number }}</span
      >
      <span
        v-if="isNew && branchName(text)"
        class="picker-tag faint"
        :title="`Created from origin/${base} with no upstream set, so your first push goes to its own branch.`"
        >new</span
      >
    </div>

    <!-- mousedown is prevented throughout so the field keeps focus, and
         with it the keyboard and the blur that closes the list. -->
    <div v-if="open && sections.length" ref="list" class="picker-list" @mousedown.prevent>
      <template v-for="(section, s) in sections" :key="section.heading ?? `top${s}`">
        <div v-if="section.heading" class="picker-heading">{{ section.heading }}</div>
        <button
          v-for="{ option, i } in section.items"
          :key="
            option.kind === 'issue'
              ? `#${option.issue.number}`
              : option.kind === 'branch'
                ? option.row.branch.name
                : '\0create'
          "
          type="button"
          tabindex="-1"
          class="picker-item"
          :class="{ active: i === index }"
          @mousemove="hover(i)"
          @click="choose(option)"
        >
          <template v-if="option.kind === 'create'">
            <Icon name="add" />
            <span class="truncate">
              Create branch <span class="mono picker-name">{{ option.name }}</span>
            </span>
            <span class="spacer" />
            <span class="faint mono picker-meta">from origin/{{ base }}</span>
          </template>

          <template v-else-if="option.kind === 'branch'">
            <Icon name="branch" />
            <span class="mono picker-name truncate">
              <span
                v-for="(part, j) in highlight(option.row.branch.name, option.hits)"
                :key="j"
                :class="{ hit: part.hit }"
                >{{ part.text }}</span
              >
            </span>
            <span v-if="option.row.workspaces.length" class="folder">
              <span
                v-for="w in option.row.workspaces"
                :key="w.path"
                class="folder-entry"
                :title="`${WORKSPACE_LABELS[w.kind]}\n${w.path}`"
              >
                <Icon :name="w.kind" :size="12" />
                <span class="truncate">{{ basename(w.path) }}</span>
              </span>
            </span>
            <span class="spacer" />
            <span
              v-if="option.row.pr"
              class="badge"
              :class="option.row.pr.state"
              :title="option.row.pr.title"
              >#{{ option.row.pr.number }}</span
            >
            <span class="faint picker-meta">{{ relativeTime(option.row.branch.committedAt) }}</span>
          </template>

          <template v-else>
            <Icon name="issue" />
            <span class="mono picker-number" :class="{ hit: option.numberHit }"
              >#{{ option.issue.number }}</span
            >
            <span class="truncate" :title="option.issue.title">
              <span
                v-for="(part, j) in highlight(option.issue.title, option.hits)"
                :key="j"
                :class="{ hit: part.hit }"
                >{{ part.text }}</span
              >
            </span>
            <span class="spacer" />
            <span v-if="option.issue.issueType" class="badge">{{ option.issue.issueType }}</span>
            <span class="faint picker-meta">{{ issueAge(option.issue) }}</span>
          </template>
        </button>
        <div
          v-if="section.heading === 'Issues' && issueNote"
          class="picker-note faint"
          :title="issueNote.title"
        >
          {{ issueNote.text }}
        </div>
      </template>
    </div>
  </div>
</template>
