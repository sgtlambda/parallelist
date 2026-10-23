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
//! startup, so the reader answers it, then follows the status sequences.
//! Observed from Claude Code: `idle` once it is ready, `working` the moment
//! a prompt is sent, `done` when the turn ends, `clear` when it exits.
//!
//! A turn that ends out of the user's sight leaves the terminal **ready**: a
//! flag, kept here so a webview reload doesn't forget it, set when `done`
//! arrives while the terminal isn't the one being viewed and cleared once it
//! is. The frontend says which one that is (`Terminals::view`).
//!
//! Each terminal also keeps when it was last **active**, which is what
//! orders the palette and the branch list among running terminals: when it
//! started, its program's status changed, or the user typed into it. Not its
//! output, or a dev server's logging would keep it on top for good.
//!
//! A terminal that **waits** for the user — blocked on an answer, or ready —
//! keeps when it started to, for as long as it goes on waiting. That orders
//! the attention queue, oldest first, and lives here so a webview reload
//! doesn't put everything back at the same moment. Going from blocked to
//! ready is still the same wait.
//!
//! A terminal can outlive its directory: Claude Code often removes its own
//! worktree once it has merged it, from inside the very shell it runs in.
//! Removing a workspace through the app closes its terminal first; one
//! removed from inside is closed **once nothing in it needs the user** —
//! not while its program works or is blocked on an answer, not while a turn
//! that ended there is unseen, and not while it is on screen (`reap`). Until
//! then it stays reachable from the overview's `removed` list.

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::tasks::now_ms;

/// How much recent output a terminal keeps for replay on reattach. A few
/// screens of a noisy build; xterm keeps the real scrollback on its side.
const BACKLOG: usize = 512 * 1024;

/// How long to wait for the last output after the shell exits. A background
/// job the shell left behind can hold the pty open indefinitely, and the exit
/// still has to be reported.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// What a terminal sends to whoever is attached.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk {
    Data(Vec<u8>),
    /// The shell is gone; the terminal has been dropped from the registry.
    Exit(i32),
}

pub type Sink = Box<dyn Fn(Chunk) -> bool + Send>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub path: String,
    pub project_id: String,
    /// What the program in it last reported over OSC 7501 (`working`,
    /// `idle`, `done`, `blocked`…), or `None` when nothing has, or it cleared
    /// its status, or it is gone.
    pub status: Option<String>,
    /// A turn ended while the terminal wasn't being viewed, and it hasn't
    /// been since.
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

/// What the user can mark a terminal as from its branch's menu, as if its
/// program had said so.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    /// A turn ended out of sight: `ready`, until the terminal is viewed.
    Unseen,
    /// Waiting for an answer, until the program reports something else.
    Blocked,
}

