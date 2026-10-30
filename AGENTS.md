# parallelist — notes for agents

A context switcher (Tauri 2 + Vue 3) that keeps the user's attention in
one place while they work across several projects, dozens of branches, the
worktrees and clones that hold them, and the shells running in those. The
user's attention is the resource the app is built to save. Most of the
branches are short-lived: a workspace is made, used for one thing and
removed, and the app aims to make each end of that take seconds. This file
records the things that are **not** obvious from reading the code: why the
model is shaped the way it is, and which apparently-redundant details are
load-bearing.

Start with `README.md` for what the app does. This file is about working on it.

## The problem it solves: attention

The app is about **attention**. The user's attention is the scarcest
resource in the setup, more than machine time or disk, and the app exists
to use it well. The user switches context all the time, across several
projects, dozens of branches and a handful of shells at once, many of them
running Claude Code, which works on its own for a while and then needs an
answer. Each switch used to mean a different terminal window, IDE or
browser tab, and a moment spent remembering where things were.

Ideally the user spends the day on **decisions**: answering a question a
program asked, looking over a result, approving a diff, starting or
finishing a branch. Finding the right window, remembering where things
were, waiting for a clone and working out whether a directory is safe to
delete are not decisions, and the app should take them over. That is how
a feature is judged. The job of parallelist is one window that shows every
project, branch, workspace and shell, says which of them needs the user
right now, and gets them there in a keystroke.

The worktree mess is where it started, and it is still one part of that job.
The user works across many branches of a few big monoliths and
constantly spins up workspaces to review or test them. Sibling directories
(`~/Code/app`, `~/Code/app-1234-fix-header`, `~/Code/app-b`) pile up until
nobody knows which branch is where, which are merged, or which are safe to
delete. That is attention lost to bookkeeping, and the app answers it so the
user doesn't have to.

### The short life of a branch

The typical unit of work is a short-lived branch: made for one thing,
worked on, merged and deleted, often within the hour and several times a
day. The target is that **a new branch with a running dev app takes
seconds to set up and seconds to remove**. If setting one up is slow, the
user skips it and does the work in a directory that is already open, on top
of something else. If removing one is risky, it stays on disk, which is how
the sibling directories piled up.

1. **Start.** New workspace (⌘N, or ⌘⇧N for the selected branch), from a
   typed name or one of the project's open issues (`BranchPicker`), which
   names the branch and the directory. Double-clicking a branch with no
   workspace skips the dialog and makes one with its defaults. The dialog closes at once: the
   worktree or clone (`git clone --local` hardlinks the object store, so a
   clone of a monolith is seconds), its setup steps (env files, linked
   dependencies, an install) and the "Open in" targets ticked in the dialog
   (an editor, a dev server or a program started in its terminal) all run
   as a task while the user carries on elsewhere.
2. **Work.** The workspace's terminal holds the work, and the marks and the
   attention queue say when it needs the user. ⌘↵ and ⌘P get there.
3. **Finish.** A merged branch, or one whose PR closed, is marked `stale`
   while it still has a workspace, and Cleanup offers the workspace and
   then the branch up, showing first what removing them would lose, so
   the user can decide from the dialog. A program that removes
   its own workspace once it has merged — Claude Code does this — is the
   same ending reached from inside, and the terminal stays until nothing
   in it needs the user.

A lot of what looks like care elsewhere in this file exists for this loop.
The removal numbers are measured so carefully (invariants 11–13) because a
throwaway workspace has to be safe to throw away without checking by hand;
creates run in the background and clean up after themselves (invariant 15)
because starting one must not cost a context switch either.

### Agents, but not only agents — and not a TUI

The app is plainly shaped by agent TUIs: Claude Code in several shells at once
is what made the switching unbearable, and it is the program whose status the
marks follow most closely. But the terminals are general-purpose. A dev
server, a REPL, a test watcher, a plain shell: whatever terminal work the user
does belongs here too. Nothing should *require* an agent, and features read
"a program in a terminal" wherever they can, rather than "Claude".

It is also a deliberate push back on the TUI hype. Everything is drifting into
the terminal lately, and GUIs were invented for a reason: a list you can
scan, a mark you can see out of the corner of your eye, a dialog that shows
what will be lost before you confirm, a click. So the terminal is for the
program running in it, and everything *around* the terminals is an ordinary
window. Don't reach for terminal idioms (text menus, key-only flows,
status drawn as characters) where a native control would do, and don't
treat the GUI as a wrapper waiting to be replaced by a TUI.

### What follows from that

Most of the behaviour described below comes from a few rules. Weigh a change
against them before anything else:

- **Decisions only.** The user's time goes to decisions: answering,
  reviewing, starting and finishing. Finding, waiting and checking by hand
  are the app's job. A feature that hands one of those back to the user
  needs a reason.
- **Seconds at both ends.** Starting a branch and finishing one are the
  most frequent steps, so they get the fewest clicks and the least waiting.
  The target is seconds for each. A change that adds a step or a wait to
  either needs a good reason.
- **One place.** Anything the user would otherwise leave the app for belongs
  in it: shells live in the detail pane rather than in an external terminal,
  task logs are a click away, and "Open in" targets run from here.
- **One list.** A piece of work is one row, whether it started as an issue,
  a pull request, a branch or a directory, and the row is where its state
  and its actions go. Don't add a second list, panel or badge for
  something a row can show.
- **Say what is waiting, and nothing else.** A mark means "this needs you":
  orange while a program is blocked, blue while a finished turn hasn't been
  replied to, a task box that a failure or an unseen finished create takes
  over until it is dealt with. Looking at a terminal clears nothing; typing
  into it does. A glance is not a decision: a turn read on the way to
  something else still waits for its reply, and dropping it from the queue
  because it was on screen for a moment is how work got lost. Noise is a
  cost: output alone never counts as activity, and a stray `.DS_Store` is
  not uncommitted work.
- **Getting there is one keystroke.** ⌘P reaches any branch of any project
  with what's waiting ranked first, and ⌘↵ goes straight to whatever has
  waited longest. A shown terminal takes the keyboard without needing a
  click first.
