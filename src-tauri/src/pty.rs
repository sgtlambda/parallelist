//! Pseudo-terminals and the shells in them, and nothing about what their
//! programs report: starting a shell, reading what it prints, answering the
//! OSC 7501 probe, and noticing it exit. `terminal.rs` builds the statuses
//! on top of what is recorded here.
//!
//! A [`Host`] is where the ptys live. [`Local`] keeps them in the process
//! that made it: the relay (`relay.rs`), so shells outlive the app, or the
//! app itself when no relay can be had, and in tests. `relay::Client` is the
//! same interface over the relay's socket.
//!
//! Everything that changes a terminal's status is an entry in its **log**:
//! a status its program printed, the foreground changing hands, the user
//! typing, a mark the app made. The host numbers the entries, and the app
//! works the status out from them (`Status::apply` in `terminal.rs`), so
//! what the statuses mean can change without a new relay. The app saves
//! what it has worked out as a **checkpoint**, and the host keeps every
//! entry after it. The next app to attach, or an app that lost its
//! connection, replays those, whatever happened in between: a turn that
//! ended while no app was attached, or in the moment before one quit.
//!
//! Output is numbered too, by its **offset** in everything the shell has
//! printed, so an app that catches up after a reconnect takes exactly what
//! it hasn't seen.
//!
//! The probe is answered here rather than in `terminal.rs`, so a program
//! started while no app is attached to the relay still gets its answer, and
//! with it reports a status for the rest of its life.

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use crate::tasks::now_ms;

/// How much recent output a terminal keeps for replay on reattach. A few
/// screens of a noisy build; xterm keeps the real scrollback on its side.
pub const BACKLOG: usize = 512 * 1024;

/// How long to wait for the last output after the shell exits. A background
/// job the shell left behind can hold the pty open indefinitely, and the exit
/// still has to be reported.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

/// Input waiting for a program that isn't reading it. Past this, typing is
/// refused rather than queued without end.
const MAX_QUEUED_INPUT: usize = 1 << 20;

/// Entries kept past a terminal's checkpoint. A program reporting what it
/// does while no app is attached adds one per report; the oldest go first,
/// and only the newest decide what the status is now.
const MAX_ENTRIES: usize = 4096;

/// How often the foreground is looked at between reads. A program that
/// starts or ends without printing anything still changes hands.
const LEADER_POLL: Duration = Duration::from_millis(250);

/// How many exits a host remembers, so an app that was away when a shell
/// exited can still be told its code and see its last output.
const RECENT_EXITS: usize = 64;

/// How much of an exited shell's last output is kept for that app: the error
/// a failed build ended on, not the whole build.
const EXIT_TAIL: usize = 16 * 1024;

const STATUS_OPENER: &[u8] = b"\x1b]7501;";

/// Longest a status sequence may grow while its terminator is awaited. Real
/// ones are a few dozen bytes; anything longer is not one, and is dropped
/// rather than buffered.
const MAX_STATUS: usize = 4096;

/// A shell to start, as data, so it can be sent to the relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spawn {
    /// Empty for the user's login shell (`CommandBuilder::new_default_prog`).
    pub argv: Vec<String>,
    pub cwd: String,
    /// The whole environment, not additions to the host's: the relay was
    /// started with whatever the app had then, which may be out of date.
    pub env: Vec<(String, String)>,
}

