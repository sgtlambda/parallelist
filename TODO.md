# TODO

Things deliberately deferred, with enough context to pick them up cold.

## Replace the placeholder icons

`src/components/Icon.vue` holds hand-drawn Lucide-style glyphs. Call sites only
ever ask for a semantic name (`<Icon name="worktree" />`), so swapping the set
means rewriting that one file and nothing else.

Agreed approach: `unplugin-icons` + `@iconify-json/*` — offline, tree-shaken,
and an icon is chosen by import path so sets can be swapped freely. Octicons is
the obvious candidate given the subject matter. **react-icons cannot be used**;
it ships React components.

## Delete merged local branches that still exist on origin

Half done. "Cleanup branches" now offers local branches with nothing checked
out **and nothing on origin** — the dozen or so leftovers that a merge plus a
worktree removal strands. Branches that are merged but still present on origin
are deliberately not offered: they don't look like leftovers, and origin still
has them.

What is left is the other ~35 in one project: merged into `staging`, still on
origin, still cluttering All. Deleting the local ref costs nothing there —
`unique_commits` would return 0 for every one of them — so the question is
purely how to present it without implying the *remote* branch goes too.

Wants: probably a third sweep, or a filter inside "Cleanup branches", keyed on
`merged && hasRemote && no workspace`. The impact machinery already handles it.

## Cache the historical pull request query

`gh::pull_requests` runs two queries. The `--state all --limit 300` one is
three of every four GraphQL requests and only feeds merged/closed badges — data
that changes slowly. Caching it on a long interval would cut GitHub usage by
roughly 75%.

Not urgent: measured usage is ~11 points per refresh against a 5000/hour
budget, so a normal day sits near 12% of one hour's allowance.

## Exercise the write paths

Everything verified so far reads: the `probe` example, the unit tests, the
timings. Creating a worktree or clone, running setup steps, and removing a
workspace have never been run end to end.

## Sweep the process group twice on timeout

`proc::kill_group` sends a single `killpg(SIGKILL)`. The kernel delivers that
to the group's members as they exist at that instant, so a process forked by
the child in the window around delivery joins the group *after* the signal and
never receives it — defeating the invariant that a timeout takes grandchildren
with it.

Fix: `killpg`, sleep ~50ms, `killpg` again, and only then `child.wait()`. The
ordering is the point — reaping last keeps the leader a zombie, which keeps the
pgid reserved, so the second sweep cannot land on a recycled group.

Not observed in the real app on recent versions, which is why this sits here
rather than in a commit. `git fetch` forks ssh at the *start* of a 45s window,
so the race needs the timeout to fire during those few milliseconds. What does
hit it is `proc::tests::a_timeout_takes_grandchildren_with_it`: a 200ms timeout
against a script whose first act is a fork. It failed twice under CPU load
(a cargo build running alongside) and passed in every other run.

So the test wants de-racing too, whatever happens to `kill_group`: have the
grandchild sleep 3s against a 1s timeout and assert after 4s, so it verifies
the kill rather than the scheduler.

## A delete affordance in the detail pane

"Delete branch…" exists only on the branch list's right-click menu. The detail
pane is where you land after selecting a leftover — it shows the commit, the
`local only · never pushed` badge, and then offers nothing but "+ New
workspace", which is the opposite of what you came to do.

Wants: the same entry point next to that button, routing to the same
`openCleanup([], [branch], 'Delete branch')` the menu uses. Held back only
because the menu was what was asked for.

## Housekeeping

- No CI.
- No LICENSE.
