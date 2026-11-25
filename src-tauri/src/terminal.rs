//! Shells in pseudo-terminals, one per workspace.
//!
//! The registry lives here rather than in the webview for the same reason the
//! task registry does: a webview reload must not orphan a shell that has a dev
//! server running in it. Each terminal keeps a backlog of its recent output,
//! and opening a workspace's terminal while it is still alive attaches to it
//! and replays that backlog instead of starting another shell.
//!
//! Terminals are keyed by workspace path, because that is what the UI asks
//! for: "the terminal in this workspace".
//!
//! Programs that speak the Program Status Protocol (OSC 7501) — Claude Code
//! since 2.1 — say whether they are working, waiting on the user or done.
//! They only do so once the terminal answers their `OSC 7501 ; ?` probe at
//! startup, so the host answers it, then logs the status sequences.
//! Observed from Claude Code: `idle` once it is ready, `working` the moment
//! a prompt is sent, `done` when the turn ends, `clear` when it exits.
//!
//! **The status is worked out from the host's log** (`pty::Note`): what the
//! program printed, the foreground changing hands, the user typing, a mark
//! made by hand. Each entry is applied in the order the host numbered it
//! (`Status::apply`), whether it arrives live, after a reconnect, or when the
//! next app takes the terminal over, so all three end up with the same
//! status. What has been applied is saved with the pty as a checkpoint
//! (`Kept`), and the host keeps every entry after it.
//!
//! A turn that ends leaves the terminal **ready**: a flag, set when `done`
//! arrives and cleared by the user typing into the terminal
//! (`Terminals::write`). Not by looking at it: a turn glanced at on the way
//! elsewhere still waits for its reply, and only the reply says the user has
//! dealt with it. Whether the terminal is on screen or its window focused
//! never enters into it.
//!
//! Each terminal also keeps when it was last **active**, which is what
//! orders the palette and the branch list among running terminals: when it
//! started, its program's status changed, or the user typed into it. Not its
//! output, or a dev server's logging would keep it on top for good.
//!
//! A terminal that **waits** for the user — blocked on an answer, or ready —
//! keeps when it started to, for as long as it goes on waiting. That orders
//! the attention queue, oldest first. Going from blocked to ready is still
//! the same wait.
//!
//! A terminal can outlive its directory: Claude Code often removes its own
//! worktree once it has merged it, from inside the very shell it runs in.
//! Removing a workspace through the app closes its terminal first; one
//! removed from inside is closed **once nothing in it needs the user** —
//! not while its program works or is blocked on an answer, not while a turn
//! that ended there awaits a reply, and not while it is on screen (`reap`).
//! Until then it stays reachable through its `Home`: the project and branch
//! it was opened on, which is where the frontend lists any terminal whose
//! workspace the overview doesn't put on a branch.
//!
//! **Nothing here waits on the host while holding a lock.** The registry's
//! lock is taken to find a terminal and let go before anything is sent, and
//! what the Terminals dialog needs (the foreground of each) is already here,
//! from the log.

use portable_pty::{CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::git::WorkspaceKind;
use crate::pty::{self, append_backlog, Entry, Host, Note, Opened as HostOpened, Snapshot, Spawn, Survivor};
use crate::tasks::now_ms;

/// What a terminal sends to whoever is attached.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Data(Vec<u8>),
    /// The shell is gone; the terminal has been dropped from the registry.
    Exit(i32),
    /// The terminal relay went, and the shell with it.
    RelayGone,
    /// Another window of this build took the terminal over.
    Moved,
}

pub type Sink = Box<dyn Fn(Chunk) -> bool + Send>;

/// Shown where output was lost: more of it than the relay keeps went by while
/// the app was reconnecting.
const GAP: &[u8] = b"\r\n\x1b[2m[Some output was lost while reconnecting to the terminal relay.]\x1b[0m\r\n";

/// Where a terminal belongs, fixed when its shell starts. The frontend lists
/// a terminal on its workspace's branch, as the overview has it; this is
/// where it goes instead whenever the overview doesn't put its workspace on
/// a branch — removed from inside the terminal, on a detached HEAD
/// mid-rebase, or newer than the overview — so a running terminal always
/// has a row. The frontend keeps the branch up to date from each overview,
/// for a branch renamed or switched from inside.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Home {
    pub project_id: String,
    /// The branch it was opened on.
    pub branch: String,
    /// What its workspace was then (`workspace::kind_at`).
    pub kind: WorkspaceKind,
}

impl Home {
    /// For a terminal whose app went before it could say where it belongs.
    /// It still has to be listed, to be ended.
    fn unknown() -> Self {
        Home {
            project_id: String::new(),
            branch: String::new(),
            kind: WorkspaceKind::Primary,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub path: String,
    #[serde(flatten)]
    pub home: Home,
    /// What the program in it last reported over OSC 7501 (`working`,
    /// `idle`, `done`, `blocked`…), or `None` when nothing has, or it cleared
    /// its status, or it is gone.
    pub status: Option<String>,
    /// A turn ended, and nothing has been typed into the terminal since.
    pub ready: bool,
    /// When it was last active, in epoch milliseconds.
    pub active_at: u64,
    /// When it started waiting for the user, blocked or ready, in epoch
    /// milliseconds; `None` while it isn't.
    pub waiting_since: Option<u64>,
    /// What the program says along with its status (OSC 7501's `msg`):
    /// Claude Code sends what it is doing while `working`. `done` carries
    /// none, so what a finished turn asks comes from `transcript` instead.
    pub message: Option<String>,
}

/// What `Terminals::open` found or started.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    /// A new shell, rather than one that was already running.
    pub started: bool,
    pub terminal: TerminalInfo,
}

/// What the user can mark a terminal as from its branch's menu, as if its
/// program had said so.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    /// A turn ended: `ready`, until the terminal is typed into. Takes the
    /// place of a block, since only one of the two is shown.
    Unread,
    /// Waiting for an answer, until the program reports something else.
    Blocked,
    /// Nothing waits there any more: neither blocked nor ready. What the
    /// program reports next is taken as usual.
    Clear,
    /// Still waiting, but as if the wait had only just begun, so it goes to
    /// the back of the attention queue. Nothing to do if it isn't waiting.
    Snooze,
}

/// What the app logs with the host (`Note::App`).
#[derive(Serialize, Deserialize)]
struct AppNote {
    mark: Mark,
}