- **Never steal attention.** Work the user started carries on without them
  (dialogs close at once and become tasks), and when it finishes it doesn't
  pull them back. A terminal target for a branch they have left starts
  without being shown, and rows don't re-sort under the pointer.
- **Trust is part of it.** A badge that is sometimes wrong means checking by
  hand, which is the context switch the app exists to save. That is why
  unknown is never clean and "unpushed" is measured so carefully.

## Domain model

**Origin-first.** The branch list is *origin's* branches, kept fresh by a
background fetch. Having a branch on disk locally is a side effect, not the
source of truth. Filters, sorting and staleness all follow from this.

**One row per piece of work.** Issues, pull requests, branches and
workspaces are not four lists. The branch list has one `Row` per branch
(`src/store.ts`), and the row carries everything about that piece of work:
the pull request with its review and CI state, the workspaces holding the
branch, the terminals running in them, and whether it is `stale`. An issue
enters the same way: picking one in the New workspace dialog uses its linked
branch or names one from its title, and from then on it is a row. There is
no separate PR view or workspace view, only filters over the rows (Active,
Open PRs, Assigned, Workspaces, All), and that list is the user's to-do
list: Assigned is what is asked of them, Active is what is in flight, a
stale row is done and waiting for Cleanup, and a row disappears when its
branch does. So a piece of work is never tracked in two places, and
anything that belongs to one (a status, a mark, an action) goes on its row.
Issues that have no branch yet are the one exception today: they appear
only in the dialog's picker, not as rows (see Open items).

**"Workspace" is the central noun** — any directory holding a branch, whether a
linked worktree or a full clone. It is the only word the UI uses for the idea:
there is no "checkout" and no "checked out" in user-facing copy, and a branch
either *has a workspace* or it doesn't. The project's own clone is the **main
workspace**; worktrees and secondary clones hang off it.

Worktree and clone are treated identically on purpose, but the pair is named
wherever a user is choosing or looking at one — the New workspace dialog's
"Kind" field spells the two out, and every workspace card is headed
"Main workspace" / "Workspace · worktree" / "Workspace · clone"
(`WORKSPACE_TITLES` in `src/format.ts`). The distinction *behaves* differently
in exactly two places:

- **Creating.** Git allows a branch in exactly one worktree per clone. When that
  conflicts, the dialog offers a clone instead. This is the whole reason a
  `repo-b` directory exists next to `repo` in the first place.
- **Removing.** `git worktree remove` versus `rm -rf`.

Clones are made with `git clone --local` from the main workspace, which hardlinks
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

3. **`workspace::list` is not `workspace::scan`.** `list` is the cheap
   discovery (one `git worktree list` per clone); `scan` adds a `git status`
   per workspace. Anything that only needs kinds and branches — the removal
   impacts, for one — uses `list`, rather than paying for the full status of
   every workspace to look up the few it was asked about.

4. **Working trees are inspected on a small thread pool.** Sequential
   `git status` across 16 workspaces was ~3.8s; in parallel it is ~0.55s. On a
   monolith with `vendor/` and `node_modules/`, each `git status` is ~140ms.

5. **A branch sitting in a secondary clone is added from that workspace.**
   A clone has its own ref namespace and the main workspace may never have seen
   the branch, so such workspaces would otherwise have no row and vanish from
   the UI while still being counted under Cleanup. **Only the branch a
   workspace actually has checked out is added** — merging every ref from every
   clone would drag in that clone's stale `origin/*` snapshot and resurrect
   hundreds of long-dead branches.

6. **New branches are created with `--no-track`.** Otherwise git sets the
   upstream to the start point and a later bare `git push` lands on the branch it
   was cut from. This is a standing rule in the user's global `CLAUDE.md`.

7. **`-webkit-app-region` does nothing in WKWebView** — it is an Electron
   feature. Window dragging uses `data-tauri-drag-region="deep"` and requires
   `core:window:allow-start-dragging`, which is **not** part of `core:default`
   and must be granted explicitly in `capabilities/`. Without it the attribute
   is a silent no-op. A `"deep"` region skips clickable descendants
   (buttons, inputs, anything with `role="button"` or a `tabindex`) — that is
   what lets the whole sidebar drag the window while its project rows still
   click and reorder. Anything *non*-clickable that needs pointer events
   inside a deep region must opt out with `data-tauri-drag-region="false"`.

8. **Modified and untracked files are counted separately.** `git status
   --porcelain` lists both, and conflating them overstates the case badly: a
   working tree whose only "change" is a stray `.DS_Store` gets reported as
   having uncommitted work, while the user's IDE correctly shows it as clean.
   Only `modified` and `unpushed` drive the warning badges and the removal
   impact dialog.

9. **Every worktree git command runs in the clone that owns the worktree**,
   which `workspace::owning_clone` resolves — not in the main workspace. Git
   keeps one worktree registry per clone, so a worktree of a *secondary* clone
   is invisible to the main one: `git worktree remove` there fails with "is not
   a working tree" and `git worktree prune` there prunes nothing. That left the
   registration alive after the directory was deleted, `scan()` read it back out
   of `git worktree list`, and the workspace reappeared on every refresh while
   the task reported success — the removal path swallowed the error. A surviving
   registration is now a failure, and it is not cosmetic: git refuses to re-add
   a worktree at a "missing but already registered" path, so a stale entry also
   blocks re-creating the workspace. Covered by
   `removes_a_worktree_that_hangs_off_a_secondary_clone`.

10. **Paths from git are compared resolved** (`workspace::resolve`). Git reports
    symlinks resolved — on macOS `/var/...` comes back as `/private/var/...` —
    while paths in `config.json` are however the user typed them, so comparing
    them raw is a false mismatch. `resolve` keeps the leaf as written, so a
    directory that has already been deleted still compares.

