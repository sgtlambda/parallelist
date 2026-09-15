# Switcheroo

A macOS app for working with a lot of branches across a few big repos: see
what's on origin, put a branch into a **workspace** — a worktree or a clone —
and clean up the ones you're done with.

Built with [Tauri 2](https://tauri.app) (Rust) and Vue 3.

## Running / development

```sh
./run.sh            # dev build, reloads as you edit
./run.sh --build    # release .app, installed to /Applications and launched
```

The script checks for `pnpm`, `cargo` and `git`, puts `~/.cargo/bin` on `PATH`,
runs `pnpm install` the first time, and hands over to `pnpm tauri dev`.

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

A place a branch lives on disk is a **workspace**. It is always one of two
things: a linked **worktree** (cheap, shares the object store) or a full
**clone**. The project's own clone — the one you added to Switcheroo — is the
**main workspace**, and the other two hang off it.

That worktree/clone distinction only matters twice:

- **Creating one.** Git allows a branch in exactly one worktree per clone. When
  the branch you want already has a worktree, Switcheroo says so and offers a
  clone instead — which is what an `app-b` directory next to `app` really is.
- **Removing one.** A worktree is unregistered with `git worktree remove`; a
  clone is just deleted.

Everywhere else they are the same thing, and the UI treats them that way — the
New workspace dialog spells the choice out, and every workspace card names its
own kind.

A clone is an independent repository with its own refs, so a branch sitting in
one may not exist in the main workspace at all. Those branches are added to the
list from the workspace itself. Only branches a workspace actually has checked
out are added — merging every ref from every clone would drag in each clone's
stale `origin/*` copies and resurrect hundreds of dead branches.

## Layout

```
projects       branches (origin-first)        the selected branch
─────────────  ─────────────────────────────  ──────────────────────
acme-app       fix/login-redirect             commit, author, drift
web-portal       #1042 ● open  ⑂      2d      pull request + checks
docs-site      search-index                   every workspace of it
                 #1037 ● open  ⑂      5d      open in… / remove
               ci/cache-deps
                 merged  ⑂  stale    9d
```

Tabs: **Active** (open PR, has a workspace, or committed to recently) · **Open
PRs** · **Assigned** (open PRs assigned to you) · **Workspaces** · **All**.

Search always looks across every branch regardless of the tab, and matches
branch names, commit subjects, authors, PR numbers and titles, and **workspace
directory paths** — so a clone is findable by its folder name (`app-b`) even
when its branch exists nowhere else.

`⌘F` search (or just start typing) · `⌘R` fetch now · `⌘N` new workspace of the selected branch.

## Creating a workspace

The dialog suggests a directory from the repo's path template
(`{parent}/{project}-{slug}`, where the slug is the branch's last segment prefixed
with its PR number; `{branch}` and `{pr}` are available too) and lets you edit
it. Setup steps are offered as checkboxes, so each one is a per-workspace
decision rather than a policy.

A step is just a shell command run inside the new workspace, with `{primary}`
(the main workspace's path), `{path}` and `{branch}` substituted. When a project
is added, its recipe is guessed from what the directory actually contains:

- **copy untracked env files** — `.env` and friends out of the main workspace.
  Only untracked ones: a committed `.env.cypress` differs between branches, and
  overwriting it would silently revert what that branch committed.
- **symlink dependencies** — point `vendor` / `node_modules` at the main
  workspace
- **install** — `composer install`, `pnpm install` / `yarn` / `npm`, whichever
  the lockfiles call for

Everything stays editable afterwards, and you can add steps of your own.

Output streams into a log pane. A branch that does not exist yet is created
with `--no-track`, so your first `git push` goes to its own branch instead of
the one it was cut from.

Clones are made with `git clone --local` from the main workspace, which
hardlinks the object store, then `origin` is pointed back at the real remote. A
second copy of a large repo therefore costs seconds rather than a full network
clone.

## Cleanup

The branch list's menu has two sweeps, each counting what it would offer:

- **Cleanup workspaces** — workspaces of finished branches (merged, or deleted
  on origin), plus directories that are not git workspaces at all. Removing a
  worktree can delete its branch along with it. The main workspace is never
  offered.
- **Cleanup branches** — local branches with no workspace *and* nothing on
  origin: what a merge plus a worktree removal strands. Nothing points at
  them, so no workspace removal can ever offer them. Ones carrying commits of
  their own start unticked. A single branch can also be deleted straight from
  its right-click menu.

Each row shows what removing it would destroy — uncommitted changes, unpushed
commits, stashes, size on disk. Nothing is blocked; you just get to see the
bill first.

## Configuration

App settings live in
`~/Library/Application Support/com.jmversteeg.switcheroo/config.json` and are
edited in the settings pane. A repo may also commit a `.switcheroo.json` at its
root, which wins field by field over the stored config —
`baseBranch`, `pathTemplate`, `setupSteps` and `openTargets`:

```json
{
  "baseBranch": "main",
  "pathTemplate": "{parent}/{project}-{slug}",
  "setupSteps": [
    {
      "id": "copy-env",
      "label": "Copy env files",
      "defaultEnabled": true,
      "script": "for f in .env .env.testing; do [ -e {primary}/$f ] && cp {primary}/$f .; done"
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

Don't wrap a placeholder in quotes of your own — substituted values are already
shell-quoted, and `"{path}"` would embed literal quote characters.

Warp has no URI action that runs a command, so starting something in a Warp tab
means writing a launch configuration and opening that:

```
d=~/.warp/launch_configurations; mkdir -p $d;
find $d -maxdepth 1 -name "switcheroo-*.yaml" -delete 2>/dev/null;
n=switcheroo-$(date +%s)-$RANDOM;
printf "name: %s\nwindows:\n  - tabs:\n      - layout:\n          cwd: %s\n          commands:\n            - exec: claude\n" $n {path} > $d/$n.yaml && open warp://launch/$n
```

The unique name matters: Warp caches launch configurations by name, so reusing
one filename makes it replay the stale copy and appear to do nothing. Old files
are pruned with `find` rather than `rm`, because zsh aborts on a glob that
matches nothing even under `rm -f`.

## Development

Needs [Rust](https://rustup.rs) and [pnpm](https://pnpm.io).

```sh
./run.sh              # install if needed, then run it
./run.sh --build      # release .app -> /Applications, launched
pnpm tauri dev        # what run.sh ends up calling
pnpm tauri build      # -> src-tauri/target/release/bundle/macos/Switcheroo.app
pnpm typecheck        # vue-tsc
pnpm test             # cargo test
pnpm lint             # cargo clippy
```

`bundle.targets` is `["app"]` on purpose: DMG bundling wants Finder scripting
access and fails without it. The `.app` is the deliverable.

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