/// What the registry reports about terminals, attached or not.
pub enum TerminalEvent<'a> {
    Exit(&'a TerminalInfo),
    /// The status changed, or the terminal is newly known; `info.status` is
    /// the new one.
    Status(&'a TerminalInfo),
}

struct Output {
    backlog: Vec<u8>,
    /// Everything the shell has printed up to here has been taken: the
    /// offset the next output should start at.
    received: u64,
    /// Where output goes while someone is attached. Returns false once the
    /// receiving end is gone, and is then dropped.
    sink: Option<Sink>,
    exited: bool,
}

impl Output {
    fn deliver(&mut self, chunk: Chunk) {
        if let Some(sink) = &self.sink {
            if !sink(chunk) {
                self.sink = None;
            }
        }
    }

    /// Take output from `offset` on: what is new of it, after a mark where
    /// some went missing in between.
    fn receive(&mut self, offset: u64, data: &[u8]) {
        let end = offset + data.len() as u64;
        if end <= self.received {
            return;
        }
        if offset > self.received {
            append_backlog(&mut self.backlog, GAP);
            self.deliver(Chunk::Data(GAP.to_vec()));
        }
        let fresh = &data[self.received.saturating_sub(offset) as usize..];
        append_backlog(&mut self.backlog, fresh);
        self.received = end;
        self.deliver(Chunk::Data(fresh.to_vec()));
    }
}

struct Terminal {
    /// The host's name for its pty.
    id: String,
    info: TerminalInfo,
    host: Arc<dyn Host>,
    output: Mutex<Output>,
    /// The shell's pid. While it leads the foreground process group, nothing
    /// else is running, so no program status can be current.
    shell_pid: Mutex<Option<i32>>,
    status: Mutex<Status>,
    /// Registered, but the host hasn't said yet whether it started a shell.
    pending: AtomicBool,
    /// While pending, another open took it over and is waiting on it too.
    shared_open: AtomicBool,
    /// Closed while pending: hang up once the shell has started.
    close_asked: AtomicBool,
    /// The size last asked of the host, which screen search replays the
    /// backlog at (`Terminals::screens`). `None` for one taken over from an
    /// earlier app until a window attaches to it.
    size: Mutex<Option<PtySize>>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Status {
    state: Option<String>,
    message: Option<String>,
    ready: bool,
    active_at: u64,
    waiting_since: Option<u64>,
    /// The process leading the foreground, as last logged.
    leader: Option<i32>,
    /// The last entry of the host's log applied.
    seq: u64,
}

impl Status {
    /// Start the wait when it begins and end it when it does, after
    /// anything that changes `state` or `ready`.
    fn settle(&mut self, at: u64) {
        let waiting = self.ready || self.state.as_deref() == Some("blocked");
        if !waiting {
            self.waiting_since = None;
        } else if self.waiting_since.is_none() {
            self.waiting_since = Some(at);
        }
    }

    /// Apply the next entry of the host's log, for a terminal whose shell is
    /// `shell`. Returns whether anything the webview shows changed: typing
    /// that answers nothing only moves `active_at`.
    fn apply(&mut self, entry: &Entry, shell: Option<i32>) -> bool {
        if entry.seq <= self.seq {
            return false;
        }
        self.seq = entry.seq;
        let at = entry.at;
        let shell_in_front = |status: &Status| shell.is_some() && status.leader == shell;
        match &entry.note {
            Note::Status(payload) => {
                let Some(report) = parse_status(payload) else { return false };
                let mut changed = self.report(report, at);
                // A program that has already exited printed it: the shell is
                // back in front.
                if self.state.is_some() && shell_in_front(self) {
                    changed |= self.report(Report::default(), at);
                }
                changed
            }
            Note::Leader(leader) => {
                self.leader = *leader;
                // A program killed outright never sends `clear`.
                self.state.is_some() && shell_in_front(self) && self.report(Report::default(), at)
            }
            Note::Typed => {
                // The user typed into it: whatever waited for a reply has
                // had one.
                self.active_at = at;
                let was = std::mem::take(&mut self.ready);
                self.settle(at);
                was
            }
            Note::App(value) => match serde_json::from_value::<AppNote>(value.clone()) {
                Ok(AppNote { mark }) => {
                    self.mark(mark, at);
                    true
                }
                Err(_) => false,
            },
        }
    }

    fn report(&mut self, Report { state, message }: Report, at: u64) -> bool {
        if self.state == state {
            // Saying what it does now is news, but not activity.
            if self.message == message {
                return false;
            }
            self.message = message;
            return true;
        }
        self.message = message;
        // A turn that ends waits for a reply, whether or not the user is
        // looking. Anything else that happens next — a new prompt, quitting
        // — the user did themselves.
        self.ready = state.as_deref() == Some("done");
        self.state = state;
        self.active_at = at;
        self.settle(at);
        true
    }

    /// `Unread` sets the flag and lifts a block, leaving any other status
    /// alone, so a terminal that is working still blinks. `Blocked` is the
    /// status itself, and goes like one: at the program's next report, or,
    /// in a plain shell, when the foreground next changes hands. `Clear`
    /// lifts both.
    fn mark(&mut self, mark: Mark, at: u64) {
        let unblock = |status: &mut Status| {
            if status.state.as_deref() == Some("blocked") {
                status.state = None;
                status.message = None;
            }
        };
        match mark {
            Mark::Unread => {
                unblock(self);
                self.ready = true;
            }
            Mark::Blocked => {
                self.state = Some("blocked".into());
                self.message = None;
                self.ready = false;
            }
            Mark::Clear => {
                unblock(self);
                self.ready = false;
            }
            Mark::Snooze => {
                if self.waiting_since.is_some() {
                    self.waiting_since = Some(at);
                }
            }
        }
        self.settle(at);
    }
}

/// What a terminal keeps with its pty in the host, as its checkpoint, so the
/// next app to attach knows where it belongs and what its status was.
#[derive(Serialize, Deserialize)]
struct Kept {
    home: Home,
    status: Status,
    /// On screen when the app quit and left it running: the next app opens
    /// on it.
    #[serde(default)]
    on_screen: bool,
}

/// Where a terminal the host describes belongs, and its status as of now:
/// its checkpoint with every entry since applied.
fn fold(survivor: &Survivor) -> (Home, Status, bool) {
    let kept: Option<Kept> = survivor
        .checkpoint
        .as_ref()
        .and_then(|c| serde_json::from_value(c.state.clone()).ok());
    let (home, mut status, on_screen) = match kept {
        Some(kept) => (kept.home, kept.status, kept.on_screen),
        None => (
            Home::unknown(),
            Status {
                active_at: now_ms(),
                ..Status::default()
            },
            false,
        ),
    };
    if let Some(checkpoint) = &survivor.checkpoint {
        status.seq = status.seq.max(checkpoint.seq);
    }
    for entry in &survivor.entries {
        status.apply(entry, survivor.shell_pid);
    }
    (home, status, on_screen)
}

impl Terminal {
    fn info(&self) -> TerminalInfo {
        let status = self.status.lock().unwrap();
        TerminalInfo {
            status: status.state.clone(),
            ready: status.ready,
            active_at: status.active_at,
            waiting_since: status.waiting_since,
            message: status.message.clone(),
            ..self.info.clone()
        }
    }

    /// A terminal as the host describes it, with its output and status
    /// caught up.
    fn from_survivor(host: &Arc<dyn Host>, survivor: Survivor) -> (Arc<Terminal>, bool) {
        let (home, status, on_screen) = fold(&survivor);
        let received = survivor.offset + survivor.backlog.len() as u64;
        let term = Arc::new(Terminal {
            id: survivor.id,
            info: TerminalInfo {
                path: key(&survivor.path),
                home,
                status: None,
                ready: false,
                active_at: 0,
                waiting_since: None,
                message: None,
            },
            host: Arc::clone(host),
            output: Mutex::new(Output {
                backlog: survivor.backlog,
                received,
                sink: None,
                exited: false,
            }),
            shell_pid: Mutex::new(survivor.shell_pid),
            status: Mutex::new(status),
            pending: AtomicBool::new(false),
            shared_open: AtomicBool::new(false),
            close_asked: AtomicBool::new(false),
            size: Mutex::new(None),
        });
        (term, on_screen)
    }

    /// Apply an entry of its log. Returns whether anything shown changed.
    fn apply(&self, entry: &Entry) -> bool {
        let shell = *self.shell_pid.lock().unwrap();
        self.status.lock().unwrap().apply(entry, shell)
    }

    /// Catch up with the host's word on it after a reconnect: the output and
    /// log entries it missed. Returns whether anything shown changed.
    fn catch_up(&self, survivor: &Survivor) -> bool {
        self.output.lock().unwrap().receive(survivor.offset, &survivor.backlog);
        let mut changed = false;
        for entry in &survivor.entries {
            changed |= self.apply(entry);
        }
        changed
    }

    /// Its workspace is gone and nothing in it needs the user any more: the
    /// program in it is neither working nor blocked on an answer, no turn
    /// ended there without a reply, and it isn't on screen. `shown` is the terminal
    /// that is. A directory that is still there but no longer holds a
    /// workspace counts as gone: a dev server left running in a removed
    /// worktree recreates its cache folder there.
    fn outlived(&self, shown: Option<&str>) -> bool {
        if shown == Some(self.info.path.as_str()) {
            return false;
        }
        let status = self.status.lock().unwrap();
        if status.ready || matches!(status.state.as_deref(), Some("working" | "blocked")) {
            return false;
        }
        drop(status);
        !crate::git::is_git_repo(Path::new(&self.info.path))
    }

    /// The process leading the foreground, unless that is the shell itself.
    fn foreground(&self) -> Option<i32> {
        let leader = self.status.lock().unwrap().leader?;
        (Some(leader) != *self.shell_pid.lock().unwrap()).then_some(leader)
    }

    /// Hang up on the shell (`pty::Local::hang_up`).
    fn hang_up(&self) {
        self.host.hang_up(&self.id);
    }

    /// Save where it belongs and its status with the pty, as covering every
    /// entry applied so far, saying whether it is the terminal on screen as
    /// the app quits.
    fn save(&self, on_screen: bool) {
        let status = self.status.lock().unwrap().clone();
        let seq = status.seq;
        let kept = Kept {
            home: self.info.home.clone(),
            status,
            on_screen,
        };
        if let Ok(value) = serde_json::to_value(kept) {
            self.host.checkpoint(&self.id, seq, value);
        }
    }

    /// Hand the terminal to `sink`, replaying its backlog first. Whoever
    /// was attached is replaced: a webview since reloaded, or one that's
    /// about to be.
    fn attach(&self, sink: Sink) {
        let mut output = self.output.lock().unwrap();
        let alive = output.backlog.is_empty() || sink(Chunk::Data(output.backlog.clone()));
        output.sink = alive.then_some(sink);
    }
}

pub type Hook = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

struct Inner {
    host: Arc<dyn Host>,
    by_path: Mutex<HashMap<String, Arc<Terminal>>>,
    /// Told about every exit and status change, attached or not. A webview
    /// that reloaded marks running terminals from `list` before attaching to
    /// any of them, and a shell that exits meanwhile has no sink to report
    /// through.
    hook: Option<Hook>,
    /// The terminal on screen, focused window or not. Switching to another
    /// app is not leaving it. Only `reap` and `keep_view` care; nothing about
    /// a terminal's status follows from being looked at.
    shown: Mutex<Option<String>>,
    /// The terminal that was on screen when the last app quit, until the
    /// webview asks for it (`take_restored_view`).
    restored_view: Mutex<Option<String>>,
    /// Another window of this build took the terminals over.
    moved: AtomicBool,
    /// Quitting and keeping the terminals, with this one on screen: every
    /// save from now on says so, or a status arriving while the app quits
    /// would save over it.
    kept_view: Mutex<Option<String>>,
}

impl Inner {
    fn find(&self, id: &str) -> Option<Arc<Terminal>> {
        self.by_path.lock().unwrap().values().find(|t| t.id == id).cloned()
    }

    fn all(&self) -> Vec<Arc<Terminal>> {
        self.by_path.lock().unwrap().values().cloned().collect()
    }

    /// Its status changed: tell the webview, and save it with the pty.
    fn changed(&self, term: &Terminal) {
        if let Some(hook) = &self.hook {
            hook(TerminalEvent::Status(&term.info()));
        }
        let on_screen = self.kept_view.lock().unwrap().as_deref() == Some(term.info.path.as_str());
        term.save(on_screen);
    }

    fn on_event(&self, event: pty::Event) {
        match event {
            pty::Event::Output { id, offset, data } => {
                if let Some(term) = self.find(&id) {
                    term.output.lock().unwrap().receive(offset, &data);
                }
            }
            pty::Event::Entry { id, entry } => {
                let Some(term) = self.find(&id) else { return };
                if term.apply(&entry) {
                    self.changed(&term);
                    // Claude Code finishing the turn in which it removed its
                    // own worktree, typically.
                    if term.outlived(self.shown.lock().unwrap().as_deref()) {
                        term.hang_up();
                    }
                }
            }
            pty::Event::Exit { id, code } => self.exited(&id, Chunk::Exit(code)),
            pty::Event::Adopt(survivor) => {
                self.adopt(survivor);
            }
            pty::Event::Resync(snapshot) => self.resync(snapshot),
            pty::Event::Superseded => {
                self.moved.store(true, Ordering::SeqCst);
                let ids: Vec<String> = self.all().iter().map(|t| t.id.clone()).collect();
                for id in ids {
                    self.exited(&id, Chunk::Moved);
                }
            }
        }
    }

    /// Out of the registry, telling whoever is attached why, and the webview.
    fn exited(&self, id: &str, why: Chunk) {
        // Out of the registry first, so an open racing this exit starts a new
        // shell rather than attaching to a dead one.
        let mut map = self.by_path.lock().unwrap();
        let Some(path) = map.iter().find(|(_, t)| t.id == id).map(|(p, _)| p.clone()) else {
            return;
        };
        let term = map.remove(&path).expect("just found");
        drop(map);
        let mut output = term.output.lock().unwrap();
        output.exited = true;
        output.deliver(why);
        output.sink = None;
        drop(output);
        *term.status.lock().unwrap() = Status::default();
        if let Some(hook) = &self.hook {
            hook(TerminalEvent::Exit(&term.info()));
        }
    }

    /// Take in a terminal the host has and the registry doesn't, in place of
    /// whatever the registry has for its path. Returns whether the terminal
    /// was on screen when the last app quit.
    fn adopt(&self, survivor: Survivor) -> bool {
        if let Some(known) = self.find(&survivor.id) {
            if known.catch_up(&survivor) {
                self.changed(&known);
            }
            return false;
        }
        let (term, on_screen) = Terminal::from_survivor(&self.host, survivor);
        let replaced = self
            .by_path
            .lock()
            .unwrap()
            .insert(term.info.path.clone(), Arc::clone(&term));
        // The host keeps one terminal per path, so one it says is there has
        // taken over from whatever the registry had. One still being opened
        // hands its webview over (`Terminals::open`); any other is gone.
        if let Some(old) = replaced.filter(|old| !old.pending.load(Ordering::SeqCst)) {
            let mut output = old.output.lock().unwrap();
            output.exited = true;
            output.deliver(Chunk::Exit(-1));
            output.sink = None;
        }
        if let Some(hook) = &self.hook {
            hook(TerminalEvent::Status(&term.info()));
        }
        on_screen
    }

    /// After a reconnect: keep the terminals the host still has, catching
    /// them up; take in the ones it has that the registry doesn't; and the
    /// rest have exited, with the code the host remembers, or with the relay
    /// if it remembers none.
    fn resync(&self, snapshot: Snapshot) {
        let present: HashSet<&str> = snapshot.terminals.iter().map(|s| s.id.as_str()).collect();
        let exits: HashMap<&str, &pty::Exited> = snapshot.exited.iter().map(|e| (e.id.as_str(), e)).collect();
        let gone: Vec<String> = self
            .all()
            .iter()
            .filter(|t| !t.pending.load(Ordering::SeqCst) && !present.contains(t.id.as_str()))
            .map(|t| t.id.clone())
            .collect();
        for id in gone {
            let why = match exits.get(id.as_str()) {
                Some(exit) => {
                    // What it printed last, while no one was listening.
                    if let Some(term) = self.find(&id) {
                        term.output.lock().unwrap().receive(exit.offset, &exit.tail);
                    }
                    Chunk::Exit(exit.code)
                }
                None => Chunk::RelayGone,
            };
            self.exited(&id, why);
        }
        for survivor in snapshot.terminals {
            self.adopt(survivor);
        }
    }
}

/// What another build's relay says about one of its terminals, as far as
/// it can be told without taking it over: where it belongs, and its status
/// now, with whatever its program reported since its app last saved.
pub fn describe(survivor: &Survivor) -> TerminalInfo {
    let (home, status, _) = fold(survivor);
    TerminalInfo {
        path: key(&survivor.path),
        home,
        status: status.state,
        ready: status.ready,
        active_at: status.active_at,
        waiting_since: status.waiting_since,
        message: status.message,
    }
}

pub struct Terminals {
    inner: Arc<Inner>,
}

impl Default for Terminals {
    /// Shells in this process, told to no one: for tests.
    fn default() -> Self {
        Self::with_host(None, Arc::new(pty::Local::default()), Snapshot::default())
    }
}

/// Variables the shell must not inherit. A Finder-launched app has almost no
/// environment, but a dev build launched from a terminal carries that
/// terminal's identity (`TERM_PROGRAM=iTerm.app` makes shell integrations
/// emit iTerm-only escapes) and the build tooling's (`npm_*`, `CARGO_*`). The
/// login shell sets up everything the user actually configured.
const STRIPPED_PREFIXES: &[&str] = &[
    "npm_", "PNPM_", "TAURI_", "CARGO_", "TERM_", "ITERM_", "LC_TERMINAL", "VSCODE_", "KITTY_",
    "WEZTERM_", "GHOSTTY_", "ALACRITTY_",
];
const STRIPPED: &[&str] = &["CARGO", "INIT_CWD", "NODE", "COLORTERM", "TERM"];

fn key(path: &str) -> String {
    path.trim_end_matches('/').to_string()
}

const MOVED: &str = "These terminals are open in another parallelist window.";

impl Terminals {
    /// Shells in this process, which end with it.
    pub fn new(hook: Hook) -> Self {
        Self::with_host(Some(hook), Arc::new(pty::Local::default()), Snapshot::default())
    }

    /// Shells in `host`, starting with the ones it already had.
    pub fn with_host(hook: Option<Hook>, host: Arc<dyn Host>, snapshot: Snapshot) -> Self {
        let inner = Arc::new(Inner {
            host: Arc::clone(&host),
            by_path: Mutex::default(),
            hook,
            shown: Mutex::default(),
            restored_view: Mutex::default(),
            moved: AtomicBool::new(false),
            kept_view: Mutex::default(),
        });
        for survivor in snapshot.terminals {
            let path = key(&survivor.path);
            if inner.adopt(survivor) {
                *inner.restored_view.lock().unwrap() = Some(path);
            }
        }
        let weak = Arc::downgrade(&inner);
        host.set_sink(Arc::new(move |event| {
            if let Some(inner) = weak.upgrade() {
                inner.on_event(event);
            }
        }));
        Terminals { inner }
    }

    /// Whether the shells outlive the app: they run in a relay.
    pub fn persistent(&self) -> bool {
        self.inner.host.persistent()
    }

    /// The user's login shell in `path`, with the environment a terminal
    /// emulator gives it.
    pub fn shell_command(path: &str) -> CommandBuilder {
        let mut cmd = CommandBuilder::new_default_prog();
        cmd.cwd(path);
        let names: Vec<String> = cmd
            .iter_full_env_as_str()
            .map(|(name, _)| name.to_string())
            .collect();
        for name in names {
            if STRIPPED.contains(&name.as_str())
                || STRIPPED_PREFIXES.iter().any(|p| name.starts_with(p))
            {
                cmd.env_remove(&name);
            }
        }
        // Launchd's PATH, or a dev build's with node_modules/.bin in front;
        // the login shell builds on whatever it is given.
        if let Some(path) = crate::proc::login_path() {
            cmd.env("PATH", path);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "Parallelist");
        // Programs that link their output decide by `TERM_PROGRAM`, and don't
        // know ours. Claude Code links `[Image #1]` to the image this way.
        cmd.env("FORCE_HYPERLINK", "1");
        // An app started from the Dock has no locale, and zsh then mangles
        // every non-ASCII character. Terminal.app fills the same gap.
        if ["LANG", "LC_ALL", "LC_CTYPE"]
            .iter()
            .all(|name| cmd.get_env(name).is_none())
        {
            cmd.env("LANG", "en_US.UTF-8");
        }
        cmd
    }

    /// Attach `sink` to the workspace's terminal, starting one with `command`
    /// if there is none, at `home`; one already running keeps its own. The
    /// backlog of an existing terminal is replayed into the sink before
    /// anything new.
    pub fn open(
        &self,
        home: Home,
        path: &str,
        size: PtySize,
        command: CommandBuilder,
        sink: Sink,
    ) -> Result<Opened, String> {
        if self.inner.moved.load(Ordering::SeqCst) {
            return Err(MOVED.into());
        }
        let key = key(path);
        let mut map = self.inner.by_path.lock().unwrap();
        if let Some(term) = map.get(&key).cloned() {
            if !term.output.lock().unwrap().exited {
                drop(map);
                if term.pending.load(Ordering::SeqCst) {
                    term.shared_open.store(true, Ordering::SeqCst);
                }
                term.attach(sink);
                *term.size.lock().unwrap() = Some(size);
                let _ = self.inner.host.resize(&term.id, size);
                return Ok(Opened {
                    started: false,
                    terminal: term.info(),
                });
            }
        }

        if !Path::new(path).is_dir() {
            return Err(format!("{path} does not exist"));
        }

        // In the registry before the shell starts, so its first output finds
        // it. A UUID rather than a counter, since a relay outlives the app.
        let status = Status {
            active_at: now_ms(),
            ..Status::default()
        };
        let state = serde_json::to_value(Kept {
            home: home.clone(),
            status: status.clone(),
            on_screen: false,
        })
        .map_err(|e| e.to_string())?;
        let term = Arc::new(Terminal {
            id: uuid::Uuid::new_v4().to_string(),
            info: TerminalInfo {
                path: key.clone(),
                home,
                status: None,
                ready: false,
                active_at: 0,
                waiting_since: None,
                message: None,
            },
            host: Arc::clone(&self.inner.host),
            output: Mutex::new(Output {
                backlog: Vec::new(),
                received: 0,
                sink: Some(sink),
                exited: false,
            }),
            shell_pid: Mutex::default(),
            status: Mutex::new(status),
            pending: AtomicBool::new(true),
            shared_open: AtomicBool::new(false),
            close_asked: AtomicBool::new(false),
            size: Mutex::new(Some(size)),
        });
        map.insert(key.clone(), Arc::clone(&term));
        drop(map);

        let opened = self.inner.host.open(&term.id, &key, &Spawn::from_command(&command), size, state);
        term.pending.store(false, Ordering::SeqCst);
        let forget = || {
            let mut map = self.inner.by_path.lock().unwrap();
            if map.get(&key).is_some_and(|t| Arc::ptr_eq(t, &term)) {
                map.remove(&key);
            }
        };
        match opened {
            Ok(HostOpened::Started { pid }) => {
                *term.shell_pid.lock().unwrap() = pid;
                if term.close_asked.load(Ordering::SeqCst) {
                    term.hang_up();
                }
                Ok(Opened {
                    started: true,
                    terminal: term.info(),
                })
            }
            Ok(HostOpened::Existing { id }) => {
                // Taken in before the answer came (`Event::Adopt`); the
                // webview goes to it.
                forget();
                let existing = self.inner.find(&id).ok_or("the terminal relay lost the terminal")?;
                let sink = term.output.lock().unwrap().sink.take();
                if let Some(sink) = sink {
                    existing.attach(sink);
                }
                *existing.size.lock().unwrap() = Some(size);
                let _ = self.inner.host.resize(&existing.id, size);
                Ok(Opened {
                    started: false,
                    terminal: existing.info(),
                })
            }
            Err(e) => {
                forget();
                // Another open waits on it too: it hears the end here, as
                // this one hears it in the error.
                if term.shared_open.load(Ordering::SeqCst) {
                    let mut output = term.output.lock().unwrap();
                    output.exited = true;
                    output.deliver(Chunk::Exit(-1));
                    output.sink = None;
                }
                // It may have started after all, if only the answer was
                // lost: hang up on it, which a host that never started it
                // ignores.
                self.inner.host.hang_up(&term.id);
                Err(e)
            }
        }
    }

    fn get(&self, path: &str) -> Result<Arc<Terminal>, String> {
        self.inner
            .by_path
            .lock()
            .unwrap()
            .get(&key(path))
            .cloned()
            .ok_or_else(|| format!("no terminal in {path}"))
    }

    /// Type `data` into the terminal. Returns when it was last active,
    /// which this makes now. The host logs the typing, and a turn that
    /// ended there has had its reply once it comes back.
    pub fn write(&self, path: &str, data: &[u8]) -> Result<u64, String> {
        let term = self.get(path)?;
        let at = now_ms();
        self.inner.host.write(&term.id, data, Some(at))?;
        Ok(at)
    }

    /// Pass on what xterm sends while the user only looks and scrolls (focus
    /// and mouse reports, arrow keys for a wheel): the program gets it, but
    /// looking isn't activity, and isn't a reply. Returns when it was last
    /// active. The webview tells these apart (`NOT_TYPED` in `terminal.ts`),
    /// since an arrow key from a wheel looks like any other here.
    pub fn write_untyped(&self, path: &str, data: &[u8]) -> Result<u64, String> {
        let term = self.get(path)?;
        self.inner.host.write(&term.id, data, None)?;
        let active_at = term.status.lock().unwrap().active_at;
        Ok(active_at)
    }

    pub fn resize(&self, path: &str, size: PtySize) -> Result<(), String> {
        let term = self.get(path)?;
        *term.size.lock().unwrap() = Some(size);
        self.inner.host.resize(&term.id, size)
    }

    /// Hang up on the shell and whatever it is running in the foreground.
    /// The exit is reported through the sink like any other. Returns whether
    /// there was a terminal to close.
    pub fn close(&self, path: &str) -> bool {
        let Ok(term) = self.get(path) else {
            return false;
        };
        // A host may not know the id until its open returns.
        term.close_asked.store(true, Ordering::SeqCst);
        term.hang_up();
        true
    }

    /// Hang up on every terminal: quitting with "End all".
    pub fn close_all(&self) {
        for term in self.inner.all() {
            term.hang_up();
        }
    }

    /// Quitting and leaving the terminals running: save each, saying which
    /// one is on screen, for the next app to open on.
    pub fn keep_view(&self) {
        let shown = self.inner.shown.lock().unwrap().clone();
        self.inner.kept_view.lock().unwrap().clone_from(&shown);
        for term in self.inner.all() {
            term.save(shown.as_deref() == Some(term.info.path.as_str()));
        }
    }

    /// Wait until everything sent to the host has reached it: before the app
    /// quits, so its hang-ups and checkpoints aren't lost with it.
    pub fn flush(&self) {
        self.inner.host.flush();
    }

    /// The terminal that was on screen when the last app quit, if it is
    /// still running here. Asked once, at startup: a reload of the webview
    /// keeps its own place.
    pub fn take_restored_view(&self) -> Option<TerminalInfo> {
        let path = self.inner.restored_view.lock().unwrap().take()?;
        let term = self.inner.by_path.lock().unwrap().get(&path).cloned()?;
        Some(term.info())
    }

    /// Close every terminal whose directory has gone and that nothing in
    /// needs the user any more (`Terminal::outlived`), and return their
    /// paths. Asked whenever that may have just become true: a refresh that
    /// may be the first to notice the directory is gone, the user leaving a
    /// terminal, and its program going quiet (in the reader).
    pub fn reap(&self) -> Vec<String> {
        let shown = self.inner.shown.lock().unwrap().clone();
        self.inner
            .all()
            .into_iter()
            .filter(|term| term.outlived(shown.as_deref()))
            .map(|term| {
                term.hang_up();
                term.info.path.clone()
            })
            .collect()
    }

    /// Which terminal is on screen, if any. It changes nothing about that
    /// terminal — looking at a finished turn isn't replying to it — but
    /// leaving one may be the last thing a removed workspace's terminal was
    /// waiting for.
    pub fn view(&self, path: Option<&str>) {
        *self.inner.shown.lock().unwrap() = path.map(key);
        self.reap();
    }

    /// The process leading the terminal's foreground, other than its shell:
    /// whatever program is running in it, such as Claude Code.
    pub fn foreground(&self, path: &str) -> Option<i32> {
        self.get(path).ok()?.foreground()
    }

    /// What runs in the foreground of each terminal that has something
    /// other than its shell there, by path: the command line `ps` shows for
    /// it, which is the program's own title where it sets one (`npm run dev`,
    /// `claude`). What the Terminals dialog lists, so the user can tell a dev
    /// server from an idle shell before ending either. The foregrounds are
    /// the ones logged, so nothing here asks the host.
    pub fn programs(&self) -> HashMap<String, String> {
        let pids: Vec<(String, i32)> = self
            .inner
            .all()
            .iter()
            .filter_map(|term| Some((term.info.path.clone(), term.foreground()?)))
            .collect();
        if pids.is_empty() {
            return HashMap::new();
        }
        let list = pids.iter().map(|(_, pid)| pid.to_string()).collect::<Vec<_>>().join(",");
        let Ok(out) = crate::proc::run_timeout(
            "ps",
            &["-ww", "-o", "pid=,args=", "-p", &list],
            None,
            Duration::from_secs(5),
        ) else {
            return HashMap::new();
        };
        // `ps` exits 1 when one of the pids has gone meanwhile, and still
        // lists the others.
        let commands = parse_ps(&out.stdout, executable);
        pids.into_iter()
            .filter_map(|(path, pid)| Some((path, commands.get(&pid)?.clone())))
            .collect()
    }

    /// Mark a terminal by hand, logged with the host as everything else that
    /// changes its status is, and reported like any status change.
    pub fn mark(&self, path: &str, mark: Mark) -> Result<(), String> {
        let term = self.get(path)?;
        let note = serde_json::to_value(AppNote { mark }).map_err(|e| e.to_string())?;
        self.inner.host.note(&term.id, now_ms(), note)
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        self.inner.all().iter().map(|t| t.info()).collect()
    }

    /// What each terminal shows right now, as text: its backlog replayed
    /// into a screen of its current size. Only asked for by the palette's
    /// screen search, so nothing is parsed while no one searches. The
    /// backlog starts at a line break somewhere in the past rather than at
    /// the shell's start, which can leave a full-screen program's first
    /// frame off; the newest output, which is what a screen shows, is whole.
    pub fn screens(&self) -> Vec<crate::screen_search::Screen> {
        let snapshots: Vec<(String, Vec<u8>, Option<PtySize>)> = self
            .inner
            .all()
            .iter()
            .map(|term| {
                let backlog = term.output.lock().unwrap().backlog.clone();
                let size = *term.size.lock().unwrap();
                (term.info.path.clone(), backlog, size)
            })
            .collect();
        crate::proc::parallel_map(snapshots, |(path, backlog, size)| crate::screen_search::Screen {
            path: path.clone(),
            text: screen_text(backlog, size.map_or(UNSIZED, |s| (s.rows, s.cols))),
        })
    }

    /// How many terminals wait for the user: blocked on an answer, or with a
    /// turn that ended and got no reply. The ones `terminalMark` draws a box
    /// around.
    pub fn waiting(&self) -> usize {
        self.list()
            .iter()
            .filter(|info| info.waiting_since.is_some())
            .count()
    }

    /// Whether a program in any terminal reports `working`: what blinks the
    /// terminal glyph in the app, and the Dock icon's cursor (`dock.rs`).
    pub fn any_working(&self) -> bool {
        self.list()
            .iter()
            .any(|info| info.status.as_deref() == Some("working"))
    }
}

/// What a status payload says: the state it sets, `None` for `clear`, and
/// its message.
#[derive(Debug, Default, PartialEq)]
struct Report {
    state: Option<String>,
    message: Option<String>,
}

/// Parse a status payload. Statuses with an `id` are a program's secondary
/// ones (its background tasks); only its own, without one, describes the
/// terminal. `msg` is base64, as Claude Code sends it (it builds the
/// payload as `state=…:app=…[:kind=…][:progress=…][:title=…][:msg=…]`).
fn parse_status(payload: &str) -> Option<Report> {
    use base64::Engine;
    let mut state = None;
    let mut message = None;
    for field in payload.split(':') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        match name {
            "state" => state = Some(value),
            "id" => return None,
            "msg" => {
                message = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .filter(|m| !m.trim().is_empty())
            }
            _ => {}
        }
    }
    let state = state?;
    Some(Report {
        state: (state != "clear").then(|| state.to_string()),
        message,
    })
}

/// What a terminal no window has given a size since this app started is
/// replayed at: wide, so long lines wrap no more than they did.
const UNSIZED: (u16, u16) = (40, 120);

/// The text a terminal of `rows` × `cols` shows after `output`, its
/// wrapped lines joined.
fn screen_text(output: &[u8], (rows, cols): (u16, u16)) -> String {
    let size = |n: u16| std::num::NonZeroU16::new(n).unwrap_or(std::num::NonZeroU16::MIN);
    let mut parser = vt100::Parser::new(size(rows), size(cols), 0);
    parser.process(output);
    parser.screen().contents()
}

/// The file a process was started from, as the kernel has it.
fn executable(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    String::from_utf8(buf).ok()
}

/// `ps -o pid=,args=` as pid → command line, with the program named by its
/// file name rather than the path it was started by. `executable` says where
/// that path ends, since it may hold spaces; a program that set its own
/// title (`claude`) is taken as it reads.
fn parse_ps(stdout: &str, executable: impl Fn(i32) -> Option<String>) -> HashMap<i32, String> {
    stdout
        .lines()
        .filter_map(|line| {
            let (pid, args) = line.trim_start().split_once(char::is_whitespace)?;
            let pid: i32 = pid.parse().ok()?;
            let args = args.trim();
            let (program, rest) = match executable(pid).filter(|exe| args.starts_with(exe.as_str())) {
                Some(exe) => (exe.clone(), args[exe.len()..].trim_start().to_string()),
                None => {
                    let (program, rest) = args.split_once(' ').unwrap_or((args, ""));
                    (program.to_string(), rest.to_string())
                }
            };
            let program = program.rsplit('/').next().unwrap_or(&program).to_string();
            let command = if rest.is_empty() { program } else { format!("{program} {rest}") };
            Some((pid, command))
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Instant;

    pub(crate) const SIZE: PtySize = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    pub(crate) fn home() -> Home {
        Home {
            project_id: "p".into(),
            branch: "feature".into(),
            kind: WorkspaceKind::Worktree,
        }
    }

    pub(crate) fn sink() -> (Sink, Receiver<Chunk>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(move |chunk| tx.send(chunk).is_ok()), rx)
    }

    /// A plain `sh`, so the tests don't run the user's shell profile.
    pub(crate) fn sh(dir: &Path) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.cwd(dir);
        cmd.env("PS1", "$ ");
        cmd
    }

    /// Collect output until `want` shows up, or fail after a few seconds.
    pub(crate) fn read_until(rx: &Receiver<Chunk>, want: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Chunk::Data(bytes)) => seen.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Chunk::Exit(code)) => seen.push_str(&format!("<exit {code}>")),
                Ok(Chunk::RelayGone) => seen.push_str("<exit relay>"),
                Ok(Chunk::Moved) => seen.push_str("<moved>"),
                Err(_) => {}
            }
            if seen.contains(want) {
                return seen;
            }
        }
        panic!("never saw {want:?} in {seen:?}");
    }

    pub(crate) fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("parallelist-term-{name}-{}", std::process::id()));
        // A workspace, as far as `outlived` can tell.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn runs_in_the_workspace_and_reports_its_exit() {
        let dir = temp_dir("cwd");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        assert!(terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);
        terminals.write(&path, b"pwd; exit 3\n").unwrap();
        let seen = read_until(&rx, "<exit 3>");
        assert!(seen.contains(&path), "{seen:?}");
        assert!(terminals.list().is_empty(), "an exited terminal leaves the registry");
    }

    /// Typing is activity; the focus report xterm sends on clicking in is
    /// not, and neither is output.
    #[test]
    fn typing_makes_a_terminal_active_and_focusing_does_not() {
        let dir = temp_dir("active");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        let started = terminals.list()[0].active_at;
        assert!(started > 0, "a new terminal is active from the start");

        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(terminals.write_untyped(&path, b"\x1b[I").unwrap(), started);
        let typed = terminals.write(&path, b"echo out-$((40+2))\n").unwrap();
        assert!(typed > started);
        read_until(&rx, "out-42");
        assert_eq!(terminals.list()[0].active_at, typed);
        terminals.close(&path);
    }

    /// A webview reload attaches to the running shell and gets its recent
    /// output back, rather than starting a second one.
    #[test]
    fn reopening_attaches_and_replays() {
        let dir = temp_dir("replay");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo first-$((40+2))\n").unwrap();
        read_until(&rx, "first-42");
        drop(rx);

        let (tx, rx) = sink();
        assert!(!terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started, "no second shell");
        read_until(&rx, "first-42");
        terminals.write(&path, b"echo second-$((40+3))\n").unwrap();
        read_until(&rx, "second-43");
        assert_eq!(terminals.list().len(), 1);
        assert!(terminals.close(&path));
        read_until(&rx, "<exit");
    }

    /// Closing takes the foreground job down with the shell — a dev server
    /// left running after its terminal is gone has no way to be stopped.
    #[test]
    fn closing_hangs_up_on_the_foreground_job() {
        let dir = temp_dir("close");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        // `exec` keeps the pid it printed, so this is the sleep's own.
        terminals
            .write(&path, b"sh -c 'echo pid=$$; exec sleep 300'\n")
            .unwrap();
        let seen = read_until(&rx, "\npid=");
        let sleeper: i32 = seen
            .rsplit("pid=")
            .next()
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|n| n.parse().ok())
            .expect("a pid");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(unsafe { libc::kill(sleeper, 0) }, 0, "sleep is running");

        assert!(terminals.close(&path));
        read_until(&rx, "<exit");
        let deadline = Instant::now() + Duration::from_secs(3);
        while unsafe { libc::kill(sleeper, 0) } == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_ne!(unsafe { libc::kill(sleeper, 0) }, 0, "sleep {sleeper} survived");
    }

    #[test]
    fn names_the_program_in_the_foreground_and_not_the_shell() {
        let dir = temp_dir("programs");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo ready\n").unwrap();
        read_until(&rx, "\nready");
        assert!(terminals.programs().is_empty(), "only the shell is running");

        terminals.write(&path, b"sleep 300\n").unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut programs = terminals.programs();
        while programs.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            programs = terminals.programs();
        }
        assert_eq!(programs.get(&path).map(String::as_str), Some("sleep 300"));
        terminals.close(&path);
    }

    #[test]
    fn ps_output_names_programs_by_file_name() {
        let exe = |pid| match pid {
            412 => Some("/opt/homebrew/bin/node".to_string()),
            500 => Some("/Applications/Visual Studio Code.app/Contents/MacOS/Electron".to_string()),
            9001 => Some("/Users/me/.local/share/claude/versions/2.1.309".to_string()),
            _ => None,
        };
        let parsed = parse_ps(
            "  412 /opt/homebrew/bin/node /tmp/vite.js --port 5173\n9001 claude\n  77 -zsh\n\
             500 /Applications/Visual Studio Code.app/Contents/MacOS/Electron --inspect\n",
            exe,
        );
        assert_eq!(parsed[&412], "node /tmp/vite.js --port 5173");
        assert_eq!(parsed[&9001], "claude", "a program's own title is kept");
        assert_eq!(parsed[&77], "-zsh");
        assert_eq!(parsed[&500], "Electron --inspect", "a path with spaces in it");
    }

    #[test]
    fn a_missing_directory_is_an_error_not_home() {
        let terminals = Terminals::default();
        let (tx, _rx) = sink();
        let missing = "/nonexistent/parallelist-workspace";
        let err = terminals
            .open(home(), missing, SIZE, sh(Path::new(missing)), tx)
            .unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }

    /// What is on screen, not everything that went by: a line redrawn in
    /// place shows once, and a full-screen program shows its own screen.
    #[test]
    fn a_screen_is_what_the_output_leaves_on_it() {
        let output = b"$ build\r\nstep 1/3\r\x1b[2Kstep 2/3\r\x1b[2Kstep 3/3\r\nok\r\n$ ";
        let text = screen_text(output, (24, 80));
        assert_eq!(text.lines().collect::<Vec<_>>(), ["$ build", "step 3/3", "ok", "$ "]);
        let full = b"before\r\n\x1b[?1049h\x1b[H\x1b[2Jin the editor";
        assert_eq!(screen_text(full, (24, 80)).trim(), "in the editor");
    }

    #[test]
    fn screens_are_read_from_running_terminals() {
        let dir = temp_dir("screens");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::default();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo on-screen-$((40+2))\n").unwrap();
        read_until(&rx, "on-screen-42");
        let screens = terminals.screens();
        assert_eq!(screens.len(), 1);
        assert_eq!(screens[0].path, path);
        assert!(screens[0].text.contains("on-screen-42"), "{:?}", screens[0].text);
        terminals.close(&path);
    }

    #[test]
    fn statuses_parse_as_claude_code_sends_them() {
        let state = |s: &str| Report { state: Some(s.into()), message: None };
        assert_eq!(parse_status("state=working:app=claude-code"), Some(state("working")));
        assert_eq!(
            parse_status("state=blocked:app=claude-code:kind=permission:msg=a b"),
            Some(state("blocked")),
            "a message that isn't base64 is dropped"
        );
        // As observed from Claude Code 2.1.309 running a Bash command.
        assert_eq!(
            parse_status("state=working:app=claude-code:msg=Q3JlYXRpbmcgdGhlIHByb2JlIGZpbGU="),
            Some(Report {
                state: Some("working".into()),
                message: Some("Creating the probe file".into())
            })
        );
        assert_eq!(parse_status("state=clear"), Some(Report::default()));
        // A background task's own status says nothing about the terminal.
        assert_eq!(parse_status("state=working:app=claude-code:id=7"), None);
        assert_eq!(parse_status("nonsense"), None);
    }

    fn status_hook() -> (Hook, Receiver<(Option<String>, bool)>) {
        let (tx, rx) = mpsc::channel();
        let hook: Hook = Arc::new(move |event| {
            if let TerminalEvent::Status(info) = event {
                let _ = tx.send((info.status.clone(), info.ready));
            }
        });
        (hook, rx)
    }

    /// A program in the terminal's foreground that reports each state it
    /// is told, as Claude Code stays in the foreground between turns. It is
    /// told through a pipe rather than typed at, because typing is the
    /// reply a finished turn waits for, and these tests tell the two apart.
    struct Reporter(std::fs::File);

    impl Reporter {
        fn start(terminals: &Terminals, path: &str, name: &str) -> Self {
            let fifo = std::env::temp_dir()
                .join(format!("parallelist-term-{name}-{}.fifo", std::process::id()));
            let _ = std::fs::remove_file(&fifo);
            let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
            assert!(made.success());
            let script = "while read s; do printf \"\\033]7501;state=%s\\007\" \"$s\"; done";
            terminals
                .write(path, format!("sh -c '{script}' < {}\n", fifo.display()).as_bytes())
                .unwrap();
            // Opening to write waits until the loop has it open to read.
            Reporter(std::fs::OpenOptions::new().write(true).open(&fifo).unwrap())
        }

        fn report(&mut self, state: &str) {
            writeln!(self.0, "{state}").unwrap();
        }
    }

    #[test]
    fn a_finished_turn_is_ready_until_typed_into() {
        let dir = temp_dir("ready");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, _rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        let mut program = Reporter::start(&terminals, &path, "ready");
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        program.report("working");
        assert_eq!(next(), (Some("working".into()), false));
        assert_eq!(terminals.list()[0].waiting_since, None);
        program.report("done");
        assert_eq!(next(), (Some("done".into()), true), "ended, no reply yet");
        assert!(terminals.list()[0].ready);
        assert_eq!(terminals.waiting(), 1);
        let since = terminals.list()[0].waiting_since.expect("waiting");

        // Looking isn't replying, and neither is the focus report xterm
        // sends on clicking in, or what it sends for a wheel.
        terminals.view(Some(&path));
        terminals.write_untyped(&path, b"\x1b[I").unwrap();
        terminals.write_untyped(&path, b"\x1b[<64;10;5M").unwrap();
        assert!(statuses.try_recv().is_err(), "nothing changed");
        assert!(terminals.list()[0].ready, "still waiting for a reply");
        assert_eq!(terminals.waiting(), 1);

        // A keystroke is.
        terminals.write(&path, b"y").unwrap();
        assert_eq!(next(), (Some("done".into()), false), "replied to");
        assert_eq!(terminals.waiting(), 0);
        assert_eq!(terminals.list()[0].waiting_since, None);

        program.report("working");
        assert_eq!(next(), (Some("working".into()), false));
        program.report("done");
        assert_eq!(next(), (Some("done".into()), true), "ended on screen: still a reply owed");
        program.report("blocked");
        assert_eq!(next(), (Some("blocked".into()), false));
        assert_eq!(terminals.waiting(), 1, "blocked waits, seen or not");
        let again = terminals.list()[0].waiting_since.expect("waiting");
        assert!(again >= since, "a new wait starts at the back");
        assert!(terminals.close(&path));
    }

    #[test]
    fn a_terminal_marked_by_hand_reads_as_if_its_program_had_said_so() {
        let dir = temp_dir("marked");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, _rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        let mut program = Reporter::start(&terminals, &path, "marked");
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        program.report("working");
        assert_eq!(next(), (Some("working".into()), false));
        terminals.mark(&path, Mark::Unread).unwrap();
        assert_eq!(next(), (Some("working".into()), true), "still working");
        terminals.write(&path, b"y").unwrap();
        assert_eq!(next(), (Some("working".into()), false), "replied to");

        terminals.mark(&path, Mark::Blocked).unwrap();
        assert_eq!(next(), (Some("blocked".into()), false));
        assert_eq!(terminals.waiting(), 1);
        let since = terminals.list()[0].waiting_since.expect("waiting");
        terminals.mark(&path, Mark::Unread).unwrap();
        assert_eq!(next(), (None, true), "unread instead of blocked");
        assert_eq!(terminals.list()[0].waiting_since, Some(since), "still the same wait");
        program.report("working");
        assert_eq!(next(), (Some("working".into()), false), "the program's word wins");
        assert_eq!(terminals.list()[0].waiting_since, None);

        terminals.mark(&path, Mark::Blocked).unwrap();
        assert_eq!(next(), (Some("blocked".into()), false));
        terminals.mark(&path, Mark::Clear).unwrap();
        assert_eq!(next(), (None, false), "cleared");
        assert_eq!(terminals.list()[0].waiting_since, None);
        terminals.mark(&path, Mark::Unread).unwrap();
        assert_eq!(next(), (None, true));
        terminals.mark(&path, Mark::Clear).unwrap();
        assert_eq!(next(), (None, false), "cleared");
        assert_eq!(terminals.list()[0].waiting_since, None);
        program.report("working");
        assert_eq!(next(), (Some("working".into()), false));
        terminals.mark(&path, Mark::Clear).unwrap();
        assert_eq!(next(), (Some("working".into()), false), "still working");

        terminals.mark(&path, Mark::Snooze).unwrap();
        assert_eq!(next(), (Some("working".into()), false));
        assert_eq!(terminals.list()[0].waiting_since, None, "snoozing starts no wait");
        terminals.mark(&path, Mark::Blocked).unwrap();
        assert_eq!(next(), (Some("blocked".into()), false));
        let since = terminals.list()[0].waiting_since.expect("waiting");
        std::thread::sleep(Duration::from_millis(5));
        terminals.mark(&path, Mark::Snooze).unwrap();
        assert_eq!(next(), (Some("blocked".into()), false), "still blocked");
        let snoozed = terminals.list()[0].waiting_since.expect("still waiting");
        assert!(snoozed > since, "waits as if it had only just started");

        assert!(terminals.mark("/nowhere", Mark::Blocked).is_err());
        assert!(terminals.close(&path));
    }

    /// Claude Code removing its own worktree: the terminal stays while the
    /// turn runs, while its end awaits a reply and while it is on screen,
    /// and closes once the user has replied and left.
    #[test]
    fn a_removed_workspace_keeps_its_terminal_until_nothing_needs_the_user() {
        let dir = temp_dir("removed");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        let mut program = Reporter::start(&terminals, &path, "removed");
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        program.report("working");
        assert_eq!(next(), (Some("working".into()), false));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(terminals.reap().is_empty(), "still working");

        program.report("done");
        assert_eq!(next(), (Some("done".into()), true));
        assert!(terminals.reap().is_empty(), "its last turn awaits a reply");

        terminals.view(Some(&path));
        assert!(terminals.reap().is_empty(), "looking isn't replying");
        terminals.write(&path, b"y").unwrap();
        assert_eq!(next(), (Some("done".into()), false), "replied to");
        assert_eq!(terminals.list().len(), 1, "still on screen");

        terminals.view(None);
        read_until(&rx, "<exit");
        assert!(terminals.list().is_empty());
    }

    /// A dev server running on in a removed worktree recreates the directory
    /// with a cache folder in it. That isn't the workspace being there.
    #[test]
    fn a_directory_recreated_without_its_workspace_is_gone_too() {
        let dir = temp_dir("removed-recreated");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::new(Arc::new(|_| {}));
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join(".vite")).unwrap();
        assert_eq!(terminals.reap(), [path]);
        read_until(&rx, "<exit");
    }

    /// Nothing needs the user in a plain shell, or once the program in it
    /// goes quiet: the directory going is enough, as removing it through
    /// the app would have closed it too.
    #[test]
    fn a_terminal_with_nothing_to_show_goes_with_its_directory() {
        let dir = temp_dir("removed-idle");
        let path = dir.to_string_lossy().to_string();
        let terminals = Terminals::new(Arc::new(|_| {}));
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo up\n").unwrap();
        read_until(&rx, "up");
        assert!(terminals.reap().is_empty(), "the directory is still there");

        // Quiet the moment it reports so, without waiting for a refresh.
        let mut program = Reporter::start(&terminals, &path, "removed-idle");
        program.report("working");
        std::thread::sleep(Duration::from_millis(200));
        std::fs::remove_dir_all(&dir).unwrap();
        program.report("idle");
        read_until(&rx, "<exit");

        let dir = temp_dir("removed-plain");
        let path = dir.to_string_lossy().to_string();
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(terminals.reap(), [path]);
        read_until(&rx, "<exit");
    }

    /// The probe is answered, so a program that waits for the answer starts
    /// reporting; and a status outlives nothing: once the program is gone
    /// and the shell prompts again, it is cleared even though no `clear` came.
    #[test]
    fn answers_the_probe_and_follows_the_status() {
        let dir = temp_dir("status");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();

        terminals
            .write(
                &path,
                b"sh -c \"stty -icanon -echo; printf '\\033]7501;?\\007'; head -c 9 | tr '\\033\\007' EB\"\n",
            )
            .unwrap();
        read_until(&rx, "E]7501;?B");

        terminals
            .write(&path, b"sh -c \"printf '\\033]7501;state=working:app=t\\007'; sleep 0.3\"\n")
            .unwrap();
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status").0;
        assert_eq!(next(), Some("working".to_string()));
        assert_eq!(terminals.list()[0].status.as_deref(), Some("working"));
        assert_eq!(next(), None, "cleared once the shell is back in front");
        assert!(terminals.close(&path));
        read_until(&rx, "<exit");
    }

    #[test]
    fn the_shell_gets_a_terminal_environment_and_not_the_launchers() {
        std::env::set_var("npm_config_prefix", "/tmp/leak");
        std::env::set_var("TERM_PROGRAM", "iTerm.app");
        let cmd = Terminals::shell_command("/tmp");
        assert!(cmd.get_env("npm_config_prefix").is_none());
        assert_eq!(cmd.get_env("TERM_PROGRAM").unwrap(), "Parallelist");
        assert_eq!(cmd.get_env("TERM").unwrap(), "xterm-256color");
        assert_eq!(cmd.get_env("FORCE_HYPERLINK").unwrap(), "1");
        assert!(cmd.is_default_prog(), "the user's login shell");
    }
}