/// What the registry reports about terminals, attached or not.
pub enum TerminalEvent<'a> {
    Exit(&'a TerminalInfo),
    /// The status changed; `info.status` is the new one.
    Status(&'a TerminalInfo),
}

struct Output {
    backlog: Vec<u8>,
    /// Where output goes while someone is attached. Returns false once the
    /// receiving end is gone, and is then dropped.
    sink: Option<Sink>,
    exited: bool,
}

struct Terminal {
    info: TerminalInfo,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<Output>,
    /// The shell's pid. While it leads the foreground process group, nothing
    /// else is running, so no program status can be current.
    shell_pid: Option<i32>,
    status: Mutex<Status>,
}

#[derive(Default)]
struct Status {
    state: Option<String>,
    message: Option<String>,
    ready: bool,
    active_at: u64,
    waiting_since: Option<u64>,
}

impl Status {
    /// Start the wait when it begins and end it when it does, after
    /// anything that changes `state` or `ready`.
    fn settle(&mut self) {
        let waiting = self.ready || self.state.as_deref() == Some("blocked");
        if !waiting {
            self.waiting_since = None;
        } else if self.waiting_since.is_none() {
            self.waiting_since = Some(now_ms());
        }
    }
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

    /// Act on the OSC 7501 sequences in a chunk of output. `viewed` says
    /// whether the user is looking at this terminal. Returns whether the
    /// status changed.
    fn follow_status(&self, payloads: Vec<String>, viewed: bool) -> bool {
        let mut changed = false;
        for payload in payloads {
            if payload.starts_with('?') {
                // The probe. Anything starting with `?` is taken as support.
                let mut writer = self.writer.lock().unwrap();
                let _ = writer.write_all(b"\x1b]7501;?\x07");
                let _ = writer.flush();
            } else if let Some(report) = parse_status(&payload) {
                changed |= self.set_status(report, viewed);
            }
        }
        // A program killed outright never sends `clear`. Its shell is back in
        // the foreground by the time anything prints a prompt.
        if self.status.lock().unwrap().state.is_some() && self.shell_pid.is_some() {
            let leader = self.master.lock().unwrap().process_group_leader();
            if leader == self.shell_pid {
                changed |= self.set_status(Report::default(), viewed);
            }
        }
        changed
    }

    fn set_status(&self, Report { state, message }: Report, viewed: bool) -> bool {
        let mut status = self.status.lock().unwrap();
        if status.state == state {
            // Saying what it does now is news, but not activity.
            if status.message == message {
                return false;
            }
            status.message = message;
            return true;
        }
        status.message = message;
        // A turn that ends out of sight waits to be seen. Anything else that
        // happens next — a new prompt, quitting — the user did themselves.
        status.ready = state.as_deref() == Some("done") && !viewed;
        status.state = state;
        status.active_at = now_ms();
        status.settle();
        true
    }

    /// `Unseen` sets only the flag, leaving what the program reported alone,
    /// so a terminal that is working still blinks. `Blocked` is the status
    /// itself, and goes like one: at the program's next report, or, in a
    /// plain shell, at its next output.
    fn mark(&self, mark: Mark) {
        let mut status = self.status.lock().unwrap();
        match mark {
            Mark::Unseen => status.ready = true,
            Mark::Blocked => {
                status.state = Some("blocked".into());
                status.message = None;
                status.ready = false;
            }
        }
        status.settle();
    }

    /// The user is looking at it: whatever was waiting to be seen has been.
    fn seen(&self) -> bool {
        let mut status = self.status.lock().unwrap();
        let was = std::mem::take(&mut status.ready);
        status.settle();
        was
    }

    /// Its workspace is gone and nothing in it needs the user any more: the
    /// program in it is neither working nor blocked on an answer, no turn
    /// ended there unseen, and it isn't on screen. `shown` is the terminal
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

    /// SIGHUP to the shell alone is enough: it leads the terminal's session,
    /// and when it exits the kernel hangs up on the foreground job too
    /// (`closing_hangs_up_on_the_foreground_job`; an explicit `killpg` of
    /// that group was tried and made no difference). Jobs the user put in
    /// the background are the shell's to hang up, as in any terminal.
    fn hang_up(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }

    /// Send to the attached sink, if any; drop it if its receiver is gone.
    fn deliver(output: &mut Output, chunk: Chunk) {
        if let Some(sink) = &output.sink {
            if !sink(chunk) {
                output.sink = None;
            }
        }
    }
}

pub type Hook = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

#[derive(Default)]
pub struct Terminals {
    by_path: Arc<Mutex<HashMap<String, Arc<Terminal>>>>,
    /// Told about every exit and status change, attached or not. A webview
    /// that reloaded marks running terminals from `list` before attaching to
    /// any of them, and a shell that exits meanwhile has no sink to report
    /// through.
    hook: Option<Hook>,
    /// The terminal the user is looking at: on screen, in the focused window.
    viewing: Arc<Mutex<Option<String>>>,
    /// The terminal on screen, focused window or not. Switching to another
    /// app is not leaving it.
    shown: Arc<Mutex<Option<String>>>,
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

impl Terminals {
    pub fn new(hook: Hook) -> Self {
        Self {
            by_path: Arc::default(),
            hook: Some(hook),
            viewing: Arc::default(),
            shown: Arc::default(),
        }
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
    /// if there is none. Returns whether a new shell was started. The backlog
    /// of an existing terminal is replayed into the sink before anything new.
    pub fn open(
        &self,
        project_id: &str,
        path: &str,
        size: PtySize,
        command: CommandBuilder,
        sink: Sink,
    ) -> Result<bool, String> {
        let key = key(path);
        let mut map = self.by_path.lock().unwrap();
        if let Some(term) = map.get(&key) {
            let mut output = term.output.lock().unwrap();
            if !output.exited {
                // Replaces whoever was attached: that was a webview since
                // reloaded, or one that's about to be.
                let alive =
                    output.backlog.is_empty() || sink(Chunk::Data(output.backlog.clone()));
                output.sink = alive.then_some(sink);
                drop(output);
                let _ = term.master.lock().unwrap().resize(size);
                return Ok(false);
            }
        }

        // portable-pty quietly falls back to $HOME for a missing directory; a
        // shell somewhere else than the card says is worse than an error.
        if !Path::new(path).is_dir() {
            return Err(format!("{path} does not exist"));
        }

        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("could not open a terminal: {e}"))?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("could not start a shell: {e}"))?;
        // Ours must go, or the reader never sees end-of-file: the pty stays
        // open for as long as anything holds its slave side.
        drop(pair.slave);
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let shell_pid = child.process_id().map(|pid| pid as i32);
        let term = Arc::new(Terminal {
            info: TerminalInfo {
                path: key.clone(),
                project_id: project_id.to_string(),
                status: None,
                ready: false,
                active_at: 0,
                waiting_since: None,
                message: None,
            },
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(child.clone_killer()),
            output: Mutex::new(Output {
                backlog: Vec::new(),
                sink: Some(sink),
                exited: false,
            }),
            shell_pid,
            status: Mutex::new(Status {
                active_at: now_ms(),
                ..Status::default()
            }),
        });
        map.insert(key.clone(), Arc::clone(&term));
        drop(map);

        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        let reading = Arc::clone(&term);
        let status_hook = self.hook.clone();
        let viewing = Arc::clone(&self.viewing);
        let shown = Arc::clone(&self.shown);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut scanner = StatusScanner::default();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut output = reading.output.lock().unwrap();
                        append_backlog(&mut output.backlog, &buf[..n]);
                        Terminal::deliver(&mut output, Chunk::Data(buf[..n].to_vec()));
                        drop(output);
                        let viewed =
                            viewing.lock().unwrap().as_deref() == Some(&reading.info.path);
                        if reading.follow_status(scanner.feed(&buf[..n]), viewed) {
                            if let Some(hook) = &status_hook {
                                hook(TerminalEvent::Status(&reading.info()));
                            }
                            // Claude Code finishing the turn in which it
                            // removed its own worktree, typically.
                            if reading.outlived(shown.lock().unwrap().as_deref()) {
                                reading.hang_up();
                            }
                        }
                    }
                }
            }
            let _ = drained_tx.send(());
        });

        let by_path = Arc::clone(&self.by_path);
        let hook = self.hook.clone();
        std::thread::spawn(move || {
            let code = child
                .wait()
                .map(|status| status.exit_code() as i32)
                .unwrap_or(-1);
            let _ = drained_rx.recv_timeout(DRAIN_GRACE);
            // Out of the registry first, so an open racing this exit starts a
            // new shell rather than attaching to a dead one.
            let mut map = by_path.lock().unwrap();
            if map.get(&key).is_some_and(|t| Arc::ptr_eq(t, &term)) {
                map.remove(&key);
            }
            drop(map);
            let mut output = term.output.lock().unwrap();
            output.exited = true;
            Terminal::deliver(&mut output, Chunk::Exit(code));
            output.sink = None;
            drop(output);
            *term.status.lock().unwrap() = Status::default();
            if let Some(hook) = hook {
                hook(TerminalEvent::Exit(&term.info()));
            }
        });

        Ok(true)
    }

    fn get(&self, path: &str) -> Result<Arc<Terminal>, String> {
        self.by_path
            .lock()
            .unwrap()
            .get(&key(path))
            .cloned()
            .ok_or_else(|| format!("no terminal in {path}"))
    }

    /// Type `data` into the terminal. Returns when it was last active,
    /// which this makes now unless `data` is only a focus report: xterm sends
    /// one on its own whenever the terminal is clicked into or left, if the
    /// program asked for them, as Claude Code does. Looking isn't activity.
    pub fn write(&self, path: &str, data: &[u8]) -> Result<u64, String> {
        let term = self.get(path)?;
        let mut writer = term.writer.lock().unwrap();
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        drop(writer);
        let mut status = term.status.lock().unwrap();
        if !matches!(data, b"\x1b[I" | b"\x1b[O") {
            status.active_at = now_ms();
        }
        Ok(status.active_at)
    }

    pub fn resize(&self, path: &str, size: PtySize) -> Result<(), String> {
        let term = self.get(path)?;
        let master = term.master.lock().unwrap();
        master.resize(size).map_err(|e| e.to_string())
    }

    /// Hang up on the shell and whatever it is running in the foreground.
    /// The exit is reported through the sink like any other. Returns whether
    /// there was a terminal to close.
    pub fn close(&self, path: &str) -> bool {
        let Ok(term) = self.get(path) else {
            return false;
        };
        term.hang_up();
        true
    }

    /// Close every terminal whose directory has gone and that nothing in
    /// needs the user any more (`Terminal::outlived`), and return their
    /// paths. Asked whenever that may have just become true: a refresh that
    /// may be the first to notice the directory is gone, the user leaving a
    /// terminal, and its program going quiet (in the reader).
    pub fn reap(&self) -> Vec<String> {
        let shown = self.shown.lock().unwrap().clone();
        let terms: Vec<Arc<Terminal>> = self.by_path.lock().unwrap().values().cloned().collect();
        terms
            .into_iter()
            .filter(|term| term.outlived(shown.as_deref()))
            .map(|term| {
                term.hang_up();
                term.info.path.clone()
            })
            .collect()
    }

    /// Which terminal is on screen, if any, and whether its window has
    /// focus — together, whether the user is looking at it. Looking at one
    /// clears its ready flag, which is reported like any status change.
    /// Leaving one may be the last thing a removed workspace's terminal was
    /// waiting for.
    pub fn view(&self, path: Option<&str>, focused: bool) {
        let path = path.map(key);
        *self.shown.lock().unwrap() = path.clone();
        let viewing = path.filter(|_| focused);
        *self.viewing.lock().unwrap() = viewing.clone();
        if let Some(term) = viewing.and_then(|p| self.get(&p).ok()) {
            if term.seen() {
                if let Some(hook) = &self.hook {
                    hook(TerminalEvent::Status(&term.info()));
                }
            }
        }
        self.reap();
    }

    /// The process leading the terminal's foreground, other than its shell:
    /// whatever program is running in it, such as Claude Code.
    pub fn foreground(&self, path: &str) -> Option<i32> {
        let term = self.get(path).ok()?;
        let leader = term.master.lock().unwrap().process_group_leader()?;
        (Some(leader) != term.shell_pid).then_some(leader)
    }

    /// Mark a terminal by hand, reported like any status change.
    pub fn mark(&self, path: &str, mark: Mark) -> Result<(), String> {
        let term = self.get(path)?;
        term.mark(mark);
        if let Some(hook) = &self.hook {
            hook(TerminalEvent::Status(&term.info()));
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        self.by_path
            .lock()
            .unwrap()
            .values()
            .map(|t| t.info())
            .collect()
    }

    /// How many terminals wait for the user: blocked on an answer, or with a
    /// turn that ended unseen. The ones `terminalMark` draws a box around.
    pub fn waiting(&self) -> usize {
        self.list()
            .iter()
            .filter(|info| info.waiting_since.is_some())
            .count()
    }
}