11. **"Unpushed" means origin has the commit in no form** (`status::unpushed`).
    Counting `@{u}..HEAD` was wrong both ways on the user's real workspaces:
    it reported commits that a rebase on another machine had put on origin
    under new hashes, and commits merged to origin through another branch.
    And it reported *zero* for branches that had never been pushed, because
    they had no upstream to compare with — so Cleanup offered real work up
    as costing "nothing". Now a commit counts only if it is on no
    `origin/*` ref, is not a `--cherry-pick` copy of one on the upstream or
    base branch, and its changes aren't already all in the base branch (a
    `merge-tree` that changes nothing — this is what catches squash merges).
    "Behind" is neutral information, never a warning. Each filter has a
    test in `status.rs` modelled on the workspace that needed it.

12. **Unknown is never clean.** When git can't be asked about a workspace
    (missing directory, timeout, corrupt index), its status is `null` with
    a `statusError`, shown as "status unknown" and treated as at risk.
    Defaulting the counts to zero is how a broken workspace used to look
    perfectly clean. `null` with *no* `statusError` means "not asked yet":
    the `peek` that fills the list while a project's first refresh runs
    (local refs plus `workspace::list`, ~0.3s against several seconds). It
    gets a quiet "checking…" on the card and no row badge, but `atRisk`
    still treats it as at risk.

13. **Removal impact is counted per kind**, because the kinds lose different
    things. A worktree's stashes live in its clone (`refs/stash` is shared)
    and survive it; its commits survive on its branch unless that branch is
    deleted too. A clone takes every branch it has, its stashes, and strands
    any worktrees registered against it (one real clone held work on dozens of
    branches while its card counted 1).

14. **Fetching fetches secondary clones too.** A clone's `origin/*` only
    moves when something fetches in that clone, and every status measured
    there is against it. Left to the user, one was seven weeks stale.

15. **A failed create removes what it made**, and a clone of a branch that
    was never pushed takes it from the main workspace *before* repointing
    origin. Right after `git clone --local` the branch is visible as
    `origin/<branch>`, because the main workspace was the origin. The
    `fetch --prune` from the real remote then deletes that ref, the checkout
    fails, and the directory used to stay behind holding only `.git`. A
    never-checked-out clone like that reads as a workspace with every file
    deleted, and it takes the suggested path,
    so each retry landed one suffix further on: `-b`, then `-c`. Covered by
    `clones_a_branch_that_was_never_pushed` and
    `a_failed_create_leaves_nothing_behind`.


## Conventions

- **The app is described in three places, and they change together.**
  `README.md`, the landing page in `site/` and this file each say what the
  app does, what it needs and how to build it. A change to any of those —
  a feature added, renamed or dropped, a requirement, a `run.sh` flag, what
  a mark means — updates all three in the same commit, not later. The site
  also copies things from the app that won't follow it on their own:
  `site/icon.svg` and `site/favicon.svg` are `src-tauri/icons/icon.svg` and
  `icon-small.svg`, and the small marks in its "What needs you" table are
  drawn by hand from `TerminalMark.vue`'s colours. Its Figure 1 is the app
  itself: the real frontend built for a browser (see **The demo** below), so
  it follows the app on its own. The site deploys from `main` through
  `.github/workflows/pages.yml` whenever `site/`, `src/` or `demo/` changes,
  and that workflow builds `site/demo`, which is not committed.
- **The UI says "Project"; the code says "repo"** (`RepoConfig`, `repoId`,
  `add_repo`, the `repos` key in `config.json`). This is deliberate — renaming
  the internals is churn with a config-migration risk and none of it is visible.
  Don't "fix" the inconsistency without being asked.
- **The UI says "main workspace"; the wire says `primary`.** `WorkspaceKind`
  serialises that variant as `'primary'`, and setup steps and "Open in" commands
  substitute `{primary}` for its path. Both are load-bearing: the placeholder is
  baked into the user's saved `config.json` and into `.parallelist.json` files
  committed inside repos, which the app can never rewrite. Renaming it without
  an alias fails *silently* — `.replace("{primary}", …)` stops matching and the
  literal text reaches the shell. Same rule as above: don't "fix" it unasked.
  Note `OpenTarget.primary` is a third, unrelated meaning ("show as a button").
- **Slow work is a task, and tasks live in the backend.** Refreshing,
  creating a workspace, removing one and deleting a branch each run as a task
  in `src-tauri/src/tasks.rs`: a title, a status and a log. The registry is
  Rust-side so a task and its log survive a webview reload mid-install;
  `src/tasks.ts` mirrors it from `list_tasks` plus the `task` / `task-line` /
  `task-removed` events (header revisions and line numbers make the events
  that race the listing harmless). The `TaskBox` sits in the right end of
  the attention queue's strip, without a card of its own, over a backdrop
  of the strip's colour that fades in from the left (so cards running on
  under it are covered, not shown through). With no cards in the strip it
  moves to the middle of it instead, with no backdrop, in place of the
  logo that otherwise sits there, its frames lighting up one after another. It features one task
  (anything asking for attention, else the oldest running) with a `+N`
  count; clicking it opens `TasksDialog`. The box animates with `motion-v` (enter,
  sinking exit, its lines via `layout`), chosen over hand-rolled CSS. That
  doubled the JS bundle (~48 → ~95 KB gzipped), accepted for an app loaded from
  disk; `LazyMotion` + `m` was measured and saves nothing, because the package
  entry pulls in the whole engine. **Tasks cannot be cancelled** — decided,
  not overlooked. Scheduling, also decided:
  - Refreshes of one project are **coalesced** (`Coalesced` in `tasks.rs`): a
    request during a run waits for the *next* run, shared by everyone who
    asked meanwhile, with `fetch` or-ed across them. It can't reuse the run in
    flight — that may predate whatever prompted the request.
  - Removals and branch deletions run **one at a time per project** (a lane):
    concurrent `git branch -D` trips over `packed-refs.lock`, and parallel
    `rm -rf` of `vendor/` just fights over the disk. Creates run in parallel.
  - History keeps the newest successful refresh per project, up to 20 failed
    refreshes, and the newest 20 of everything else. A failure asks for
    attention until dismissed; a newer refresh of the same project quiets an
    older refresh failure.