impl Spawn {
    pub fn from_command(cmd: &CommandBuilder) -> Self {
        Spawn {
            argv: if cmd.is_default_prog() {
                Vec::new()
            } else {
                cmd.get_argv().iter().map(|a| a.to_string_lossy().into_owned()).collect()
            },
            cwd: cmd.get_cwd().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
            env: cmd
                .iter_full_env_as_str()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn command(&self) -> CommandBuilder {
        let mut cmd = if self.argv.is_empty() {
            CommandBuilder::new_default_prog()
        } else {
            CommandBuilder::from_argv(self.argv.iter().map(Into::into).collect())
        };
        cmd.env_clear();
        for (name, value) in &self.env {
            cmd.env(name, value);
        }
        if !self.cwd.is_empty() {
            cmd.cwd(&self.cwd);
        }
        cmd
    }
}

/// Something that happened to a terminal that may change its status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Note {
    /// An OSC 7501 payload its program printed. The probe isn't one: it has
    /// been answered.
    Status(String),
    /// The process leading its foreground changed: the shell, or a program
    /// it started.
    Leader(Option<i32>),
    /// The user typed into it.
    Typed,
    /// Something the app recorded, which only the app interprets (a mark).
    App(serde_json::Value),
}

/// One numbered entry in a terminal's log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub seq: u64,
    /// When, in epoch milliseconds.
    pub at: u64,
    pub note: Note,
}

/// What the app last saved about a terminal, and the last entry it covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub seq: u64,
    pub state: serde_json::Value,
}

/// A running terminal as a host describes it to an app that takes it over.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Survivor {
    pub id: String,
    pub path: String,
    pub shell_pid: Option<i32>,
    /// Where `backlog` starts in everything the shell has printed.
    pub offset: u64,
    /// Its recent output. Sent as a frame's body, not in its header.
    #[serde(skip)]
    pub backlog: Vec<u8>,
    pub checkpoint: Option<Checkpoint>,
    /// What happened since the checkpoint, oldest first.
    pub entries: Vec<Entry>,
}

/// A shell that exited lately, as an app that was away then is told of it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Exited {
    pub id: String,
    pub code: i32,
    /// Where `tail` starts in everything the shell printed.
    pub offset: u64,
    /// The last of its output, base64 in the frame's header.
    #[serde(with = "base64_bytes")]
    pub tail: Vec<u8>,
}

mod base64_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// Every terminal a host has, and the shells that exited lately.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub terminals: Vec<Survivor>,
    /// Oldest first.
    pub exited: Vec<Exited>,
}

/// What a host reports.
#[derive(Debug, Clone)]
pub enum Event {
    /// Output from `offset` on in everything the shell has printed.
    Output { id: String, offset: u64, data: Vec<u8> },
    Entry { id: String, entry: Entry },
    /// The shell is gone, and the last of its output has been sent.
    Exit { id: String, code: i32 },
    /// A running terminal the app didn't know: one it asked to open in a
    /// workspace that already had one. Sent before the answer to the open.
    Adopt(Survivor),
    /// The connection to the host was lost and made again: everything the
    /// host has now, which is the truth about which terminals still run.
    Resync(Snapshot),
    /// Another app took the terminals over.
    Superseded,
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// What `Host::open` did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Opened {
    /// Started a shell, with this pid.
    Started { pid: Option<i32> },
    /// The workspace already had a terminal, under this id. It has been
    /// sent as an `Event::Adopt` first.
    Existing { id: String },
}

/// Where ptys live. Ids are the caller's, unique for as long as the host
/// lives (the app uses UUIDs, since a relay outlives the app's counters).
/// One terminal per path: opening a path that has one answers with it.
pub trait Host: Send + Sync {
    /// Start a shell for `path`, keeping `state` as its first checkpoint.
    fn open(
        &self,
        id: &str,
        path: &str,
        spawn: &Spawn,
        size: PtySize,
        state: serde_json::Value,
    ) -> Result<Opened, String>;
    /// Send input. `typed_at` when the user typed it, which is logged.
    fn write(&self, id: &str, data: &[u8], typed_at: Option<u64>) -> Result<(), String>;
    fn resize(&self, id: &str, size: PtySize) -> Result<(), String>;
    /// SIGHUP to the shell. The exit is reported as an event like any other.
    fn hang_up(&self, id: &str);
    /// Log something only the app interprets.
    fn note(&self, id: &str, at: u64, value: serde_json::Value) -> Result<(), String>;
    /// Keep `state` as covering every entry up to `seq`, and drop those.
    fn checkpoint(&self, id: &str, seq: u64, state: serde_json::Value);
    /// Where events go. Set once; events before it are dropped.
    fn set_sink(&self, sink: Sink);
    /// Wait until everything sent so far has reached the host: before the
    /// app quits, so its last checkpoints and hang-ups aren't lost.
    fn flush(&self) {}
    /// Whether its shells outlive the app.
    fn persistent(&self) -> bool {
        false
    }
}

