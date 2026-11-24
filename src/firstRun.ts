import { computed, reactive, ref } from 'vue'
import { api, type OpenTarget, type TargetCategory, type TargetSuggestion } from './api'
import { state } from './store'

/**
 * The first-run choice of "Open in" targets: what `suggest_targets` found
 * on this Mac, which of them are ticked, and adopting the ticked ones. Kept
 * apart from any one way of showing it, so the concepts in the sandbox all
 * run the same logic.
 */

/** In the order shown. Finder comes first and needs no heading. */
export const CATEGORIES: { key: TargetCategory; title: string }[] = [
  { key: 'other', title: '' },
  { key: 'agent', title: 'In the terminal' },
  { key: 'editor', title: 'Editors' },
  { key: 'terminal', title: 'Terminal apps' },
  { key: 'git', title: 'Git clients' },
]

export function useSuggestions() {
  const list = ref<TargetSuggestion[] | null>(null)
  const error = ref<string | null>(null)
  const chosen = reactive(new Set<string>())

  api
    .suggestTargets()
    .then((all) => {
      list.value = all
      for (const s of all) if (s.recommended) chosen.add(s.target.id)
    })
    .catch((e) => (error.value = String(e)))

  const found = computed(() => (list.value ?? []).filter((s) => s.found && !s.configured))
  const byCategory = (from: TargetSuggestion[]) =>
    CATEGORIES.map((c) => ({ ...c, items: from.filter((s) => s.category === c.key) })).filter(
      (c) => c.items.length,
    )

  function toggle(id: string, on = !chosen.has(id)) {
    if (on) chosen.add(id)
    else chosen.delete(id)
  }

  /** The ticked targets, in the catalog's order. */
  const picked = computed(() =>
    (list.value ?? []).filter((s) => chosen.has(s.target.id)).map((s) => s.target),
  )

  return { list, error, chosen, found, byCategory, toggle, picked }
}

/** Add `targets` and mark the choice made; none skips it. The store takes
 *  the saved config, or its next save would write the old one back. */
export async function adoptTargets(targets: OpenTarget[]) {
  const next = await api.adoptTargets(JSON.parse(JSON.stringify(targets)))
  next.listWidth ??= state.config?.listWidth ?? null
  state.config = next
}
