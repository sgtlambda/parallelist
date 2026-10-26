# parallelist

A context switcher that tries to keep your attention in one place while you
switch between projects, branches and shells all day. Every project, every
branch on origin, every workspace holding one and every shell running there
sit in one window, which shows what needs you and gets you there in a
keystroke.

The idea is that your attention is the scarce resource, so it should go to
decisions: answering a question a program asked, looking over a result,
starting or finishing a branch. Finding the right window, waiting for a
clone and checking whether a directory is safe to delete are the app's job.
A mark means something is waiting for a decision, and nothing else is
marked.

Issues, pull requests, branches and workspaces are not separate lists. A
branch's row carries its pull request, its workspaces and their terminals,
and an issue becomes a row when you make a workspace for it. Filtered to
what is assigned to you or in flight, the list is the to-do list, and a row
goes when the work is merged and cleaned up.

The shells run whatever you like. Coding agents such as Claude Code are the
obvious case, and the app shows when one is working, waiting on you or done,
but a dev server or a plain shell is just as at home. Around them is an
ordinary window, with lists to scan and things to click.

Built with [Tauri 2](https://tauri.app) and Vue 3.

## A branch, start to finish

parallelist is built around branches you start and finish many times a day.
Each branch gets a **workspace**, a directory of its own holding it as a git
worktree or a full clone, so several programs can work on the same project
at once without changing each other's files. Some branches need careful
review and a trail of comments for whoever reads the history later. Others
are a way to build several things side by side while you iterate, and are
merged and removed within the hour. Both kinds need quick ends: a new
workspace with its dev app running in seconds, and gone in seconds once the
branch is done. If setting one up were slow, you would skip it and work in a
directory that is already open. If removing one felt risky, it would stay on
disk.

1. **Start.** In **New workspace** (`⌘N`), type a branch name or pick one of
   the project's open issues, which names the branch and the directory. The
   workspace is made in the background (a
   worktree, or a clone sharing the main workspace's objects when the branch
   already has a worktree, so even a large one takes seconds), its setup
   steps copy env files, link dependencies or run an install, and whatever
   you ticked to open it in starts when it's ready: an editor, a dev server,
   a program in its terminal. Double-clicking a branch with no workspace
   makes one as the dialog would with nothing changed, and opens its
   terminal once it is set up.
2. **Work.** The workspace's terminal runs whatever the work needs, and the
   marks and the attention queue say when it needs an answer or has finished.
3. **Finish.** Once the branch is merged or its pull request closed, its row
   is marked stale, and Cleanup offers up the workspace and then the branch,
   showing first what removing them would lose, so you can decide from the
   dialog. When a program removes its own workspace after merging, as Claude
   Code does, its terminal stays until you've replied to how it ended.

## Features

- **A terminal per workspace**, built in, that keeps running while you look
  elsewhere. Claude Code running in one reports whether it is working, blocked
  or finished, and the app marks it. Double-clicking a branch goes into its
  shell: the one already running there, or a new one.
- **`⌘P` jumps to any branch of any project**, with what's waiting first.
- **An attention queue** along the bottom: every terminal waiting for you,
  oldest on the left, with what Claude asked; then the ones still working.
  `⌘↵` goes to the one that has waited longest.
- **Origin-first branch list**, kept fresh by a background fetch, with pull
  request, review and CI state from `gh`.
- **New workspaces in the background**, so you carry on while they're made: a
  worktree, or a clone when the branch already has a worktree. Optional setup
  steps copy env files, symlink dependencies or run an install.
- **Cleanup that shows the bill first**: uncommitted changes, commits origin
  doesn't have in any form, stashes and size on disk, before anything is
  removed. A workspace git can't answer for shows as unknown, never as clean.

## Requirements

- `git`
- `gh`, authenticated with `gh auth login` (optional, for pull request data)
- [Rust](https://rustup.rs) and [pnpm](https://pnpm.io) to build

## Running

```sh
./run.sh            # dev build, reloads as you edit
./run.sh --build    # release .app, installed to /Applications and launched
pnpm sandbox        # components in a browser, with made-up data (src/sandbox)
pnpm typecheck      # vue-tsc
pnpm test           # cargo test
pnpm lint           # cargo clippy
pnpm dev:demo       # the landing page's demo: this frontend, in a browser
```

Run from any branch but `main`, `run.sh` puts the branch name in the top-left
of the window, so a build from a worktree can't be mistaken for the real one.

The landing page (`site/`) embeds that demo: the app's own components with
made-up projects and scripted terminals behind them (`demo/`), built into
`site/demo` by `pnpm build:demo`. A command added to `src/api.ts` needs an
answer in `demo/backend/index.ts`.

## Configuration

Settings are edited in the app and stored in
`~/Library/Application Support/com.jmversteeg.parallelist/config.json`. A
project can commit a `.parallelist.json` at its root to override `baseBranch`,
`pathTemplate`, `setupSteps` and `openTargets`.

Setup steps and "Open in" targets are shell templates run through your login
shell, with `{path}`, `{primary}` (the main workspace) and `{branch}`
substituted and shell-quoted, plus `{prUrl}` in "Open in" targets:

```
open -na Ghostty --args --working-directory={path}
phpstorm {path}
```

## License

[MIT](LICENSE)