/// A terminal's log, checkpoint and output, as the host keeps them.
#[derive(Default)]
struct Book {
    backlog: Vec<u8>,
    /// Everything the shell has printed, counted: the offset of what comes
    /// next.
    end: u64,
    last_seq: u64,
    checkpoint: Option<Checkpoint>,
    entries: VecDeque<Entry>,
    leader: Option<i32>,
}

impl Book {
    /// Log `note`, unless it changes nothing: the same leader again. Typing
    /// right after typing replaces it, so a busy shell's log stays short.
    fn push(&mut self, at: u64, note: Note) -> Option<Entry> {
        if let Note::Leader(leader) = note {
            if leader == self.leader {
                return None;
            }
            self.leader = leader;
        }
        if note == Note::Typed && self.entries.back().is_some_and(|e| e.note == Note::Typed) {
            self.entries.pop_back();
        }
        self.last_seq += 1;
        let entry = Entry { seq: self.last_seq, at, note };
        self.entries.push_back(entry.clone());
        if self.entries.len() > MAX_ENTRIES {
            self.entries.pop_front();
        }
        Some(entry)
    }

    fn survivor(&self, pty: &Pty, with_backlog: bool) -> Survivor {
        let backlog = if with_backlog { self.backlog.clone() } else { Vec::new() };
        Survivor {
            id: pty.id.clone(),
            path: pty.path.clone(),
            shell_pid: pty.shell_pid,
            offset: self.end - backlog.len() as u64,
            backlog,
            checkpoint: self.checkpoint.clone(),
            entries: self.entries.iter().cloned().collect(),
        }
    }
}

struct Pty {
    id: String,
    path: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    shell_pid: Option<i32>,
    /// To the thread that writes input, so nothing waits on a program that
    /// isn't reading: not the relay's requests, not other terminals.
    input: mpsc::Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    book: Mutex<Book>,
}

struct Shared {
    ptys: Mutex<HashMap<String, Arc<Pty>>>,
    /// Held from looking for a path's terminal until a new one is in
    /// `ptys`, so two opens of one path can't both start a shell.
    opening: Mutex<()>,
    sink: OnceLock<Sink>,
    /// Held while anything is logged or output is kept and sent on, so the
    /// relay can take every terminal and attach an app between two of
    /// those: nothing is kept without also going to whoever is attached.
    gate: Mutex<()>,
    keep_backlog: bool,
    exited: Mutex<VecDeque<Exited>>,
}

impl Shared {
    fn emit(&self, event: Event) {
        if let Some(sink) = self.sink.get() {
            sink(event);
        }
    }

    fn get(&self, id: &str) -> Result<Arc<Pty>, String> {
        self.ptys
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no terminal {id}"))
    }

    /// Log `note` and send it on.
    fn record(&self, pty: &Pty, at: u64, note: Note) {
        let _gate = self.gate.lock().unwrap();
        let entry = pty.book.lock().unwrap().push(at, note);
        if let Some(entry) = entry {
            self.emit(Event::Entry { id: pty.id.clone(), entry });
        }
    }
}

/// Ptys in this process.
pub struct Local {
    shared: Arc<Shared>,
}

impl Default for Local {
    fn default() -> Self {
        Self::new(false)
    }
}