- **A create outlives its dialog.** `CreateDialog` only builds the request
  and closes. The task is drawn as a pending card in `DetailPane` (keyed on
  project + branch, so it shows even for a branch no ref knows about yet —
  `refresh` keeps such a selection alive via `hasPendingCreate`). The pane
  renders real workspaces and create tasks as one list keyed by path: a
  finished create keeps its card, marked ready, until a refresh *asked for
  after it finished* has landed (`state.overviewAt`), and Vue then patches the
  element in place. Hiding it on completion left the slot empty for as long as
  the refresh took; keeping it without the cutoff resurrected it as "ready"
  once its workspace was later removed. When the refresh lands and the real
  workspace is not listed on the selected row, the frontend flags the task for
  attention and the box shows it as "Ready" with the "Open in" buttons. "Off
  screen" means not listed, not merely navigated away, so a refresh that misses
  the new directory still produces the notice.
  The "Open in" targets ticked in the dialog travel on the task
  (`openTargets`), so a webview reload mid-create still runs them, and the
  webview runs them when the task turns done — it is where terminal targets
  run anyway. `tasks.ts` gets that hook from `startTaskSync` instead of
  importing `openIn.ts`, which would close the store → tasks → terminal →
  store cycle described under terminals. A terminal target is shown only if
  the user is still on that branch; otherwise its shell starts unseen
  (`runInTerminal` with `reveal` off), because jumping there would also hand
  it the keyboard mid-sentence in some other terminal. A shown one starts
  at once: the pane mounts a terminal only for a listed workspace, so it
  also accepts a done create that still has its card, and the shell no
  longer waits seconds for the refresh to list it.
- **A removal greys its card instead of hiding it.** The Cleanup dialog is
  only the review step; confirming queues one task per item and closes. The
  card stays, marked "removing", until the refresh drops it, and comes back
  to life with "removal failed" if the task fails.
- **`src/status.ts` is the only place a workspace-state badge is defined.**
  The branch list, the workspace card and Cleanup each used to spell their
  own, and drifted: the list said "uncommitted" for unpushed commits while
  the card beside it said "unpushed". The numbers behind them come from
  `src-tauri/src/status.rs`.
- **`src/components/Icon.vue` is the only place a glyph is defined.** Call sites
  ask for a semantic name (`<Icon name="worktree" />`), never a picture, so the
  icon set can be swapped by editing one file.
