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

## Delete merged local branches

Cleanup removes directories, not branches. One project currently carries 47 local
branches already merged into `staging`, a dozen of which no longer exist on
origin at all. They are hidden from Active but still clutter All.

Wants: a list of merged local branches with the same warn-don't-block treatment
as checkout removal, skipping anything currently checked out (git refuses to
delete those anyway).

## Cache the historical pull request query

`gh::pull_requests` runs two queries. The `--state all --limit 300` one is
three of every four GraphQL requests and only feeds merged/closed badges — data
that changes slowly. Caching it on a long interval would cut GitHub usage by
roughly 75%.

Not urgent: measured usage is ~11 points per refresh against a 5000/hour
budget, so a normal day sits near 12% of one hour's allowance.

## `{repo}` means two different things

In a path template it is the repository's *directory name*
(`checkout::suggest_path`). In an "open in" target it is the primary clone's
*full path* (`open_in`). Both are documented as `{repo}`. Worth unifying, but it
is a user-visible config change, so it needs a migration or an alias.

## Exercise the write paths

Everything verified so far reads: the `probe` example, the unit tests, the
timings. Creating a worktree or clone, running setup steps, and removing a
checkout have never been run end to end.

## Housekeeping

- No CI.
- No LICENSE.