const STATUS_OPENER: &[u8] = b"\x1b]7501;";

/// Longest a status sequence may grow while its terminator is awaited. Real
/// ones are a few dozen bytes; anything longer is not one, and is dropped
/// rather than buffered.
const MAX_STATUS: usize = 4096;

/// Finds OSC 7501 sequences in a stream of output, including ones split
/// across reads.
#[derive(Default)]
struct StatusScanner {
    /// The unfinished start of a sequence from the previous read.
    carry: Vec<u8>,
}

impl StatusScanner {
    /// The payloads of the sequences completed by `data`.
    fn feed(&mut self, data: &[u8]) -> Vec<String> {
        let joined;
        let buf: &[u8] = if self.carry.is_empty() {
            data
        } else {
            self.carry.extend_from_slice(data);
            joined = std::mem::take(&mut self.carry);
            &joined
        };

        let mut found = Vec::new();
        let mut at = 0;
        while let Some(i) = find(&buf[at..], STATUS_OPENER) {
            let opener = at + i;
            let start = opener + STATUS_OPENER.len();
            match terminator(&buf[start..]) {
                Some((len, skip)) => {
                    found.push(String::from_utf8_lossy(&buf[start..start + len]).into_owned());
                    at = start + len + skip;
                }
                None => {
                    if buf.len() - opener <= MAX_STATUS {
                        self.carry = buf[opener..].to_vec();
                    }
                    return found;
                }
            }
        }
        // An opener cut off by the end of the read: keep what could still
        // become one.
        let from = buf.len().saturating_sub(STATUS_OPENER.len() - 1).max(at);
        if let Some(k) = (from..buf.len()).find(|&k| STATUS_OPENER.starts_with(&buf[k..])) {
            self.carry = buf[k..].to_vec();
        }
        found
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Where an OSC payload ends — BEL, or ESC `\` — and how long the
/// terminator is. `None` when it hasn't arrived yet.
fn terminator(rest: &[u8]) -> Option<(usize, usize)> {
    for (j, &b) in rest.iter().enumerate() {
        match b {
            0x07 => return Some((j, 1)),
            0x1b => {
                return match rest.get(j + 1) {
                    Some(b'\\') => Some((j, 2)),
                    // Malformed: whatever comes next starts something else.
                    Some(_) => Some((j, 1)),
                    None => None,
                }
            }
            _ => {}
        }
    }
    None
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

/// Keep the newest `BACKLOG` bytes, cut at a line break so a replay doesn't
/// start in the middle of an escape sequence.
fn append_backlog(backlog: &mut Vec<u8>, data: &[u8]) {
    backlog.extend_from_slice(data);
    if backlog.len() > BACKLOG {
        let excess = backlog.len() - BACKLOG;
        let cut = backlog[excess..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|i| excess + i + 1)
            .unwrap_or(excess);
        backlog.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::Receiver;
    use std::time::Instant;

    const SIZE: PtySize = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    fn sink() -> (Sink, Receiver<Chunk>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(move |chunk| tx.send(chunk).is_ok()), rx)
    }

    /// A plain `sh`, so the tests don't run the user's shell profile.
    fn sh(dir: &Path) -> CommandBuilder {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.cwd(dir);
        cmd.env("PS1", "$ ");
        cmd
    }

    /// Collect output until `want` shows up, or fail after a few seconds.
    fn read_until(rx: &Receiver<Chunk>, want: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Chunk::Data(bytes)) => seen.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Chunk::Exit(code)) => seen.push_str(&format!("<exit {code}>")),
                Err(_) => {}
            }
            if seen.contains(want) {
                return seen;
            }
        }
        panic!("never saw {want:?} in {seen:?}");
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
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
        assert!(terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap());
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        let started = terminals.list()[0].active_at;
        assert!(started > 0, "a new terminal is active from the start");

        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(terminals.write(&path, b"\x1b[I").unwrap(), started);
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo first-$((40+2))\n").unwrap();
        read_until(&rx, "first-42");
        drop(rx);

        let (tx, rx) = sink();
        assert!(!terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap(), "no second shell");
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
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
    fn a_missing_directory_is_an_error_not_home() {
        let terminals = Terminals::default();
        let (tx, _rx) = sink();
        let missing = "/nonexistent/parallelist-workspace";
        let err = terminals
            .open("p", missing, SIZE, sh(Path::new(missing)), tx)
            .unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
    }

    #[test]
    fn the_backlog_keeps_the_newest_output_from_a_line_start() {
        let mut backlog = Vec::new();
        for i in 0..BACKLOG / 8 {
            append_backlog(&mut backlog, format!("{i:07}\n").as_bytes());
        }
        append_backlog(&mut backlog, b"last\n");
        assert!(backlog.len() <= BACKLOG);
        assert!(backlog.ends_with(b"last\n"));
        assert!(backlog[..8].ends_with(b"\n") && backlog[..7].iter().all(u8::is_ascii_digit));
    }

    #[test]
    fn status_sequences_are_found_across_reads() {
        let mut scanner = StatusScanner::default();
        assert_eq!(scanner.feed(b"text\x1b]7501;state=idle:app=x\x07more"), ["state=idle:app=x"]);
        // Split in the opener, in the payload and in an ESC \\ terminator.
        assert!(scanner.feed(b"abc\x1b]75").is_empty());
        assert!(scanner.feed(b"01;state=wor").is_empty());
        assert!(scanner.feed(b"king\x1b").is_empty());
        assert_eq!(scanner.feed(b"\\tail"), ["state=working"]);
        // Other OSCs, and a title that merely mentions the number, are not it.
        assert!(scanner.feed(b"\x1b]0;7501;x\x07\x1b]133;A\x07").is_empty());
        // A runaway sequence is dropped rather than buffered forever.
        assert!(scanner.feed(b"\x1b]7501;").is_empty());
        assert!(scanner.feed(&vec![b'x'; MAX_STATUS * 2]).is_empty());
        assert!(scanner.carry.is_empty());
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

    /// Make the reporter `ready` starts report `state`.
    fn report(terminals: &Terminals, path: &str, state: &str) {
        terminals.write(path, format!("{state}\n").as_bytes()).unwrap();
    }

    #[test]
    fn a_turn_that_ends_out_of_sight_is_ready_until_seen() {
        let dir = temp_dir("ready");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, _rx) = sink();
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        // A program in the foreground reporting each state it reads, as
        // Claude Code stays in the foreground between turns.
        terminals
            .write(
                &path,
                b"sh -c 'while read s; do printf \"\\033]7501;state=%s\\007\" \"$s\"; done'\n",
            )
            .unwrap();
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        report(&terminals, &path, "working");
        assert_eq!(next(), (Some("working".into()), false));
        assert_eq!(terminals.list()[0].waiting_since, None);
        report(&terminals, &path, "done");
        assert_eq!(next(), (Some("done".into()), true), "ended while nobody looked");
        assert!(terminals.list()[0].ready);
        assert_eq!(terminals.waiting(), 1);
        let since = terminals.list()[0].waiting_since.expect("waiting");

        terminals.view(Some(&path), true);
        assert_eq!(next(), (Some("done".into()), false), "seen");
        assert_eq!(terminals.waiting(), 0);
        assert_eq!(terminals.list()[0].waiting_since, None);

        report(&terminals, &path, "working");
        assert_eq!(next(), (Some("working".into()), false));
        report(&terminals, &path, "done");
        assert_eq!(next(), (Some("done".into()), false), "ended in plain sight");
        report(&terminals, &path, "blocked");
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        terminals
            .write(
                &path,
                b"sh -c 'while read s; do printf \"\\033]7501;state=%s\\007\" \"$s\"; done'\n",
            )
            .unwrap();
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        report(&terminals, &path, "working");
        assert_eq!(next(), (Some("working".into()), false));
        terminals.mark(&path, Mark::Unseen).unwrap();
        assert_eq!(next(), (Some("working".into()), true), "still working");
        terminals.view(Some(&path), true);
        assert_eq!(next(), (Some("working".into()), false), "seen");

        terminals.mark(&path, Mark::Blocked).unwrap();
        assert_eq!(next(), (Some("blocked".into()), false));
        assert_eq!(terminals.waiting(), 1);
        let since = terminals.list()[0].waiting_since.expect("waiting");
        terminals.mark(&path, Mark::Unseen).unwrap();
        assert_eq!(next(), (Some("blocked".into()), true));
        assert_eq!(terminals.list()[0].waiting_since, Some(since), "still the same wait");
        report(&terminals, &path, "working");
        assert_eq!(next(), (Some("working".into()), false), "the program's word wins");
        assert_eq!(terminals.list()[0].waiting_since, None);

        assert!(terminals.mark("/nowhere", Mark::Blocked).is_err());
        assert!(terminals.close(&path));
    }

    /// Claude Code removing its own worktree: the terminal stays while the
    /// turn runs, while its end is unseen and while it is on screen, and
    /// closes once the user has seen it and left.
    #[test]
    fn a_removed_workspace_keeps_its_terminal_until_nothing_needs_the_user() {
        let dir = temp_dir("removed");
        let path = dir.to_string_lossy().to_string();
        let (hook, statuses) = status_hook();
        let terminals = Terminals::new(hook);
        let (tx, rx) = sink();
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        terminals
            .write(
                &path,
                b"sh -c 'while read s; do printf \"\\033]7501;state=%s\\007\" \"$s\"; done'\n",
            )
            .unwrap();
        let next = || statuses.recv_timeout(Duration::from_secs(5)).expect("a status");

        report(&terminals, &path, "working");
        assert_eq!(next(), (Some("working".into()), false));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(terminals.reap().is_empty(), "still working");

        report(&terminals, &path, "done");
        assert_eq!(next(), (Some("done".into()), true));
        assert!(terminals.reap().is_empty(), "its last turn is unseen");

        terminals.view(Some(&path), true);
        assert_eq!(next(), (Some("done".into()), false), "seen");
        terminals.view(Some(&path), false);
        assert_eq!(terminals.list().len(), 1, "on screen in another app's shadow");

        terminals.view(None, true);
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo up\n").unwrap();
        read_until(&rx, "up");
        assert!(terminals.reap().is_empty(), "the directory is still there");

        // Quiet the moment it reports so, without waiting for a refresh.
        terminals
            .write(
                &path,
                b"sh -c 'while read s; do printf \"\\033]7501;state=%s\\007\" \"$s\"; done'\n",
            )
            .unwrap();
        report(&terminals, &path, "working");
        std::thread::sleep(Duration::from_millis(200));
        std::fs::remove_dir_all(&dir).unwrap();
        report(&terminals, &path, "idle");
        read_until(&rx, "<exit");

        let dir = temp_dir("removed-plain");
        let path = dir.to_string_lossy().to_string();
        let (tx, rx) = sink();
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();
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
        terminals.open("p", &path, SIZE, sh(&dir), tx).unwrap();

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