- **Terminals live in the detail pane, one per workspace.** Clicking a
  workspace card swaps the pane to that workspace's terminal; every button on
  the card stops its click. What each branch shows (overview or which
  terminal) is remembered per project + branch, in memory only.
  - The shells are a backend registry (`src-tauri/src/terminal.rs`, portable-pty),
    keyed by workspace path, for the same reason tasks are: a webview reload
    must not orphan a shell running a dev server. Each keeps a 512 KB backlog;
    opening a live one attaches and replays it instead of starting another.
  - The webview side (`src/terminal.ts`, xterm 6 with the default DOM
    renderer) gives each session its own element, `open()`ed once and
    *re-parented* between hosts, never rebuilt — that is what keeps
    scrollback and selection across switching branches and projects.
  - Output travels on a Tauri `Channel` as raw bytes. Input is **one write in
    flight per terminal**, with keystrokes typed meanwhile batched into the
    next: a write per keystroke as independent invokes arrived out of order
    (`$TERM` became `$TREM`).
  - The shell is the user's login shell with `PATH` from the login shell, the
    launching terminal's identity and the build tooling's variables stripped,
    and `LANG` filled in when a Dock launch left it empty.
  - Closing sends SIGHUP to the shell; the kernel hangs up on its foreground
    job when it exits (tested — an explicit `killpg` made no difference).
    Removing a workspace closes its terminal first.
  - **Quitting with shells running asks first.** ⌘Q and closing the last
    window both reach `RunEvent::ExitRequested`; with any terminal in the
    registry the backend calls `prevent_exit` and emits `confirm-quit` (shells
    running, and how many of them wait), and `ConfirmQuitDialog` offers Cancel
    or Quit. Quit invokes `quit_app`, which sets `quit_confirmed` and exits,
    so the second request goes through. With no shell the app quits at once.
  - **A workspace removed from inside its own terminal** — Claude Code
    does this routinely once it has merged its branch — keeps the
    terminal until nothing in it needs the user: not while its program
    works or is blocked, not while a turn that ended there awaits a reply
    (so a keystroke is what lets one go), not while it is on screen
    (`Terminals::reap`, asked on refresh, on leaving
    a terminal and when its program goes quiet). Until then it is listed
    by its home (below), and its card shows greyed as "removed". Before
    this the shell lived on unreachable: the project lit up with a
    terminal no row could show.
    "On screen" is reported apart from focus, so switching apps isn't
    leaving it. "Removed" means the directory no longer holds a workspace
    (no `.git`), not that it is gone: a dev server left running in a removed
    worktree recreates its cache folder there, and a bare directory like that
    used to keep the terminal, and its card in the queue, for good.
  - **A terminal belongs to its directory, and the branch follows.** A
    branch renamed or switched from inside a terminal is only seen at the
    next overview, so a queue card can still go to the old name. When an
    overview lands (`land` in `store.ts`), what each branch shows and each
    terminal's home move to the branch their workspace is on now
    (`followBranches`), and the selection goes along if it was on that
    terminal, or on a branch that no longer exists. Before this the refresh
    dropped the selection and left "No branch selected", or the overview of
    the old name if it was still on origin. Rows stay keyed by branch name,
    since a row is a branch; only what hangs off it follows the directory.
  - **Every running terminal has a row, by construction.** Each has a
    *home*, set when its shell starts and kept up by `followBranches`: the
    project, the branch last seen in its workspace, and the workspace's
    kind (`terminal::Home`, on every
    `TerminalInfo`; `running` in `terminalState.ts` maps path → home, so
    nothing can be running without one). `branchesOf` in `store.ts` is the
    one place terminals are put on branches: where the overview has the
    workspace on a branch, there; otherwise on its home branch — as itself
    if listed without one (a detached HEAD, mid-rebase), as `unlisted` if
    not listed at all — with a row made up for a branch no ref has
    (neither local nor remote: "branch deleted", or not created yet). The
    branch list, the palette and the attention queue all read it, the
    queue through the palette's entries, which for the project on screen
    come from the very overview the list shows. The backend used to keep
    how each terminal's workspace last looked and put removed ones back
    into the overview; a terminal opened after the last refresh had no
    "last look", and waited in the queue as "Looking…" with no row
    anywhere. Don't reintroduce a second source for where a terminal is.
  - "Active", which the project list, branch list and cards mark, means the
    shell is running.
  - **Claude Code's status comes from OSC 7501** (its "Program Status
    Protocol"). It stays silent unless the terminal answers its
    `OSC 7501 ; ?` probe at startup, so the backend reader answers it and
    then follows the status sequences (`idle`, `working`, `done`, `blocked`,
    `clear`, observed by driving a real `claude` in a pty). Parsing is
    backend-side so status survives reloads and covers terminals never
    shown. A program killed outright never sends `clear`; the status is
    dropped once the shell leads the foreground process group again.
    `working` is what the blinking terminal glyph means. A box around it
    says something waits for the user: orange while `blocked` (on an answer,
    seen or not, since it stays stuck until answered), blue while **ready**.
    Ready is a backend flag, set whenever `done` arrives, on screen or not,
    and cleared by typing into the terminal (`Terminals::write`; a focus
    report xterm sends on its own doesn't count, as it doesn't for
    activity). Not by looking at it: it used to be, and a turn glanced at
    while on the way elsewhere fell out of the queue without a reply. The
    frontend still reports which terminal is on screen
    (`terminal_viewing`), but only so a removed workspace's terminal can be
    reaped on leaving it; window focus is no longer reported at all. The
    row menu's "Mark unread" sets the flag by hand.
    A project or row with several terminals combines them part by part:
    blinks if any works, orange if any is blocked, else blue if any is
    ready (`terminalMark`, the one place that is decided, drawn by
    `TerminalMark.vue`).
  - **Running terminals sort first**, in the palette and in the branch
    list (after the pinned main workspace) alike: blocked, then ready, then
    the rest, and within each the one **active** most recently
    (`terminalRank` in `src/terminalState.ts`, the one ordering both use).
    Active means started, a status change, or typed into — kept by the
    backend as `activeAt` — and never output, or a chatty dev server would
    sit on top for good. A focus report xterm sends on clicking in isn't
    typing either. Recency is deliberately not reactive: the branch list
    re-sorts on its own only when a tier changes, and picks up recency
    whenever it re-sorts anyway, so rows don't move under the pointer.
    When rows do change places, they slide there (`src/reorder.ts`, a
    hand-rolled FLIP on the Web Animations API), so the eye can follow the
    one that moved. Only within one listing: another tab, search or project
    is a different list and just appears. It is not motion-v's `layout`,
    which animates every layout change, keystrokes in the search included,
    on a list that can be hundreds of rows.
    The mark state lives in `terminalState.ts`, apart from the xterm
    sessions, because the store sorts by it and `terminal.ts` imports the
    store: importing that back evaluates `terminal.ts` first and its
    top-level `watch` reads the store before it exists.
  - **⌘-clicking a link follows it** (`src-tauri/src/link.rs`); a plain
    click only focuses and selects, as in iTerm and VS Code. A `file:`
    link opens in Quick Look (`qlmanage -p`, one panel at a time),
    `http(s):` and `mailto:` go to `open`, and nothing else is followed.
    A file link is never `open`ed: terminal output is untrusted, and an
    OSC 8 link's text needn't resemble its target, so `open` would launch
    whatever app or script it named. Shells get `FORCE_HYPERLINK=1`,
    because programs decide whether to link by a `TERM_PROGRAM` list ours
    isn't on; with it Claude Code links `[Image #1]` in the transcript to
    the stored image (observed in a pty — only once the message is sent,
    and not in a `claude` that inherited `CLAUDE_CODE_CHILD_SESSION`,
    which stops it storing images). Inline images were looked into and
    dropped: Claude Code draws only mod images inline, by kitty graphics
    with Unicode placeholders, and only for kitty or Ghostty, which
    xterm's image addon can't render anyway.
  - Zoom is one font size for every terminal, saved as `terminalFontSize`:
    ⌘+/⌘−/⌘0 while a terminal is on screen, or the header's − 100% +.
  - A terminal takes the keyboard whenever it is shown, including by
    selecting its branch again, unless an input has focus or a dialog is
    up (`mayTakeFocus`). Having to click into it first was the complaint.
- **The command palette (⌘P) searches every project's branches** without
  refreshing them: each project lists the newer of its cached overview and
  a `peek`, so it has no PRs and doesn't search them. ⌘P rather than ⌘K
  because terminals use ⌘K to clear. Ranking is fzy's score (`src/fuzzy.ts`,
  about a point per well-placed character) plus a flat bonus — 2 to 4.5 by
  terminal tier, else one for the current project — so those settle short
  queries and barely touch long ones, and an empty query opens on running
  terminals first. Recency only breaks ties, so it never beats a better
  match. Going to a branch clears a search or switches to All when the
  list would otherwise hide it.
