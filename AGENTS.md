# Switcheroo — notes for agents

A macOS app (Tauri 2 + Vue 3) for managing git branches, worktrees and clones
across a few large repos. This file records the things that are **not** obvious
from reading the code: why the model is shaped the way it is, and which
apparently-redundant details are load-bearing.

Start with `README.md` for what the app does. This file is about working on it.

## The problem it solves

The user works across many branches of a few big monoliths and
constantly spins up worktrees to review or test them. Sibling directories
(`~/Code/app`, `~/Code/app-1234-fix-header`, `~/Code/app-b`) accumulate until
nobody knows what is checked out where, which are merged, or which are safe to
delete. The app exists to answer that.

## Domain model

**Origin-first.** The branch list is *origin's* branches, kept fresh by a
background fetch. Having a branch on disk locally is a side effect, not the
source of truth. Filters, sorting and staleness all follow from this.

**"Checkout" is the central noun** — any directory holding a branch, whether a
linked worktree or a full clone. The UI treats them identically on purpose. The
distinction matters in exactly two places:

- **Creating.** Git allows a branch in exactly one worktree per clone. When that
  conflicts, the dialog offers a clone instead. This is the whole reason a
  `repo-b` directory exists next to `repo` in the first place.
- **Removing.** `git worktree remove` versus `rm -rf`.

Clones are made with `git clone --local` from the primary clone, which hardlinks
the object store (seconds, not minutes, on a large repo), then `origin` is
repointed at the real remote so pushing works.

Everything shells out to the real `git` and `gh` binaries rather than linking
libgit2, so hooks, credential helpers, SSH config and worktree semantics behave
exactly as they do in a terminal.

## Invariants — do not "simplify" these away

Each of these was arrived at by observing a real failure. They look like
redundant complexity and are not.

1. **Every child process has a timeout and its own process group.** A half-open
   TCP connection leaves `git fetch` blocked in `packet_reader_read` forever —
   no data, no error, no recovery. Killing git alone provably leaves its `ssh`
   child alive holding the dead socket (observed surviving 7 minutes after its
   parent died), so the timeout `killpg`s the whole group. Covered by
   `a_timeout_takes_grandchildren_with_it`.

2. **`PATH` comes from the login shell**, resolved once at startup via
   `$SHELL -lc`. An app launched from Finder inherits launchd's minimal `PATH`
   with no Homebrew, so `gh` fails to resolve and PR data silently disappears.
   **This makes terminal testing misleading**: `open` propagates the caller's
   environment, so a build launched from a terminal works while the same build
   launched from the Dock does not. Test the packaged app the way the user
   launches it, or reproduce with `env -i PATH=/usr/bin:/bin`.

3. **`scan()` takes the ref set as an argument** instead of recomputing it. It
   used to call `git::branches` again, repeating the expensive `--merged` pass.

4. **Working trees are inspected on a small thread pool.** Sequential
   `git status` across 16 checkouts was ~3.8s; in parallel it is ~0.55s. On a
   monolith with `vendor/` and `node_modules/`, each `git status` is ~140ms.

5. **A branch checked out in a secondary clone is added from that checkout.**
   A clone has its own ref namespace and the primary may never have seen the
   branch, so such checkouts would otherwise have no row and vanish from the UI
   while still being counted under Cleanup. **Only checked-out branches are
   added** — merging every ref from every clone would drag in that clone's stale
   `origin/*` snapshot and resurrect hundreds of long-dead branches.

6. **New branches are created with `--no-track`.** Otherwise git sets the
   upstream to the start point and a later bare `git push` lands on the branch it
   was cut from. This is a standing rule in the user's global `CLAUDE.md`.

7. **`-webkit-app-region` does nothing in WKWebView** — it is an Electron
   feature. Window dragging uses `data-tauri-drag-region="deep"` and requires
   `core:window:allow-start-dragging`, which is **not** part of `core:default`
   and must be granted explicitly in `capabilities/`. Without it the attribute
   is a silent no-op.

8. **Modified and untracked files are counted separately.** `git status
   --porcelain` lists both, and conflating them overstates the case badly: a
   working tree whose only "change" is a stray `.DS_Store` gets reported as
   having uncommitted work, while the user's IDE correctly shows it as clean.
   Only `modified` and `unpushed` drive the warning badges and the removal
   impact dialog.

## Conventions

- **The UI says "Project"; the code says "repo"** (`RepoConfig`, `repoId`,
  `add_repo`, the `repos` key in `config.json`). This is deliberate — renaming
  the internals is churn with a config-migration risk and none of it is visible.
  Don't "fix" the inconsistency without being asked.
- **`src/components/Icon.vue` is the only place a glyph is defined.** Call sites
  ask for a semantic name (`<Icon name="worktree" />`), never a picture, so the
  icon set can be swapped by editing one file.
- Error messages that say "git repository" are factual statements about git and
  should stay that way, even though the UI otherwise says "project".

## Verifying work

**The GUI cannot be screenshotted.** Screen recording permission is not granted
to the terminal and the user declined to grant it. Do not plan on visual
inspection of the running app.

Instead:

```sh
cd src-tauri && cargo run --example probe -- ~/Code/some-repo
```

`probe` exercises the entire git/gh layer headlessly and prints branches,
checkouts, the sniffed setup recipe, PRs, and whether every checkout resolves to
a branch row. It found two real bugs in one session — prefer it to guessing.

To check SVG or other rendered output, `qlmanage -t -s 900 -o <dir> file.svg`
renders through WebKit, the same engine the app uses. ImageMagick's built-in SVG
renderer is too limited to be a useful check.

## Build and run

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # rustup ran with --no-modify-path
pnpm install
pnpm tauri dev
pnpm tauri build      # -> src-tauri/target/release/bundle/macos/Switcheroo.app
pnpm typecheck        # vue-tsc
pnpm test             # cargo test
pnpm lint             # cargo clippy
```

`bundle.targets` is `["app"]` on purpose: DMG bundling needs Finder scripting
access and fails without it. The `.app` is the deliverable.

## Configuration

- App config: `~/Library/Application Support/com.jmversteeg.switcheroo/config.json`
- Per-project override: `.switcheroo.json` committed at a project's root, which
  wins field by field over the app config.
- "Open in" targets are user-editable shell templates (`{path}`, `{branch}`,
  `{repo}`, `{prUrl}`, substituted and shell-quoted) run through the login shell.

## Open items

- Icons are hand-drawn Lucide-style placeholders. Agreed upgrade path is
  `unplugin-icons` + `@iconify-json/*` (offline, tree-shaken); only `Icon.vue`
  changes. **react-icons is React-only and cannot be used here.**
- Right-clicking a project in the sidebar removes it immediately, with no
  confirmation and no visual hint that it is possible. Known, deliberately left
  alone, flagged to the user.
- No CI, no LICENSE.
- The `git fetch` hang was traced to a half-open connection upstream. The timeout
  makes it recoverable; the root cause is network-side and is not fixed.