impl Local {
    /// `keep_backlog` for the relay, which replays it to the next app. The app
    /// keeps its own for the webview.
    pub fn new(keep_backlog: bool) -> Self {
        let shared = Arc::new(Shared {
            ptys: Mutex::default(),
            opening: Mutex::default(),
            sink: OnceLock::new(),
            gate: Mutex::default(),
            keep_backlog,
            exited: Mutex::default(),
        });
        watch_leaders(Arc::downgrade(&shared));
        Local { shared }
    }

    pub fn is_empty(&self) -> bool {
        self.shared.ptys.lock().unwrap().is_empty()
    }

    /// Every terminal, handed to `then` under the gate: whatever `then` sets
    /// up to receive events gets everything after the snapshot, and nothing
    /// in it twice.
    pub fn with_snapshot<R>(&self, backlogs: bool, then: impl FnOnce(Snapshot) -> R) -> R {
        let _gate = self.shared.gate.lock().unwrap();
        let ptys: Vec<Arc<Pty>> = self.shared.ptys.lock().unwrap().values().cloned().collect();
        let snapshot = Snapshot {
            terminals: ptys
                .iter()
                .map(|pty| pty.book.lock().unwrap().survivor(pty, backlogs))
                .collect(),
            exited: self.shared.exited.lock().unwrap().iter().cloned().collect(),
        };
        then(snapshot)
    }

    /// Send a terminal to whoever is attached as one to take in
    /// (`Event::Adopt`): one started for a connection that isn't the app's,
    /// which the app would otherwise never hear of.
    pub fn announce(&self, id: &str) {
        let _gate = self.shared.gate.lock().unwrap();
        let Ok(pty) = self.shared.get(id) else { return };
        let survivor = pty.book.lock().unwrap().survivor(&pty, self.shared.keep_backlog);
        self.shared.emit(Event::Adopt(survivor));
    }

    /// Hang up on every shell.
    pub fn hang_up_all(&self) {
        let ptys: Vec<Arc<Pty>> = self.shared.ptys.lock().unwrap().values().cloned().collect();
        for pty in ptys {
            let _ = pty.killer.lock().unwrap().kill();
        }
    }
}

/// Note each terminal's foreground changing hands, read or no read. Ends
/// with the host.
fn watch_leaders(shared: Weak<Shared>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(LEADER_POLL);
        let Some(shared) = shared.upgrade() else { return };
        let ptys: Vec<Arc<Pty>> = shared.ptys.lock().unwrap().values().cloned().collect();
        for pty in ptys {
            let leader = pty.master.lock().unwrap().process_group_leader();
            if pty.book.lock().unwrap().leader != leader {
                shared.record(&pty, now_ms(), Note::Leader(leader));
            }
        }
    });
}

impl Host for Local {
    fn open(
        &self,
        id: &str,
        path: &str,
        spawn: &Spawn,
        size: PtySize,
        state: serde_json::Value,
    ) -> Result<Opened, String> {
        let shared = &self.shared;
        let _opening = shared.opening.lock().unwrap();
        {
            let _gate = shared.gate.lock().unwrap();
            let ptys = shared.ptys.lock().unwrap();
            let found = ptys.get(id).or_else(|| ptys.values().find(|p| p.path == path)).cloned();
            drop(ptys);
            if let Some(pty) = found {
                let survivor = pty.book.lock().unwrap().survivor(&pty, shared.keep_backlog);
                shared.emit(Event::Adopt(survivor));
                return Ok(Opened::Existing { id: pty.id.clone() });
            }
        }

        // portable-pty quietly falls back to $HOME for a missing directory; a
        // shell somewhere else than the card says is worse than an error.
        if !spawn.cwd.is_empty() && !Path::new(&spawn.cwd).is_dir() {
            return Err(format!("{} does not exist", spawn.cwd));
        }
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| format!("could not open a terminal: {e}"))?;
        let mut child = pair
            .slave
            .spawn_command(spawn.command())
            .map_err(|e| format!("could not start a shell: {e}"))?;
        // Ours must go, or the reader never sees end-of-file: the pty stays
        // open for as long as anything holds its slave side.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let shell_pid = child.process_id().map(|pid| pid as i32);