- **The attention queue** (the AQ; `src/attention.ts`, `AttentionQueue.vue`) is a
  strip under all three columns with one card per terminal waiting for the
  user — blocked, or ready — in the order they started waiting: oldest on
  the left, newcomers on the right. First in, first out and nothing else;
  unlike the palette, blocked doesn't jump ahead of ready. The backend
  keeps when each wait began (`waitingSince`, set by `Status::settle`) so
  a reload keeps the order; blocked turning into ready is the same wait,
  and a terminal that starts waiting again joins at the back. The terminal
  on screen keeps its card (a blocked one waits there until answered, a
  ready one until typed into), marked "here", and ⌘↵ goes to the first
  card past it. The strip is there
  even when empty, so the panes above never change height and the terminal
  in them never reflows as cards come and go. Its cards find their branch
  through the palette's entries, placed as the list places them (above).
  xterm sends ⌘↵ as a plain Return — it would submit a Claude Code prompt —
  so each terminal's `attachCustomKeyEventHandler` lets it through to the
  window's handler instead. Over a dialog ⌘↵ is left alone.
  Behind the queue, past a divider, are the terminals that are *working*,
  oldest first: never where ⌘↵ goes, but what joins the queue next. The two
  are one keyed list, so a turn ending unseen slides its card from the head
  of the working line to the back of the queue instead of replacing it.
  A card is two lines: its workspace's directory name — what the user knows
  the terminal by, and it usually names the project too — with how long it
  has waited, and a line that says what is going on. A name of the form
  `<project>-<rest>`, where the rest shares a word with the branch, shows
  the rest in bold and the project after it in normal weight, truncated
  first (`splitWorkspaceName` in `src/format.ts`); a clone suffix like
  `app-b` shares none and shows as it is. A working one shows the
  `msg` Claude Code sends with `working` (base64 in the payload; what it is
  doing, e.g. a Bash command's description). A waiting one shows what the
  turn ended on, which the status can't say: `done` carries no `msg`, even
  when the turn ends on a question (observed driving `claude` 2.1.309 in a
  pty). It comes from Claude Code's transcript (`src-tauri/src/transcript.rs`):
  the terminal's foreground process is Claude Code's pid,
  `~/.claude/sessions/<pid>.json` names its session and starting
  directory, and the session is `~/.claude/projects/<mangled dir>/<id>.jsonl`.
  Found by pid, not by newest file, because several sessions can share a
  directory. From its last assistant entry it takes the last question, else
  the last paragraph; an `AskUserQuestion` call's question; or, for a tool
  call waiting on permission, what the call is for. This is Claude Code's
  private layout, so anything missing is just no line. The webview asks
  for it twice per wait (`terminal_question`), since `done` can arrive
  before the transcript has the turn.
- **With no project, an onboarding screen takes the place of all three
  columns** (`Onboarding.vue`, chosen in `App.vue` by an empty
  `config.projects`): a project list with nothing in it, a branch list and a
  detail pane whose empty states ("Select a project", "Pick a branch") had
  nothing to say. The AQ goes too, its row with it (`--queue-h` is 0), as do
  the splitters, and the title bar strip widens (`.titlebar.wide`) so the settings gear sits at the window's corner
  rather than over where the sidebar was. It carries its own error
  banner, because a folder git rejects lands in `state.error`, which only the
  branch list showed. `addProject` is in the store so the sidebar's strip and
  the screen's button are one path.
- **A folder dropped on the window goes where it belongs** (`open_folder`):
  the project it is the main workspace of, else the branch of the workspace
  it is, else it is added as a project. A subfolder counts as its workspace,
  and a worktree of a clone nobody added brings in the clone. "Add project"
  goes through the same command, so picking a worktree there works too.
  Only projects sharing the folder's origin are listed (`workspace::locate`):
  asking all thirteen of the user's made a miss take ~0.9s. Tauri's native
  drop handler takes the drop, so the page never sees `dragover`/`drop`;
  `onDragDropEvent` (`onDragDrop` in `api.ts`, used by `DropTargets.vue`) is
  the only way in, and where the pointer is comes from its positions. On
  macOS those are CSS pixels although Tauri types them as physical (wry
  passes AppKit's points on unscaled), so they are not divided by the
  device pixel ratio.
- **A drag shows where it can go** (`DropTargets.vue`). Its `enter` carries
  the paths, and `are_folders` says what they are. Folders get two areas,
  each from the top of the window down to the attention queue: one over
  the sidebar and branch list that goes to the folder or adds it, and one
  over the detail pane while it shows a terminal, which pastes the path.
  With no terminal on screen the first takes the whole width. Anything with
  a file in it gets only the terminal's area, and dropped elsewhere does
  nothing. The paste (`pastePaths` in `terminal.ts`) is backslash-escaped
  and space-separated as Terminal.app types it, and a paste rather than
  keystrokes, so a program in bracketed-paste mode sees one: Claude Code
  turns a pasted image path into an attachment. It counts as typing, so it
  answers a ready turn. The areas are measured on every drag event (the
  terminal's `.pane`, the `.attention-queue`'s top), not laid out in CSS,
  so what is drawn is what the pointer is tested against.
- **⌘N and ⌘⇧N are in two places on purpose.** The File menu's
  "Add Branch…" (⌘N, the field empty, as "+ Add branch") and
  "New Workspace…" (⌘⇧N, the selected branch, as a card's "+ New
  workspace") show the keys where macOS users look for them, and
  `set_create_menu` greys each out while it would do nothing. `onKey` in
  `App.vue` handles the same keys, because WKWebView offers a key to the
  page before the menu, and the demo has no menu at all. Both land in
  `newFromShortcut`, so whichever gets the key, the result is the same.
- **Double-clicking a branch goes into its shell** (`enterShell` in
  `src/enterShell.ts`): a terminal already running on the branch, shown or
  not, takes the keyboard; else a workspace of it gets its shell; else one
  is made as the New workspace dialog would make it if opened on the branch
  and submitted at once, and its shell opens when the create is done. How
  the dialog's defaults are derived (setup steps, ticked targets, the plan,
  the request) lives in `src/newWorkspace.ts`, which the dialog and the
  double-click both use, so the two can't drift; don't derive a default
  in `CreateDialog.vue` itself. A built-in terminal target ticked by default
  runs in the new shell, in an existing workspace as after any create;
  otherwise the shell is a plain one. The shell
  is only asked for (`showTerminal`), so it starts when the pane shows it:
  a branch left meanwhile gets no shell until it is back on screen, and the
  pane holds the terminal back while a create is still setting its
  workspace up, even if a refresh has listed the directory by then.
  `enterTerminal` refocuses a terminal that is already on screen; ⌘↵ goes
  through it too.
- **The New workspace dialog's branch field is a combobox** (`BranchPicker.vue`,
  logic in `src/branchPicker.ts`) over the project's branches and its open
  issues. Its first entry is always the text as typed — "Create branch …",
  or the existing branch of that name — and Enter on it submits the dialog;
  only an entry further down is *picked* by Enter. That keeps Enter-submits
  intact. The list floats over the fields below: the old "Doesn't exist yet"
  hint pushed them down as you typed, and was replaced by a `new` tag inside
  the field. Matching is fzy, but every run of matched characters must start
  a word or be 3+ characters: plain subsequence matching over ~300 branches
  and ~300 issue titles buried the real hits (`sso` found
  `cla·ss-·o·rder` above the SSO issues). Issues come from one paginated
  `gh api graphql` (~3s for 305; `gh issue list` can't return issue types or
  linked branches), cached per project for a minute. Picking an issue
  uses its GitHub-linked branch if it has one, else names one from the
  title behind a type prefix (`fix/`, `feature/`, `maintenance/`, …) — only
  a prefix the project's branches already use — and its number leads the
  directory, as in `app-1234-fix-header`.
- **The demo** (`demo/`) is `src/` running in a browser, in an iframe on the
  landing page. `vite.config.ts` there aliases every Tauri module the
  frontend imports to a stand-in in `demo/shims/` (`invoke`, `listen`,
  `Channel`, native menus, the dialog and opener plugins), and
  `demo/backend/` answers what the Rust side would: a made-up machine
  (`world.ts`: four projects, their branches, workspaces and PRs), tasks
  with short logs, and scripted terminals (`terminals.ts`, `seed.ts`). The
  shells understand a few commands. `agent` starts a made-up coding-agent
  session, named for no real product, that reports `working` / `blocked` / `done` as the real one does
  over OSC 7501, handed to the frontend as the `terminal-status` events the
  Rust parser would have sent, so marks, the queue and the palette run
  their real code. The app's components are never forked for it, so keep
  it that way: a change that only works in Tauri belongs behind the api
  layer, where the shims can answer it. A new command in `src/api.ts`
  needs a handler in `demo/backend/index.ts`; a new event, an `emit`.
  Two things exist only because it is in a page: a terminal's focus is held
  back until the visitor has touched the frame (focusing inside an iframe
  scrolls the page to it), and what the seeded sessions do on their own
  starts when the frame scrolls into view. `pnpm dev:demo` runs it,
  `pnpm build:demo` builds it into `site/demo`.
  The component sandbox (`pnpm sandbox`) is the other place the frontend runs
  in a browser, with its own Tauri stubs: it seeds single components with
  fixtures to look at, where the demo plays one whole scripted window.
- Error messages that say "git repository" are factual statements about git and
  should stay that way, even though the UI otherwise says "project".

## Style guide

For anything a user reads: the README, the site, and copy in the app.

- **The name is "parallelist", always lowercase**, in titles, headings and at
  the start of a sentence too. The exceptions are identifiers that aren't
  copy: `productName` in `tauri.conf.json` (it names `Parallelist.app`, which
  `run.sh` and the build paths refer to) and the `TERM_PROGRAM` the terminals
  set, which programs may match on.
- **Modest, and a little self-aware, on the site and in the README.** The
  app is an attempt at a problem, not its solution: "parallelist is an
  attempt to keep that attention in one place", not "parallelist keeps your
  attention in one place". Say what it *tries* to do when describing its
  purpose; describe concrete behaviour ("the mark goes when you reply")
  plainly, without hedging every line.
- **Formal and to the point in the app.** Copy in the UI states what a thing
  is and does, without hedging or a voice of its own: the onboarding screen
  says "parallelist keeps its branches, workspaces and terminals in one
  window", not that it tries to.
- **No first person.** No "I", "we" or "my": the app isn't one person's
  story. Addressing the reader as "you" is fine.
- **A mark is not a nuisance.** A mark, a queue card or a question from a
  program is a decision for the user, and dealing with it is what they are
  there for. Copy doesn't present it as a burden or something to be spared
  ("not your problem yet", "bothering you", "leaving you alone"). Say what
  is waiting and what isn't: "waiting for an answer", "nothing to decide
  there yet". What the app protects the user from is the time between
  decisions (finding, waiting, checking by hand), which is what "never
  steal attention" refers to.
- **Plain, not clever.** Readers notice and dislike the habits of
  generated text, so avoid them: "not X, but Y" constructions, sets of
  three for rhythm, a short aphorism to close a paragraph, a sentence that
  restates the one before it, grand claims about what the app is for, and
  em-dashes used for pacing. One idea per sentence, stated once.
- **No selling.** No superlatives, no exclamation marks. If a sentence would
  be true of any app ("powerful", "seamless"), cut it.
- **Use the app's own words.** "Workspace", "main workspace", "project",
  never "checkout" or "repo" (see Conventions).
- **Let it speak for itself.** The site's demo needs no caption, no "try
  this" and no note that its terminals are scripted: if the window isn't
  obvious on sight, change the window. The same goes for anything else a
  user would be walked through. State what the app does, not how to operate
  it, and don't narrate the interface step by step ("the dialog closes",
  "click the button"): say what the user ends up with.
