# Switcheroo

A macOS app for working with a lot of branches across a few big repos: see
what's on origin, check a branch out into a worktree (or a clone), and clean up
the directories you're done with.

Built with [Tauri 2](https://tauri.app) (Rust) and Vue 3.

## Requirements

| | |
|---|---|
| macOS | Apple Silicon or Intel |
| `git` | any recent version |
| `gh` | optional — without it the app works, minus pull request data |

`gh` must be authenticated (`gh auth login`) for pull request, review and CI
state to appear.

## The idea

Branches live on **origin**. Having one on disk is a side effect, so the list
you look at is origin's branch list, kept fresh by a background fetch, with
local state hung off the side.

A place a branch lives on disk is a **workspace**. It is either a linked
worktree (cheap, shares the object store) or a full clone. That distinction
only matters twice:

- **Creating one.** Git allows a branch in exactly one worktree per clone. When
  the branch you want is already checked out, Switcheroo says so and offers a
  clone instead — which is what an `app-b` directory next to `app` really is.
- **Removing one.** A worktree is unregistered with `git worktree remove`; a
  clone is just deleted.

Everywhere else they are the same thing, and the UI treats them that way.

A clone is an independent repository with its own refs, so a branch checked out
in one may not exist in the primary clone at all. Those branches are added to
the list from the workspace itself. Only branches that are actually checked out
are added — merging every ref from every clone would drag in each clone's stale
`origin/*` copies and resurrect hundreds of dead branches.

## Layout

```
repos          branches (origin-first)        the selected branch
─────────────  ─────────────────────────────  ──────────────────────
acme-app       fix/login-redirect             commit, author, drift
web-portal       #1042 ● open  ⑂      2d      pull request + checks
docs-site      search-index                   every workspace of it
                 #1037 ● open  ⑂      5d      open in… / remove
               ci/cache-deps
                 merged  ⑂  stale    9d
```

Tabs: **Active** (open PR, checked out, or committed to recently) · **Open PRs**
· **Assigned** (open PRs assigned to you) · **Checked out** · **All**.

Search always looks across every branch regardless of the tab, and matches
branch names, commit subjects, authors, PR numbers and titles, and **workspace
directory paths** — so a clone is findable by its folder name (`app-b`) even
when its branch exists nowhere else.

`⌘F` search · `⌘R` fetch now · `⌘N` new workspace of the selected branch.

## Creating a workspace

The dialog suggests a directory from the repo's path template
(`{parent}/{project}-{slug}`, where the slug is the branch's last segment prefixed
with its PR number) and lets you edit it. Setup steps are offered as
checkboxes, so each one is a per-workspace decision rather than a policy:

- **copy files** — `.env` and friends out of the primary clone
- **symlink** — point `vendor` / `node_modules` at the primary clone
- **command** — `composer install`, `pnpm install`, anything else

Output streams into a log pane. A branch that does not exist yet is created
with `--no-track`, so your first `git push` goes to its own branch instead of
the one it was cut from.

Clones are made with `git clone --local` from the primary clone, which hardlinks
the object store, then `origin` is pointed back at the real remote. A second
copy of a large repo therefore costs seconds rather than a full network clone.

## Cleanup

Merged branches, branches deleted on origin, and directories that are not git
workspaces at all are collected under **Cleanup**. Each row shows what removing
it would destroy — uncommitted changes, unpushed commits, stashes, size on
disk. Nothing is blocked; you just get to see the bill first.

## Configuration

App settings live in
`~/Library/Application Support/com.jmversteeg.switcheroo/config.json` and are
edited in the settings pane. A repo may also commit a `.switcheroo.json` at its
root, which wins field by field:

```json
{
  "baseBranch": "main",
  "pathTemplate": "{parent}/{project}-{slug}",
  "setupSteps": [
    {
      "id": "copy-env",
      "label": "Copy env files",
      "defaultEnabled": true,
      "kind": "copyFiles",
      "files": [".env", ".env.testing"]
    }
  ]
}
```

**Open in** targets are shell templates run through your login shell, with
`{path}`, `{branch}`, `{primary}` and `{prUrl}` substituted and shell-quoted. Add,
edit or remove as many as you like:

```
open -na Ghostty --args --working-directory={path}
open -na Ghostty --args --working-directory={path} -e claude
phpstorm {path}
open {prUrl}
```

## Development

Needs [Rust](https://rustup.rs) and [pnpm](https://pnpm.io).

```sh
pnpm install
pnpm tauri dev        # run it
pnpm tauri build      # -> src-tauri/target/release/bundle/macos/Switcheroo.app
pnpm typecheck        # vue-tsc
pnpm test             # cargo test
pnpm lint             # cargo clippy
```

### Architecture

The whole git and GitHub layer shells out to the real `git` and `gh` binaries,
so worktrees, hooks, credential helpers and SSH config behave exactly as they
do in your terminal.

| module | responsibility |
|---|---|
| `src-tauri/src/proc.rs` | spawning processes: timeouts, PATH, stream draining |
| `src-tauri/src/git.rs` | branches, worktrees, working-tree state |
| `src-tauri/src/gh.rs` | pull requests, reviews, CI rollup |
| `src-tauri/src/workspace.rs` | discovering, creating and removing workspaces |
| `src-tauri/src/config.rs` | app config, per-repo overrides, setup recipes |
| `src-tauri/src/lib.rs` | Tauri commands, progress events |
| `src/store.ts` | reactive state, filtering, background sync |

Two details worth knowing before changing `proc.rs`:

- **Every child gets a timeout** and runs in its own process group, killed with
  `killpg`. A dead TCP connection makes `git fetch` block forever with no error
  and no data, and killing git alone leaves its `ssh` child alive holding the
  socket.
- **`PATH` comes from your login shell**, resolved once at startup. An app
  launched from Finder inherits launchd's minimal `PATH`, which has no
  Homebrew — so `gh` would not exist and pull request data would silently
  vanish, while the same build launched from a terminal worked fine.

To exercise the git and GitHub layer without launching the window:

```sh
cd src-tauri && cargo run --example probe -- ~/Code/some-repo
```

It prints the branches, workspaces, setup recipe and pull requests it finds, and
verifies that every workspace resolves to a branch row.

### Icons

`src/components/Icon.vue` is the only place glyphs are defined; call sites ask
for a semantic name (`<Icon name="worktree" />`). Swapping the placeholders for
a real icon set means rewriting that one file.