        let (input, inputs) = mpsc::channel::<Vec<u8>>();
        let queued = Arc::new(AtomicUsize::new(0));
        {
            let queued = Arc::clone(&queued);
            std::thread::spawn(move || {
                for data in inputs {
                    let written = writer.write_all(&data).and_then(|()| writer.flush());
                    queued.fetch_sub(data.len(), Ordering::SeqCst);
                    if written.is_err() {
                        return;
                    }
                }
            });
        }

        let pty = Arc::new(Pty {
            id: id.to_string(),
            path: path.to_string(),
            master: Mutex::new(pair.master),
            killer: Mutex::new(child.clone_killer()),
            shell_pid,
            input,
            queued,
            book: Mutex::new(Book {
                checkpoint: Some(Checkpoint { seq: 0, state }),
                ..Book::default()
            }),
        });
        shared.ptys.lock().unwrap().insert(id.to_string(), Arc::clone(&pty));

        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        let reading = Arc::clone(&pty);
        let reading_shared = Arc::clone(shared);
        std::thread::spawn(move || {
            let shared = reading_shared;
            let pty = reading;
            let mut buf = vec![0u8; 64 * 1024];
            let mut scanner = StatusScanner::default();
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let data = &buf[..n];
                let mut statuses = scanner.feed(data);
                if statuses.iter().any(|p| p.starts_with('?')) {
                    // The probe. Anything starting with `?` is taken as
                    // support.
                    let answer = b"\x1b]7501;?\x07".to_vec();
                    pty.queued.fetch_add(answer.len(), Ordering::SeqCst);
                    let _ = pty.input.send(answer);
                    statuses.retain(|p| !p.starts_with('?'));
                }
                let leader = pty.master.lock().unwrap().process_group_leader();
                let at = now_ms();
                let _gate = shared.gate.lock().unwrap();
                let mut book = pty.book.lock().unwrap();
                let offset = book.end;
                book.end += n as u64;
                if shared.keep_backlog {
                    append_backlog(&mut book.backlog, data);
                }
                // The foreground first: a status printed by a program that
                // has already exited is cleared by it.
                let mut entries: Vec<Entry> = book.push(at, Note::Leader(leader)).into_iter().collect();
                entries.extend(statuses.into_iter().filter_map(|p| book.push(at, Note::Status(p))));
                drop(book);
                shared.emit(Event::Output {
                    id: pty.id.clone(),
                    offset,
                    data: data.to_vec(),
                });
                for entry in entries {
                    shared.emit(Event::Entry { id: pty.id.clone(), entry });
                }
            }
            let _ = drained_tx.send(());
        });

        let exiting_shared = Arc::clone(shared);
        std::thread::spawn(move || {
            let shared = exiting_shared;
            let code = child.wait().map(|status| status.exit_code() as i32).unwrap_or(-1);
            let _ = drained_rx.recv_timeout(DRAIN_GRACE);
            let _gate = shared.gate.lock().unwrap();
            let mut ptys = shared.ptys.lock().unwrap();
            if ptys.get(&pty.id).is_some_and(|p| Arc::ptr_eq(p, &pty)) {
                ptys.remove(&pty.id);
            }
            drop(ptys);
            let book = pty.book.lock().unwrap();
            let tail = book.backlog[book.backlog.len().saturating_sub(EXIT_TAIL)..].to_vec();
            let offset = book.end - tail.len() as u64;
            drop(book);
            let mut exited = shared.exited.lock().unwrap();
            exited.push_back(Exited {
                id: pty.id.clone(),
                code,
                offset,
                tail,
            });
            if exited.len() > RECENT_EXITS {
                exited.pop_front();
            }
            drop(exited);
            shared.emit(Event::Exit { id: pty.id.clone(), code });
        });

        Ok(Opened::Started { pid: shell_pid })
    }

    fn write(&self, id: &str, data: &[u8], typed_at: Option<u64>) -> Result<(), String> {
        let pty = self.shared.get(id)?;
        if !data.is_empty() {
            if pty.queued.load(Ordering::SeqCst) + data.len() > MAX_QUEUED_INPUT {
                return Err("the program in this terminal isn't reading its input".into());
            }
            pty.queued.fetch_add(data.len(), Ordering::SeqCst);
            pty.input.send(data.to_vec()).map_err(|_| "the terminal has closed".to_string())?;
        }
        if let Some(at) = typed_at {
            self.shared.record(&pty, at, Note::Typed);
        }
        Ok(())
    }

    fn resize(&self, id: &str, size: PtySize) -> Result<(), String> {
        let pty = self.shared.get(id)?;
        let master = pty.master.lock().unwrap();
        master.resize(size).map_err(|e| e.to_string())
    }

    /// SIGHUP to the shell alone is enough: it leads the terminal's session,
    /// and when it exits the kernel hangs up on the foreground job too
    /// (`closing_hangs_up_on_the_foreground_job`; an explicit `killpg` of
    /// that group was tried and made no difference). Jobs the user put in
    /// the background are the shell's to hang up, as in any terminal.
    fn hang_up(&self, id: &str) {
        if let Ok(pty) = self.shared.get(id) {
            let _ = pty.killer.lock().unwrap().kill();
        }
    }

    fn note(&self, id: &str, at: u64, value: serde_json::Value) -> Result<(), String> {
        let pty = self.shared.get(id)?;
        self.shared.record(&pty, at, Note::App(value));
        Ok(())
    }

    fn checkpoint(&self, id: &str, seq: u64, state: serde_json::Value) {
        if let Ok(pty) = self.shared.get(id) {
            let mut book = pty.book.lock().unwrap();
            if book.checkpoint.as_ref().is_some_and(|c| c.seq > seq) {
                return;
            }
            book.entries.retain(|e| e.seq > seq);
            book.checkpoint = Some(Checkpoint { seq, state });
        }
    }

    fn set_sink(&self, sink: Sink) {
        let _ = self.shared.sink.set(sink);
    }
}