- **Concrete, and from the reader's side.** Describe the reader's day in
  its own terms (the shells they have open, `app-1234-fix-header` next to
  `app-b`, a program that worked for a while and now needs an answer)
  rather than the app's categories. Where a claim could be an adjective,
  make it a consequence instead: not that creating a workspace is fast,
  but that if it were slow you would skip it and work in a directory that
  is already open. Short sentences, one each, and a specific noun over a
  general one.
- **No product names on the site.** Not Claude Code, not any other agent or
  tool, in the copy or inside the demo (its made-up agent is just `agent`).
  Copy says "a program in a terminal", as the app's own wording does.

## Verifying work

**The running app cannot be screenshotted.** Screen recording permission is not
granted to the terminal and the user declined to grant it. Do not plan on visual
inspection of the running app — but its components can be looked at in the
browser sandbox, described at the end of this section.

Instead:

```sh
cd src-tauri && cargo run --example probe -- ~/Code/some-repo
```

`probe` exercises the entire git/gh layer headlessly and prints branches,
workspaces with their status, what removing each would lose, the sniffed setup
recipe, PRs, and whether every workspace resolves to a branch row. It found two
real bugs in one session — prefer it to guessing. Given a second argument it
also writes the overview as JSON, exactly as the frontend receives it, so UI
logic like `src/status.ts` can be run against real data (bundle it with the
esbuild under `node_modules/.pnpm`).

