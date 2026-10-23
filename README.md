# parallelist

A context switcher for macOS that keeps your attention in one place while you
switch between projects, branches and shells all day. Every project, every
branch on origin, every workspace holding one and every shell running there sit
in one window, which shows what needs you and gets you there in a keystroke. A
mark means something needs your attention, and nothing else gets one.

The shells run whatever you like. Coding agents such as Claude Code are the
obvious case, and the app shows when one is working, waiting on you or done,
but a dev server or a plain shell is just as at home. Around them is an
ordinary Mac window, with lists to scan and things to click.

It also manages the mess this leaves on disk: put a branch into a
**workspace** (a git worktree or a clone), see what each one holds, and clean
up the ones you're done with.

Built with [Tauri 2](https://tauri.app) and Vue 3.

## Features

- **A terminal per workspace**, built in, that keeps running while you look
  elsewhere. Claude Code running in one reports whether it is working, blocked
  or finished, and the app marks it.
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
  doesn't have, stashes and size on disk, before anything is removed.

## Requirements

- macOS
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
```

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