/// Finds OSC 7501 sequences in a stream of output, including ones split
/// across reads.
#[derive(Default)]
pub struct StatusScanner {
    /// The unfinished start of a sequence from the previous read.
    carry: Vec<u8>,
}

impl StatusScanner {
    /// The payloads of the sequences completed by `data`.
    pub fn feed(&mut self, data: &[u8]) -> Vec<String> {
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

/// Keep the newest `BACKLOG` bytes, cut at a line break so a replay doesn't
/// start in the middle of an escape sequence.
pub fn append_backlog(backlog: &mut Vec<u8>, data: &[u8]) {
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

    /// Typing after typing is one entry, the newest; the same foreground
    /// twice is none; anything in between keeps both.
    #[test]
    fn the_log_stays_short_without_losing_order() {
        let mut book = Book::default();
        assert!(book.push(1, Note::Leader(Some(10))).is_some());
        assert!(book.push(2, Note::Leader(Some(10))).is_none());
        book.push(3, Note::Typed);
        book.push(4, Note::Typed);
        book.push(5, Note::Status("state=done".into()));
        book.push(6, Note::Typed);
        let kept: Vec<(u64, &Note)> = book.entries.iter().map(|e| (e.at, &e.note)).collect();
        assert_eq!(
            kept,
            [
                (1, &Note::Leader(Some(10))),
                (4, &Note::Typed),
                (5, &Note::Status("state=done".into())),
                (6, &Note::Typed),
            ]
        );
        let seqs: Vec<u64> = book.entries.iter().map(|e| e.seq).collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "numbered in order: {seqs:?}");
    }
}