To check SVG or other rendered output, `qlmanage -t -s 900 -o <dir> file.svg`
renders through WebKit, the same engine the app uses. ImageMagick's built-in SVG
renderer is too limited to be a useful check.

### The component sandbox

The running app can't be looked at, but its components can, in a browser:
`pnpm sandbox` serves `sandbox.html` (port 1430, `PARALLELIST_SANDBOX_PORT`
to change it; use `pnpm exec vite --mode sandbox` to skip opening a tab).
Scenarios are named states of one piece of UI, listed in the left column,
shown in an iframe whose width, colour scheme and reset button are along the
top, with every backend call and component event logged underneath.

How it works, and what it leaves out:

- **Tauri is stubbed at the module boundary.** In `--mode sandbox` Vite
  aliases each `@tauri-apps/*` module the app imports to a file in
  `src/sandbox/tauri/`, all of which land in `src/sandbox/backend.ts`:
  `invoke` is answered by handlers a scenario registers (`backend.handle`),
  `listen` by `backend.emit`. A command with no handler **fails loudly**
  rather than answering with something plausible. A new `@tauri-apps/*`
  import in the app needs a stub and an entry in `stubs` in `vite.config.ts`.
  `pnpm build` and `tauri dev` never see any of this.
- **Components read singletons, so scenarios seed them the way the app
  does.** `boot()` in `src/sandbox/seed.ts` answers `get_config`, `refresh`
  and `peek` and then calls the real `loadConfig` and `refresh`;
  `terminals()` fills `terminalState`; tasks go through the real
  `startTaskSync` over `list_tasks`, and a scenario can `backend.emit` task
  events afterwards. Don't assign into the stores when the app's own path
  can be driven instead.
- **A scenario is a frame reload.** Each runs in a fresh iframe
  (`?frame=<group>/<id>`, optionally `&theme=light|dark`), so nothing leaks
  between them; Reset is a reload. `props: { open: true }` is mounted shut
  and opened once ready, because dialogs load what they show when `open`
  turns true. To add one, drop a file in `src/sandbox/scenarios/` that
  default-exports `defineGroup(...)` (from `define.ts`, not `scenarios.ts`:
  that one globs these files and importing it back is a cycle). Fixtures
  are in `fixtures.ts`.
- **Light and dark** are forced by copying `styles.css`'s `:root` values
  onto the root element (`theme.ts`), since a page can't flip
  `prefers-color-scheme`. xterm's theme still follows the OS.
- **It is Chrome, not WKWebView.** Layout is close, not proof. The native
  menu is a drawn lookalike, `open`ed dialogs return nothing, and there is
  no pty, window dragging or menu bar.
- **Screenshots.** Headless Chrome works without the browser extension:
  `Google Chrome --headless=new --window-size=W,H --virtual-time-budget=6000
  --screenshot=out.png 'http://localhost:1430/sandbox.html?frame=…'`. Under
  virtual time `requestAnimationFrame` doesn't run, so `motion-v` animations
  (the task box) are caught at their start and look faded; CSS-only ones
  (dialogs) are fine.

## Build and run

```sh
export PATH="$HOME/.cargo/bin:$PATH"   # rustup ran with --no-modify-path
pnpm install
pnpm tauri dev
pnpm tauri build      # -> src-tauri/target/release/bundle/macos/Parallelist.app
pnpm sandbox          # component sandbox in a browser, no backend needed
pnpm typecheck        # vue-tsc
pnpm test             # cargo test
pnpm lint             # cargo clippy
```

`run.sh` run from a branch other than `main` (a worktree, usually) passes
the branch name as `VITE_PARALLELIST_BRANCH`, and `App.vue` shows it in a
panel at the top of the sidebar, behind the traffic lights and the gear, on
a line under them (`--build-h` makes the room). `pnpm tauri dev` run by hand, the
demo and the sandbox don't set it, so they show nothing. A `--build` from a
branch keeps the badge in the installed `.app`.

`bundle.targets` is `["app"]` on purpose: DMG bundling needs Finder scripting
access and fails without it. The `.app` is the deliverable.

## Configuration

- App config: `~/Library/Application Support/com.jmversteeg.parallelist/config.json`
- Per-project override: `.parallelist.json` committed at a project's root, which
  wins field by field over the app config.
- **The app used to be called Switcheroo, and two traces of that are
  load-bearing.** A project without a `.parallelist.json` still has its
  `.switcheroo.json` read, because those are committed in repos the app can't
  rewrite. And when `com.jmversteeg.parallelist/config.json` doesn't exist yet,
  `AppConfig::load` copies it over from `com.jmversteeg.switcheroo/` (the bundle
  identifier names the directory, so the rename would otherwise have started
  the user from an empty project list). Keep both until asked to drop them.
- "Open in" targets and setup steps are user-editable shell templates run
  through the login shell. `{path}` is the workspace being acted on and
  `{primary}` the main workspace, in both. Path templates use `{project}` for
  the project's directory name. All substituted values are shell-quoted, so a
  template must not add quotes of its own.
- An "Open in" target's `kind` is `command` (the default: run detached, for
  launching other apps) or `terminal`: typed into the workspace's built-in
  terminal, which is then shown. A `terminal` target is disabled while that
  workspace already has a shell running, since the command would land in
  whatever it is doing; that rule lives in the frontend (`src/openIn.ts`),
  so a config only says `claude`.

## Open items

- Open issues without a branch are not rows. They are offered by the New
  workspace dialog's picker, and become a row once a workspace is made for
  one. Whether they should be listed before that, so the to-do list also
  shows work not yet started, is undecided.
- Icons are hand-drawn Lucide-style placeholders. Agreed upgrade path is
  `unplugin-icons` + `@iconify-json/*` (offline, tree-shaken); only `Icon.vue`
  changes. **react-icons is React-only and cannot be used here.**
- Right-clicking a project in the sidebar removes it immediately, with no
  confirmation and no visual hint that it is possible. Known, deliberately left
  alone, flagged to the user.
- No CI.
- The `git fetch` hang was traced to a half-open connection upstream. The timeout
  makes it recoverable; the root cause is network-side and is not fixed.
