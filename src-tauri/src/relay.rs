//! The relay: a small process that holds the ptys, so shells outlive the
//! app. Quitting, a crash, or installing a new build with `run.sh --build`
//! used to hang up on every shell, since the app held each pty's master side
//! and the kernel hangs up when that closes.
//!
//! The relay is deliberately simple: a `pty::Local` behind a Unix socket. It
//! starts shells, keeps each one's backlog and log (`pty.rs`), answers the
//! OSC 7501 probe and passes bytes through. What the statuses mean stays in
//! `terminal.rs`, in the app, so changing that needs no new relay.
//!
//! One relay per build: the build's branch, and whether it is a dev build
//! (`build_label`). Reinstalling a build gets its terminals back; a worktree's
//! build has its own, and two windows of different builds never share a
//! shell. The relay is the app's own binary started with `--relay`, in its
//! own session, and exits once it has no terminals and no app attached.
//!
//! **Nothing waits on anything else.** Each connection has a thread that
//! reads it and one that writes it, fed by a queue; each pty has its own
//! thread for input. A program that stops reading its input holds up only
//! its own terminal, and an app that stops reading is dropped once its queue
//! is full or a write has waited `STALLED`, rather than holding up the
//! shells. The app does the same on its side: nothing it sends waits for
//! the relay, and only an open waits for an answer.
//!
//! **A lost connection is not a lost relay.** The app reconnects, and the
//! relay sends every terminal it has, as at startup: the app keeps the ones
//! it knows, catching up on their output and log, takes in the ones it
//! doesn't, and reports as exited only those the relay no longer has
//! (`Event::Resync`). An app that is replaced by another of the same build
//! is told so (`Superseded`) and doesn't come back for them.
//!
//! **A relay is known to be running by the lock it holds** on its
//! `<hash>.lock` (`flock`), not by its pid: a pid can be reused once the
//! relay has gone. It is asked to end with a `Shutdown` frame in place of a
//! hello, which every relay from version 2 on understands, whatever its
//! version. Only a process that holds the lock, or a version 1 relay whose
//! command line says it is one, is ever sent a signal.
//!
//! Wire format: frames of a big-endian u32 header length, a u32 body length,
//! a JSON header, then the body as raw bytes (terminal output and input).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use portable_pty::PtySize;

use crate::pty::{self, Event, Host, Opened, Sink, Snapshot, Spawn, Survivor};

/// Bumped whenever a message changes shape. An app only talks to a relay of
/// its own version. `Shutdown` as the first frame must mean the same in
/// every version from 2 on.
pub const VERSION: u32 = 2;

/// How long a relay waits for its first app before giving up.
const FIRST_CLIENT: Duration = Duration::from_secs(15);
/// How long a relay with nothing left stays after its app has gone: an app
/// that lost its connection and is making it again still finds it, with the
/// last words and exit codes of shells that ended meanwhile.
const LINGER: Duration = Duration::from_secs(if cfg!(test) { 2 } else { 5 });
/// How long a hello may take to be answered, and a relay that holds its
/// lock to start answering.
const HANDSHAKE: Duration = Duration::from_secs(3);
/// How long an open may wait for its answer.
const REPLY: Duration = Duration::from_secs(10);
/// A write that waits this long means the other side has stopped reading.
const STALLED: Duration = Duration::from_secs(10);
/// Bytes waiting to be written to one connection. A full queue means the
/// other side has stopped reading, as surely as a stalled write does.
const QUEUE_BYTES: usize = 32 << 20;
/// How long a relay told to shut down waits for its shells to go.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// The longest wait between attempts to reconnect.
const RECONNECT_MAX: Duration = Duration::from_secs(2);

const MOVED: &str = "These terminals are open in another parallelist window.";

// ------------------------------------------------------------------ wire

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Up {
    /// `observe`: only list the terminals, and take hang-ups; the app that
    /// owns the relay stays attached.
    Hello { version: u32, observe: bool },
    /// In place of a hello: end every terminal, and the relay with them.
    Shutdown,
    Open {
        req: u64,
        id: String,
        path: String,
        spawn: Spawn,
        cols: u16,
        rows: u16,
        state: serde_json::Value,
    },
    /// The body is the input.
    Write { id: String, typed_at: Option<u64> },
    Resize { id: String, cols: u16, rows: u16 },
    Hangup { id: String },
    Note { id: String, at: u64, value: serde_json::Value },
    Checkpoint { id: String, seq: u64, state: serde_json::Value },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Reply {
    Opened { opened: Opened },
    Failed { error: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Down {
    /// A running terminal: one per terminal before `Welcome`, and after it
    /// one the app is to take in. The body is its backlog (empty to an
    /// observer).
    Terminal { survivor: Survivor },
    Welcome { version: u32, pid: u32, exited: Vec<pty::Exited> },
    Answer { req: u64, reply: Reply },
    /// The body is the output.
    Output { id: String, offset: u64 },
    Entry { id: String, entry: pty::Entry },
    Exit { id: String, code: i32 },
    /// Another app of this build attached; this one is no longer sent
    /// anything.
    Superseded,
}

const MAX_HEADER: usize = 16 << 20;
const MAX_BODY: usize = 64 << 20;

fn encode(head: &impl Serialize, body: &[u8]) -> io::Result<Vec<u8>> {
    let head = serde_json::to_vec(head)?;
    let mut buf = Vec::with_capacity(8 + head.len() + body.len());
    buf.extend_from_slice(&(head.len() as u32).to_be_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(body);
    Ok(buf)
}

fn write_frame(w: &mut impl Write, head: &impl Serialize, body: &[u8]) -> io::Result<()> {
    w.write_all(&encode(head, body)?)?;
    w.flush()
}

fn read_frame<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<(T, Vec<u8>)> {
    let mut lens = [0u8; 8];
    r.read_exact(&mut lens)?;
    let head_len = u32::from_be_bytes(lens[..4].try_into().unwrap()) as usize;
    let body_len = u32::from_be_bytes(lens[4..].try_into().unwrap()) as usize;
    if head_len > MAX_HEADER || body_len > MAX_BODY {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut head = vec![0u8; head_len];
    r.read_exact(&mut head)?;
    let mut body = vec![0u8; body_len];
    r.read_exact(&mut body)?;
    let head = serde_json::from_slice(&head)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((head, body))
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// One side of a connection, as far as sending goes: a queue, and a thread
/// that writes what is in it. Sending never waits. A write that fails or
/// waits `timeout`, or a full queue, closes the connection, which the
/// reading side sees as its end.
struct Peer {
    queue: mpsc::Sender<Out>,
    /// Bytes queued and not yet written.
    queued: Arc<AtomicUsize>,
    stream: UnixStream,
}

enum Out {
    Frame(Vec<u8>),
    /// Acknowledged once everything before it is written.
    Flush(mpsc::Sender<()>),
    /// Close once everything before it is written.
    Close,
}

impl Peer {
    fn start(stream: UnixStream, timeout: Duration) -> io::Result<Arc<Peer>> {
        stream.set_write_timeout(Some(timeout))?;
        let mut writing = stream.try_clone()?;
        let (queue, outs) = mpsc::channel::<Out>();
        let queued = Arc::new(AtomicUsize::new(0));
        let written = Arc::clone(&queued);
        std::thread::spawn(move || {
            for out in outs {
                match out {
                    Out::Frame(buf) => {
                        let sent = writing.write_all(&buf);
                        written.fetch_sub(buf.len(), Ordering::SeqCst);
                        if sent.is_err() {
                            break;
                        }
                    }
                    Out::Flush(ack) => {
                        let _ = ack.send(());
                    }
                    Out::Close => break,
                }
            }
            let _ = writing.shutdown(std::net::Shutdown::Both);
        });
        Ok(Arc::new(Peer { queue, queued, stream }))
    }

    /// Queue a frame. False, and the connection closed, when the other side
    /// has stopped reading.
    fn send(&self, head: &impl Serialize, body: &[u8]) -> bool {
        let Ok(buf) = encode(head, body) else { return true };
        let len = buf.len();
        if self.queued.fetch_add(len, Ordering::SeqCst) + len <= QUEUE_BYTES
            && self.queue.send(Out::Frame(buf)).is_ok()
        {
            return true;
        }
        self.kill();
        false
    }

    /// Close once what is queued has been written.
    fn close(&self) {
        if self.queue.send(Out::Close).is_err() {
            self.kill();
        }
    }

    /// Close now.
    fn kill(&self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// Wait up to `timeout` for what is queued to be written.
    fn flush(&self, timeout: Duration) {
        let (ack, acked) = mpsc::channel();
        if self.queue.send(Out::Flush(ack)).is_ok() {
            let _ = acked.recv_timeout(timeout);
        }
    }
}

// ----------------------------------------------------------------- files

/// What a relay says about itself next to its socket, readable without
/// speaking its protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub version: u32,
    pub pid: u32,
    /// The build it serves, as `build_label` names it.
    pub build: String,
    pub socket: PathBuf,
}

/// Where one build's relay keeps its socket, metadata, lock and log.
#[derive(Debug, Clone)]
pub struct Paths {
    pub socket: PathBuf,
    pub meta: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn new(dir: &Path, build: &str) -> Self {
        let key = format!("{:016x}", fnv1a(build.as_bytes()));
        let key = &key[..10];
        let mut socket = dir.join(format!("{key}.sock"));
        // A Unix socket's path is limited to 104 bytes on macOS. The
        // application support directory under a long user name can pass it.
        if socket.as_os_str().len() >= 104 {
            socket = std::env::temp_dir().join(format!("parallelist-{key}.sock"));
        }
        Paths {
            socket,
            meta: dir.join(format!("{key}.json")),
            lock: dir.join(format!("{key}.lock")),
            log: dir.join(format!("{key}.log")),
        }
    }
}

/// Stable across Rust versions, unlike `DefaultHasher`: the name of a
/// relay's files must not change with the compiler that built the app.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The build this binary is: its branch, as `run.sh` passes it to the
/// frontend (empty on main), and whether it is a dev build. Dev and
/// release builds of one branch are different builds.
pub fn build_label() -> String {
    let branch = option_env!("VITE_PARALLELIST_BRANCH")
        .filter(|b| !b.is_empty())
        .unwrap_or("main");
    if cfg!(debug_assertions) {
        format!("{branch} (dev)")
    } else {
        branch.to_string()
    }
}

pub fn read_meta(path: &Path) -> Option<Meta> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Write the metadata whole or not at all, so it is never read half-written.
fn write_meta(path: &Path, meta: &Meta) -> io::Result<()> {
    let tmp = path.with_extension(format!("json.{}", std::process::id()));
    std::fs::write(&tmp, serde_json::to_vec(meta)?)?;
    std::fs::rename(&tmp, path)
}

/// Take the lock that says this relay is the one running. A moment's grace,
/// since `held` takes it briefly to look.
fn take_lock(path: &Path) -> io::Result<Option<File>> {
    let file = File::options().create(true).truncate(false).write(true).open(path)?;
    for _ in 0..10 {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(Some(file));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(None)
}

/// Whether a relay holds the lock at `path`: it is running, whatever its pid
/// says. The lock goes with the process, so a crashed relay holds none.
pub fn held(path: &Path) -> bool {
    let Ok(file) = File::open(path) else { return false };
    let fd = file.as_raw_fd();
    if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(fd, libc::LOCK_UN) };
        false
    } else {
        true
    }
}

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Whether `pid` is a relay, by its command line: for version 1 relays,
/// which hold no lock, and before any signal is sent.
fn is_relay_process(pid: u32) -> bool {
    crate::proc::run_timeout(
        "ps",
        &["-ww", "-o", "args=", "-p", &pid.to_string()],
        None,
        Duration::from_secs(2),
    )
    .is_ok_and(|out| out.stdout.contains(" --relay "))
}

/// Whether the relay `meta` describes is still running.
fn running(dir: &Path, meta: &Meta) -> bool {
    if meta.version >= 2 {
        held(&Paths::new(dir, &meta.build).lock)
    } else {
        alive(meta.pid) && is_relay_process(meta.pid)
    }
}

/// Remove what the relay `meta` describes left behind, if it has gone, and
/// say whether it had. Done holding its build's lock, so no relay of that
/// build can start and bind its socket meanwhile, only to have it removed.
/// Not the lock file itself: a relay starting may be about to take it.
fn clear_if_gone(dir: &Path, meta: &Meta) -> bool {
    let paths = Paths::new(dir, &meta.build);
    let Ok(Some(lock)) = take_lock(&paths.lock) else {
        return false;
    };
    // Version 1 held no lock; its process says whether it runs.
    if meta.version < 2 && alive(meta.pid) && is_relay_process(meta.pid) {
        return false;
    }
    let _ = std::fs::remove_file(&paths.meta);
    let _ = std::fs::remove_file(&meta.socket);
    drop(lock);
    true
}

// ---------------------------------------------------------------- server

struct Relay {
    local: pty::Local,
    /// The app attached, if any. Everything the ptys report goes here.
    app: Mutex<Option<Arc<Peer>>>,
    connections: AtomicUsize,
    had_client: AtomicBool,
    /// When the last connection ended.
    last_left: Mutex<Option<Instant>>,
    /// When it was told to shut down.
    shutdown: Mutex<Option<Instant>>,
}

impl Relay {
    /// Called under the gate (`pty::Local`), so it is in order with any
    /// snapshot an app attaches with.
    fn on_event(&self, event: Event) {
        let mut app = self.app.lock().unwrap();
        let Some(peer) = app.as_ref() else { return };
        let sent = match event {
            Event::Output { id, offset, data } => peer.send(&Down::Output { id, offset }, &data),
            Event::Entry { id, entry } => peer.send(&Down::Entry { id, entry }, &[]),
            Event::Exit { id, code } => peer.send(&Down::Exit { id, code }, &[]),
            Event::Adopt(survivor) => {
                let body = survivor.backlog.clone();
                peer.send(&Down::Terminal { survivor }, &body)
            }
            Event::Resync(_) | Event::Superseded => true,
        };
        if !sent {
            // Stopped reading. Its terminals are in the next snapshot.
            *app = None;
        }
    }

    fn serve_connection(self: &Arc<Self>, stream: UnixStream) -> io::Result<()> {
        stream.set_read_timeout(Some(HANDSHAKE))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let observe = match read_frame::<Up>(&mut reader)?.0 {
            Up::Shutdown => {
                self.begin_shutdown();
                return Ok(());
            }
            Up::Hello { version, .. } if version != VERSION => {
                // It can read our version from the reply, and goes.
                let mut stream = stream;
                stream.set_write_timeout(Some(HANDSHAKE))?;
                let welcome = Down::Welcome {
                    version: VERSION,
                    pid: std::process::id(),
                    exited: Vec::new(),
                };
                return write_frame(&mut stream, &welcome, &[]);
            }
            Up::Hello { observe, .. } => observe,
            _ => return Ok(()),
        };
        stream.set_read_timeout(None)?;
        let peer = Peer::start(stream, STALLED)?;

        self.local.with_snapshot(!observe, |snapshot| {
            for mut survivor in snapshot.terminals {
                let body = std::mem::take(&mut survivor.backlog);
                peer.send(&Down::Terminal { survivor }, &body);
            }
            peer.send(
                &Down::Welcome {
                    version: VERSION,
                    pid: std::process::id(),
                    exited: snapshot.exited,
                },
                &[],
            );
            if !observe {
                let mut app = self.app.lock().unwrap();
                // Newest wins: the one before is a window that is gone, or
                // about to be. It is told, so it doesn't come back for them.
                if let Some(old) = app.replace(Arc::clone(&peer)) {
                    old.send(&Down::Superseded, &[]);
                    old.close();
                }
                self.had_client.store(true, Ordering::SeqCst);
            }
        });

        let result = self.requests(&mut reader, &peer, observe);
        let mut app = self.app.lock().unwrap();
        if app.as_ref().is_some_and(|a| Arc::ptr_eq(a, &peer)) {
            *app = None;
        }
        drop(app);
        peer.close();
        result
    }

    /// Requests, in order. None of them waits on a pty: input goes to the
    /// pty's own writer, and an open only starts a process.
    fn requests(&self, reader: &mut impl Read, peer: &Peer, observe: bool) -> io::Result<()> {
        loop {
            let (message, body) = read_frame::<Up>(reader)?;
            match message {
                Up::Hangup { id } => self.local.hang_up(&id),
                _ if observe => {}
                Up::Hello { .. } | Up::Shutdown => {}
                Up::Open { req, id, path, spawn, cols, rows, state } => {
                    let reply = match self.local.open(&id, &path, &spawn, size(cols, rows), state) {
                        Ok(opened) => {
                            // Asked for on a connection that is no longer the
                            // app's: an app since replaced, or a connection
                            // being made again. The app attached now has to
                            // hear of it, or it runs on unseen.
                            let current = self.app.lock().unwrap().as_ref().is_some_and(|a| std::ptr::eq(&**a, peer));
                            if matches!(opened, Opened::Started { .. }) && !current {
                                self.local.announce(&id);
                            }
                            Reply::Opened { opened }
                        }
                        Err(error) => Reply::Failed { error },
                    };
                    peer.send(&Down::Answer { req, reply }, &[]);
                }
                Up::Write { id, typed_at } => {
                    let _ = self.local.write(&id, &body, typed_at);
                }
                Up::Resize { id, cols, rows } => {
                    let _ = self.local.resize(&id, size(cols, rows));
                }
                Up::Note { id, at, value } => {
                    let _ = self.local.note(&id, at, value);
                }
                Up::Checkpoint { id, seq, state } => self.local.checkpoint(&id, seq, state),
            }
        }
    }

    fn begin_shutdown(&self) {
        self.shutdown.lock().unwrap().get_or_insert_with(Instant::now);
        self.local.hang_up_all();
    }

    /// Nothing to keep running for: no terminal, no app, and either an app
    /// has been and gone or none came in time. Or told to go, and its shells
    /// have gone or had their time.
    fn done(&self, started: Instant) -> bool {
        if let Some(asked) = *self.shutdown.lock().unwrap() {
            return self.local.is_empty() || asked.elapsed() > SHUTDOWN_GRACE;
        }
        let left_a_while_ago = self.last_left.lock().unwrap().is_none_or(|at| at.elapsed() > LINGER);
        self.local.is_empty()
            && self.connections.load(Ordering::SeqCst) == 0
            && left_a_while_ago
            && (self.had_client.load(Ordering::SeqCst) || started.elapsed() > FIRST_CLIENT)
    }
}

/// Run a relay until it has nothing left to do. Returns at once if another
/// relay of this build is already running.
pub fn serve(paths: &Paths, build: &str) -> io::Result<()> {
    if let Some(dir) = paths.meta.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let Some(lock) = take_lock(&paths.lock)? else {
        return Ok(());
    };
    if let Some(dir) = paths.socket.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Ours now: whatever relay left it is gone, since it held no lock.
    let _ = std::fs::remove_file(&paths.socket);
    let listener = UnixListener::bind(&paths.socket)?;
    listener.set_nonblocking(true)?;
    write_meta(
        &paths.meta,
        &Meta {
            version: VERSION,
            pid: std::process::id(),
            build: build.to_string(),
            socket: paths.socket.clone(),
        },
    )?;

    let relay = Arc::new(Relay {
        local: pty::Local::new(true),
        app: Mutex::default(),
        connections: AtomicUsize::new(0),
        had_client: AtomicBool::new(false),
        last_left: Mutex::default(),
        shutdown: Mutex::default(),
    });
    let weak = Arc::downgrade(&relay);
    relay.local.set_sink(Arc::new(move |event| {
        if let Some(relay) = weak.upgrade() {
            relay.on_event(event);
        }
    }));

    let started = Instant::now();
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                relay.connections.fetch_add(1, Ordering::SeqCst);
                let relay = Arc::clone(&relay);
                std::thread::spawn(move || {
                    let _ = relay.serve_connection(stream);
                    *relay.last_left.lock().unwrap() = Some(Instant::now());
                    relay.connections.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if relay.done(started) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
    // Ours: we hold the lock, so no other relay has taken the name over.
    let _ = std::fs::remove_file(&paths.meta);
    let _ = std::fs::remove_file(&paths.socket);
    drop(lock);
    Ok(())
}

/// `parallelist --relay <dir> <build>`: the relay process's `main`.
pub fn main(args: &[String]) -> i32 {
    let [dir, build] = args else {
        eprintln!("usage: --relay <dir> <build>");
        return 2;
    };
    // Not holding any workspace as its working directory.
    let _ = std::env::set_current_dir("/");
    match serve(&Paths::new(Path::new(dir), build), build) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("relay: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------- client

/// A connection that has said hello, and what the relay had then.
pub struct Handshake {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    snapshot: Snapshot,
}

/// Say hello and read the terminals the relay has, up to its welcome, within
/// `HANDSHAKE`. Errors as `Unsupported` if the relay isn't our version.
fn handshake(socket: &Path, observe: bool) -> io::Result<Handshake> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(HANDSHAKE))?;
    stream.set_write_timeout(Some(HANDSHAKE))?;
    write_frame(&mut stream, &Up::Hello { version: VERSION, observe }, &[])?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut snapshot = Snapshot::default();
    loop {
        match read_frame::<Down>(&mut reader)? {
            (Down::Terminal { mut survivor }, backlog) => {
                survivor.backlog = backlog;
                snapshot.terminals.push(survivor);
            }
            (Down::Welcome { version, .. }, _) if version != VERSION => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("relay speaks version {version}"),
                ))
            }
            (Down::Welcome { exited, .. }, _) => {
                snapshot.exited = exited;
                break;
            }
            _ => {}
        }
    }
    // The reader is a copy of this socket, so this clears it for both.
    stream.set_read_timeout(None)?;
    Ok(Handshake { stream, reader, snapshot })
}

/// Connects to the relay, starting one if need be.
type Dial = Box<dyn Fn() -> io::Result<Handshake> + Send + Sync>;
/// Whether the relay is running at all, reachable or not.
type Alive = Box<dyn Fn() -> bool + Send + Sync>;

enum Link {
    Up { generation: u64, peer: Arc<Peer> },
    /// Lost, and being made again.
    Down,
    Superseded,
    Closed,
}

struct Shared {
    dial: Dial,
    alive: Alive,
    link: Mutex<Link>,
    sink: OnceLock<Sink>,
    pending: Mutex<HashMap<u64, mpsc::Sender<Reply>>>,
    next: AtomicU64,
    /// The first connection's reader, until there is a sink to read for.
    first: Mutex<Option<BufReader<UnixStream>>>,
    /// The last connection's number: frames from an older one are ignored.
    generations: AtomicU64,
}

/// A relay, as a `pty::Host` for `terminal.rs`.
pub struct Client {
    shared: Arc<Shared>,
}

impl Client {
    /// A client on a connection that has said hello. `dial` connects again
    /// after it is lost; `alive` says whether the relay is running at all.
    fn new(handshake: Handshake, dial: Dial, alive: Alive) -> io::Result<(Arc<Client>, Snapshot)> {
        let Handshake { stream, reader, snapshot } = handshake;
        let peer = Peer::start(stream, REPLY)?;
        let client = Client {
            shared: Arc::new(Shared {
                dial,
                alive,
                link: Mutex::new(Link::Up { generation: 1, peer }),
                sink: OnceLock::new(),
                pending: Mutex::default(),
                next: AtomicU64::new(1),
                first: Mutex::new(Some(reader)),
                generations: AtomicU64::new(1),
            }),
        };
        Ok((Arc::new(client), snapshot))
    }

    /// A client of a relay that is running but didn't answer in time: it
    /// keeps trying, and its terminals arrive as a resync once it does.
    /// Until then nothing can be opened, which is better than a second
    /// shell beside one the relay already has.
    fn waiting(dial: Dial, alive: Alive) -> Arc<Client> {
        Arc::new(Client {
            shared: Arc::new(Shared {
                dial,
                alive,
                link: Mutex::new(Link::Down),
                sink: OnceLock::new(),
                pending: Mutex::default(),
                next: AtomicU64::new(1),
                first: Mutex::new(None),
                generations: AtomicU64::new(0),
            }),
        })
    }

    /// Attach to the relay at `socket`, reconnecting to it alone: for tests.
    #[cfg(test)]
    pub fn connect(socket: &Path) -> io::Result<(Arc<Client>, Snapshot)> {
        let socket = socket.to_path_buf();
        let redial = socket.clone();
        Client::new(
            handshake(&socket, false)?,
            Box::new(move || handshake(&redial, false)),
            Box::new(|| true),
        )
    }

    /// Drop the connection, as a relay that stopped reading would.
    #[cfg(test)]
    pub fn kill_connection(&self) {
        if let Link::Up { peer, .. } = &*self.shared.link.lock().unwrap() {
            peer.kill();
        }
    }

    fn send(&self, head: &Up, body: &[u8]) -> Result<(), String> {
        match &*self.shared.link.lock().unwrap() {
            Link::Up { peer, .. } => {
                if peer.send(head, body) {
                    Ok(())
                } else {
                    Err("the terminal relay isn't reading".into())
                }
            }
            Link::Down => Err("reconnecting to the terminal relay".into()),
            Link::Superseded => Err(MOVED.into()),
            Link::Closed => Err("the terminal relay is closed".into()),
        }
    }
}

impl Shared {
    fn emit(&self, event: Event) {
        if let Some(sink) = self.sink.get() {
            sink(event);
        }
    }

    /// Read one connection until it ends, then make another and read that,
    /// for as long as the client lives. Starts from `first`, or by making a
    /// connection when there is none yet.
    fn run(self: Arc<Self>, mut first: Option<(BufReader<UnixStream>, u64)>) {
        loop {
            if let Some((mut reader, generation)) = first.take() {
                if !self.read(&mut reader, generation) {
                    return;
                }
                {
                    let mut link = self.link.lock().unwrap();
                    match &*link {
                        Link::Up { generation: g, peer } if *g == generation => {
                            peer.kill();
                            *link = Link::Down;
                        }
                        // Closed, superseded, or already replaced.
                        _ => return,
                    }
                }
                // Whatever was asked on the lost connection won't be answered.
                self.pending.lock().unwrap().clear();
            }
            match self.reconnect() {
                Some(next) => first = Some(next),
                None => return,
            }
        }
    }

    /// Read frames until the connection ends. False when it ended because
    /// another app took the terminals over, and nothing is to be made again.
    fn read(&self, reader: &mut BufReader<UnixStream>, generation: u64) -> bool {
        while let Ok((message, body)) = read_frame::<Down>(reader) {
            match message {
                Down::Terminal { mut survivor } => {
                    survivor.backlog = body;
                    self.emit(Event::Adopt(survivor));
                }
                Down::Answer { req, reply } => {
                    if let Some(tx) = self.pending.lock().unwrap().remove(&req) {
                        let _ = tx.send(reply);
                    }
                }
                Down::Output { id, offset } => self.emit(Event::Output { id, offset, data: body }),
                Down::Entry { id, entry } => self.emit(Event::Entry { id, entry }),
                Down::Exit { id, code } => self.emit(Event::Exit { id, code }),
                Down::Superseded => {
                    let mut link = self.link.lock().unwrap();
                    if matches!(&*link, Link::Up { generation: g, .. } if *g == generation) {
                        *link = Link::Superseded;
                        drop(link);
                        self.pending.lock().unwrap().clear();
                        self.emit(Event::Superseded);
                        return false;
                    }
                    // An old connection's: this client has moved on already.
                    return true;
                }
                Down::Welcome { .. } => {}
            }
        }
        true
    }

    /// Connect again, for as long as the link is down, and catch the app up
    /// with what the relay has. `None` once the client is closed or
    /// superseded meanwhile.
    fn reconnect(&self) -> Option<(BufReader<UnixStream>, u64)> {
        let mut delay = Duration::from_millis(50);
        let mut told_gone = false;
        loop {
            if !matches!(*self.link.lock().unwrap(), Link::Down) {
                return None;
            }
            let made = (self.dial)().and_then(|handshake| {
                let Handshake { stream, reader, snapshot } = handshake;
                Ok((Peer::start(stream, REPLY)?, reader, snapshot))
            });
            match made {
                Ok((peer, reader, snapshot)) => {
                    let generation = {
                        let mut link = self.link.lock().unwrap();
                        if !matches!(*link, Link::Down) {
                            peer.close();
                            return None;
                        }
                        let generation = self.generations.fetch_add(1, Ordering::SeqCst) + 1;
                        *link = Link::Up { generation, peer };
                        generation
                    };
                    self.emit(Event::Resync(snapshot));
                    return Some((reader, generation));
                }
                Err(_) => {
                    // A relay that isn't running took its shells with it.
                    if !told_gone && !(self.alive)() {
                        told_gone = true;
                        self.emit(Event::Resync(Snapshot::default()));
                    }
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(RECONNECT_MAX);
                }
            }
        }
    }
}

impl Host for Client {
    fn open(
        &self,
        id: &str,
        path: &str,
        spawn: &Spawn,
        size: PtySize,
        state: serde_json::Value,
    ) -> Result<Opened, String> {
        let req = self.shared.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.shared.pending.lock().unwrap().insert(req, tx);
        let sent = self.send(
            &Up::Open {
                req,
                id: id.to_string(),
                path: path.to_string(),
                spawn: spawn.clone(),
                cols: size.cols,
                rows: size.rows,
                state,
            },
            &[],
        );
        if let Err(e) = sent {
            self.shared.pending.lock().unwrap().remove(&req);
            return Err(e);
        }
        match rx.recv_timeout(REPLY) {
            Ok(Reply::Opened { opened }) => Ok(opened),
            Ok(Reply::Failed { error }) => Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.shared.pending.lock().unwrap().remove(&req);
                Err("the terminal relay did not answer".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("lost the connection to the terminal relay".into())
            }
        }
    }

    fn write(&self, id: &str, data: &[u8], typed_at: Option<u64>) -> Result<(), String> {
        self.send(&Up::Write { id: id.to_string(), typed_at }, data)
    }

    fn resize(&self, id: &str, size: PtySize) -> Result<(), String> {
        self.send(
            &Up::Resize {
                id: id.to_string(),
                cols: size.cols,
                rows: size.rows,
            },
            &[],
        )
    }

    fn hang_up(&self, id: &str) {
        let _ = self.send(&Up::Hangup { id: id.to_string() }, &[]);
    }

    fn note(&self, id: &str, at: u64, value: serde_json::Value) -> Result<(), String> {
        self.send(&Up::Note { id: id.to_string(), at, value }, &[])
    }

    fn checkpoint(&self, id: &str, seq: u64, state: serde_json::Value) {
        let _ = self.send(&Up::Checkpoint { id: id.to_string(), seq, state }, &[]);
    }

    fn set_sink(&self, sink: Sink) {
        if self.shared.sink.set(sink).is_err() {
            return;
        }
        let first = self.shared.first.lock().unwrap().take();
        let first = first.and_then(|reader| match &*self.shared.link.lock().unwrap() {
            Link::Up { generation, .. } => Some((reader, *generation)),
            _ => None,
        });
        let shared = Arc::clone(&self.shared);
        std::thread::spawn(move || shared.run(first));
    }

    fn flush(&self) {
        let peer = match &*self.shared.link.lock().unwrap() {
            Link::Up { peer, .. } => Arc::clone(peer),
            _ => return,
        };
        peer.flush(Duration::from_secs(2));
    }

    fn persistent(&self) -> bool {
        true
    }
}

impl Drop for Client {
    /// Let the relay go on without us, once what is queued has been sent.
    fn drop(&mut self) {
        let mut link = self.shared.link.lock().unwrap();
        if let Link::Up { peer, .. } = std::mem::replace(&mut *link, Link::Closed) {
            peer.close();
        }
    }
}

// ------------------------------------------------------------ attaching

/// What the app found when it looked for its relay.
pub enum Attach {
    Ready(Arc<Client>, Snapshot),
    /// A relay of another version holds this build's terminals.
    Outdated(Meta),
    Failed(String),
}

/// Start the relay process for `build`, in its own session so nothing aimed
/// at the app's process group (a ^C to `pnpm tauri dev`) reaches it. The
/// one long-lived child without a timeout (invariant 1), on purpose.
fn spawn_relay(dir: &Path, paths: &Paths, build: &str) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    std::fs::create_dir_all(dir)?;
    let log = File::options().create(true).append(true).open(&paths.log)?;
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--relay")
        .arg(dir)
        .arg(build)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    // Reaped here, or it lingers as a zombie while the app runs.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Try `attempt` until it works or `within` has passed.
fn retry(within: Duration, attempt: impl Fn() -> io::Result<Handshake>) -> io::Result<Handshake> {
    let deadline = Instant::now() + within;
    loop {
        match attempt() {
            Ok(found) => return Ok(found),
            Err(e) if Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(30)),
        }
    }
}

/// Connect to the build's relay, starting one if none is running, and wait
/// for it to answer. One that is running but not answering is given a
/// moment, never a second relay beside it.
fn dial(dir: &Path, paths: &Paths, build: &str) -> io::Result<Handshake> {
    match handshake(&paths.socket, false) {
        Ok(found) => return Ok(found),
        Err(e) if e.kind() == io::ErrorKind::Unsupported => return Err(e),
        Err(_) => {}
    }
    if held(&paths.lock) {
        return retry(HANDSHAKE, || handshake(&paths.socket, false));
    }
    spawn_relay(dir, paths, build)?;
    retry(Duration::from_secs(5), || handshake(&paths.socket, false))
}

/// Find this build's relay in `dir` (the app's config directory's
/// `relays`), or start one.
pub fn attach(dir: &Path) -> Attach {
    let build = build_label();
    let paths = Paths::new(dir, &build);
    if let Some(meta) = read_meta(&paths.meta) {
        if meta.version != VERSION && !clear_if_gone(dir, &meta) {
            return Attach::Outdated(meta);
        }
    }
    let make_dial = || -> Dial {
        let (dir, paths, build) = (dir.to_path_buf(), paths.clone(), build.clone());
        Box::new(move || dial(&dir, &paths, &build))
    };
    let make_alive = || -> Alive {
        let lock = paths.lock.clone();
        Box::new(move || held(&lock))
    };
    match make_dial()().and_then(|found| Client::new(found, make_dial(), make_alive())) {
        Ok((client, snapshot)) => Attach::Ready(client, snapshot),
        // Running, but not answering yet: its shells are there, and a second
        // one beside them in this process is the one thing not to start.
        Err(_) if held(&paths.lock) => {
            Attach::Ready(Client::waiting(make_dial(), make_alive()), Snapshot::default())
        }
        Err(e) => Attach::Failed(e.to_string()),
    }
}

/// End a relay and every terminal in it, whatever its version. It is asked
/// first; a signal goes only to a process that is certainly a relay.
pub fn terminate(dir: &Path, meta: &Meta) {
    let wait_gone = |within: Duration| {
        let deadline = Instant::now() + within;
        while running(dir, meta) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(30));
        }
    };
    // Version 1 has no `Shutdown`, only the signal.
    if meta.version >= 2 {
        if let Ok(mut stream) = UnixStream::connect(&meta.socket) {
            let _ = stream.set_write_timeout(Some(HANDSHAKE));
            let _ = write_frame(&mut stream, &Up::Shutdown, &[]);
        }
        wait_gone(SHUTDOWN_GRACE + Duration::from_secs(1));
    }
    if running(dir, meta) && is_relay_process(meta.pid) {
        unsafe {
            libc::kill(meta.pid as i32, libc::SIGTERM);
        }
        wait_gone(Duration::from_secs(3));
    }
    clear_if_gone(dir, meta);
}

/// What another build's relay holds, as far as can be told.
pub enum Found {
    Terminals(Snapshot),
    /// It speaks another version, so it can only be ended whole.
    OtherVersion,
    /// It is running but didn't answer.
    NotAnswering,
}

/// Another build's relay, for the Terminals dialog.
pub struct Other {
    pub meta: Meta,
    pub found: Found,
}

/// The relays of every other build that are still running. What a relay
/// that has gone left behind is removed on the way.
pub fn others(dir: &Path) -> Vec<Other> {
    let own = Paths::new(dir, &build_label()).meta;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json") && *p != own)
        .filter_map(|p| read_meta(&p))
        .filter(|meta| running(dir, meta) || !clear_if_gone(dir, meta))
        .map(|meta| {
            let found = if meta.version != VERSION {
                Found::OtherVersion
            } else {
                match handshake(&meta.socket, true) {
                    Ok(found) => Found::Terminals(found.snapshot),
                    Err(_) => Found::NotAnswering,
                }
            };
            Other { meta, found }
        })
        .collect()
}

/// Hang up on some of another build's terminals, by id.
pub fn hang_up_in(meta: &Meta, ids: &[String]) -> io::Result<()> {
    let Handshake { mut stream, .. } = handshake(&meta.socket, true)?;
    for id in ids {
        write_frame(&mut stream, &Up::Hangup { id: id.clone() }, &[])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::tests::{home, read_until, sh, sink, temp_dir, SIZE};
    use crate::terminal::{Chunk, Terminals};

    fn start(name: &str) -> (Paths, std::thread::JoinHandle<io::Result<()>>) {
        let dir = temp_dir(&format!("relay-{name}"));
        let build = format!("test-{name}-{}", std::process::id());
        let paths = Paths::new(&dir, &build);
        let server = {
            let paths = paths.clone();
            std::thread::spawn(move || serve(&paths, &build))
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while UnixStream::connect(&paths.socket).is_err() {
            assert!(Instant::now() < deadline, "the relay never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
        (paths, server)
    }

    fn attach_to(paths: &Paths) -> Terminals {
        let (client, snapshot) = Client::connect(&paths.socket).unwrap();
        Terminals::with_host(None, client, snapshot)
    }

    #[test]
    fn a_shell_outlives_its_app_and_the_next_one_takes_it_back() {
        let (paths, server) = start("outlive");
        let dir = temp_dir("relay-outlive-ws");
        let path = dir.to_string_lossy().to_string();

        let first = attach_to(&paths);
        let (tx, rx) = sink();
        assert!(first.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);
        first.write(&path, b"echo first-$((40+2))\n").unwrap();
        read_until(&rx, "first-42");
        // A program in the foreground whose turn ends once the app has gone.
        first
            .write(&path, b"sh -c \"sleep 0.6; printf '\\033]7501;state=done:app=t\\007'; echo later-\\$((1+1)); sleep 30\"\n")
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        drop(first);
        std::thread::sleep(Duration::from_millis(1200));

        let (client, snapshot) = Client::connect(&paths.socket).unwrap();
        assert_eq!(snapshot.terminals.len(), 1, "the shell is still running");
        let backlog = String::from_utf8_lossy(&snapshot.terminals[0].backlog).to_string();
        assert!(backlog.contains("first-42") && backlog.contains("later-2"), "{backlog:?}");
        let second = Terminals::with_host(None, client, snapshot);
        let info = &second.list()[0];
        assert_eq!(info.home.branch, "feature", "where it belongs came back with it");
        assert!(info.ready, "the turn that ended meanwhile waits for a reply");

        let (tx, rx) = sink();
        assert!(!second.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);
        read_until(&rx, "later-2");
        second.write(&path, b"\x03").unwrap();
        second.write(&path, b"exit\n").unwrap();
        read_until(&rx, "<exit");
        drop(second);
        server.join().unwrap().unwrap();
        assert!(!paths.meta.exists(), "an idle relay cleans up after itself");
        // A shell being forked elsewhere in this process holds a copy of the
        // lock's descriptor until it execs.
        let deadline = Instant::now() + Duration::from_secs(1);
        while held(&paths.lock) {
            assert!(Instant::now() < deadline, "and lets go of its lock");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Quitting and keeping the terminals remembers the one on screen, and
    /// the next app opens on it, once.
    #[test]
    fn the_next_app_opens_on_the_terminal_that_was_on_screen() {
        let (paths, server) = start("view");
        let dirs = [temp_dir("relay-view-a"), temp_dir("relay-view-b")];
        let paths_of: Vec<String> = dirs.iter().map(|d| d.to_string_lossy().to_string()).collect();

        let first = attach_to(&paths);
        for (dir, path) in dirs.iter().zip(&paths_of) {
            let (tx, _rx) = sink();
            assert!(first.open(home(), path, SIZE, sh(dir), tx).unwrap().started);
        }
        first.view(Some(&paths_of[1]));
        first.keep_view();
        // A status arriving while the app quits doesn't save over it.
        first.mark(&paths_of[1], crate::terminal::Mark::Unread).unwrap();
        first.flush();
        drop(first);

        let second = attach_to(&paths);
        assert_eq!(second.list().len(), 2);
        let view = second.take_restored_view().expect("the terminal on screen");
        assert_eq!(view.path, paths_of[1]);
        assert!(second.take_restored_view().is_none(), "asked once");

        second.close_all();
        second.flush();
        drop(second);
        server.join().unwrap().unwrap();
    }

    /// Typing all afternoon with no status change still counts as activity
    /// on the next launch, and a turn that ended as the app was quitting is
    /// still waiting there. The app had saved neither.
    #[test]
    fn what_the_app_had_not_saved_comes_back_from_the_log() {
        let (paths, server) = start("log");
        let dir = temp_dir("relay-log-ws");
        let path = dir.to_string_lossy().to_string();

        let first = attach_to(&paths);
        let (tx, rx) = sink();
        first.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        std::thread::sleep(Duration::from_millis(5));
        let typed = first.write(&path, b"echo typed-$((1+1))\n").unwrap();
        read_until(&rx, "typed-2");
        drop(first);

        let second = attach_to(&paths);
        assert_eq!(second.list()[0].active_at, typed, "typing is remembered");
        // A turn that ends just as the app quits: it never sees it.
        second
            .write_untyped(&path, b"sh -c \"sleep 0.3; printf '\\033]7501;state=done:app=t\\007'; sleep 30\"\n")
            .unwrap();
        drop(second);
        std::thread::sleep(Duration::from_millis(800));

        let third = attach_to(&paths);
        assert!(third.list()[0].ready, "the turn that ended as the app quit waits for a reply");
        third.close_all();
        third.flush();
        drop(third);
        server.join().unwrap().unwrap();
    }

    /// The connection drops while the shells run on: the app reconnects,
    /// keeps every terminal, and gets the output it missed exactly once.
    #[test]
    fn a_lost_connection_loses_no_terminal_and_no_output() {
        let (paths, server) = start("reconnect");
        let dir = temp_dir("relay-reconnect-ws");
        let path = dir.to_string_lossy().to_string();
        let (client, snapshot) = Client::connect(&paths.socket).unwrap();
        let terminals = Terminals::with_host(None, Arc::clone(&client) as Arc<dyn Host>, snapshot);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo before\n").unwrap();
        read_until(&rx, "before");

        // Output while the connection is down, then the connection drops.
        terminals
            .write(&path, b"sleep 0.3; echo during-$((2+2))\n")
            .unwrap();
        terminals.flush();
        client.kill_connection();
        let seen = read_until(&rx, "during-4");
        assert_eq!(seen.matches("during-4").count(), 1, "once: {seen:?}");
        assert!(!seen.contains("<exit"), "{seen:?}");
        assert_eq!(terminals.list().len(), 1, "still there");

        terminals.write(&path, b"echo after\n").unwrap();
        read_until(&rx, "after");
        terminals.close_all();
        read_until(&rx, "<exit");
        drop(terminals);
        drop(client);
        server.join().unwrap().unwrap();
    }

    /// A client whose connections fail until `until`, as a relay that is
    /// slow to answer, or one the app can't reach for a while.
    fn delayed(paths: &Paths, until: Instant) -> Dial {
        let socket = paths.socket.clone();
        Box::new(move || {
            if Instant::now() < until {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "not yet"));
            }
            handshake(&socket, false)
        })
    }

    /// A shell that prints its last words and exits while the app can't
    /// reach the relay: the app still gets the words, then the exit.
    #[test]
    fn a_shell_that_exits_while_the_app_is_away_still_says_why() {
        let (paths, server) = start("away");
        let dir = temp_dir("relay-away-ws");
        let path = dir.to_string_lossy().to_string();
        // Back within the time a relay with nothing left waits.
        let until = Instant::now() + LINGER / 2;
        let (client, snapshot) =
            Client::new(handshake(&paths.socket, false).unwrap(), delayed(&paths, until), Box::new(|| true))
                .unwrap();
        let terminals = Terminals::with_host(None, Arc::clone(&client) as Arc<dyn Host>, snapshot);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals
            .write(&path, b"sleep 0.4; echo last-$((6+1)); exit 3\n")
            .unwrap();
        terminals.flush();
        client.kill_connection();
        let seen = read_until(&rx, "<exit 3>");
        assert!(seen.contains("last-7"), "{seen:?}");
        assert!(Instant::now() >= until, "seen after the reconnect");
        drop(terminals);
        drop(client);
        server.join().unwrap().unwrap();
    }

    /// The app found its relay running but not answering: it doesn't start
    /// shells of its own beside the relay's, and has them once it answers.
    #[test]
    fn a_relay_that_answers_late_still_hands_its_terminals_over() {
        let (paths, server) = start("late");
        let dir = temp_dir("relay-late-ws");
        let path = dir.to_string_lossy().to_string();
        let first = attach_to(&paths);
        let (tx, _rx) = sink();
        first.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        drop(first);

        let until = Instant::now() + Duration::from_millis(500);
        let client = Client::waiting(delayed(&paths, until), Box::new(|| true));
        let terminals = Terminals::with_host(None, client, Snapshot::default());
        let (tx, _) = sink();
        assert!(terminals.open(home(), &path, SIZE, sh(&dir), tx).is_err(), "no second shell");
        let deadline = Instant::now() + Duration::from_secs(5);
        while terminals.list().is_empty() {
            assert!(Instant::now() < deadline, "the relay's terminal never arrived");
            std::thread::sleep(Duration::from_millis(50));
        }
        let (tx, rx) = sink();
        assert!(!terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);
        terminals.write(&path, b"echo found-$((4+4))\n").unwrap();
        read_until(&rx, "found-8");
        terminals.close_all();
        read_until(&rx, "<exit");
        drop(terminals);
        server.join().unwrap().unwrap();
    }

    /// A second app of the same build takes the terminals over; the first is
    /// told so, and doesn't take them back.
    #[test]
    fn the_newest_app_takes_the_terminals_and_the_old_one_lets_go() {
        let (paths, server) = start("supersede");
        let dir = temp_dir("relay-supersede-ws");
        let path = dir.to_string_lossy().to_string();
        let old = attach_to(&paths);
        let (tx, old_rx) = sink();
        old.open(home(), &path, SIZE, sh(&dir), tx).unwrap();

        let new = attach_to(&paths);
        assert_eq!(new.list().len(), 1);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match old_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Chunk::Moved) => break,
                _ => assert!(Instant::now() < deadline, "the old app was never told"),
            }
        }
        assert!(old.list().is_empty());
        let (tx, _) = sink();
        assert!(old.open(home(), &path, SIZE, sh(&dir), tx).is_err(), "and can't open one");

        let (tx, rx) = sink();
        assert!(!new.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);
        new.write(&path, b"echo still-$((3+3))\n").unwrap();
        read_until(&rx, "still-6");
        new.close_all();
        read_until(&rx, "<exit");
        drop(old);
        drop(new);
        server.join().unwrap().unwrap();
    }

    /// A program that never reads its input holds up only its own terminal.
    #[test]
    fn a_terminal_that_isnt_reading_holds_up_no_other() {
        let (paths, server) = start("stuck");
        let stuck_dir = temp_dir("relay-stuck-a");
        let free_dir = temp_dir("relay-stuck-b");
        let (stuck, free) = (stuck_dir.to_string_lossy().to_string(), free_dir.to_string_lossy().to_string());
        let terminals = attach_to(&paths);
        let (tx, stuck_rx) = sink();
        terminals.open(home(), &stuck, SIZE, sh(&stuck_dir), tx).unwrap();
        terminals.write(&stuck, b"stty -icanon; sleep 30\n").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        for _ in 0..64 {
            let _ = terminals.write(&stuck, &[b'x'; 4096]);
        }
        let (tx, free_rx) = sink();
        let started = Instant::now();
        terminals.open(home(), &free, SIZE, sh(&free_dir), tx).unwrap();
        terminals.write(&free, b"echo free-$((5+5))\n").unwrap();
        read_until(&free_rx, "free-10");
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
        terminals.close_all();
        read_until(&stuck_rx, "<exit");
        read_until(&free_rx, "<exit");
        drop(terminals);
        server.join().unwrap().unwrap();
    }

    /// A relay that takes the connection and never answers is given up on,
    /// rather than holding up the app's launch.
    #[test]
    fn a_relay_that_never_answers_is_given_up_on() {
        // Short: a socket's path is limited to 104 bytes.
        let socket = std::env::temp_dir().join(format!("pl-silent-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).unwrap();
        let _held_open = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(10));
            drop(stream);
        });
        let started = Instant::now();
        assert!(handshake(&socket, false).is_err());
        assert!(started.elapsed() < HANDSHAKE + Duration::from_secs(1), "{:?}", started.elapsed());
    }

    /// One terminal per workspace: opening one that has a terminal, under
    /// any id, answers with that one.
    #[test]
    fn a_workspace_never_gets_a_second_shell() {
        let (paths, server) = start("unique");
        let dir = temp_dir("relay-unique-ws");
        let path = dir.to_string_lossy().to_string();
        let first = attach_to(&paths);
        let (tx, _rx) = sink();
        assert!(first.open(home(), &path, SIZE, sh(&dir), tx).unwrap().started);

        // An app that has lost track of it, as after a timed-out open.
        let (client, snapshot) = Client::connect(&paths.socket).unwrap();
        drop(snapshot);
        client.set_sink(Arc::new(|_| {}));
        let opened = client
            .open("stray", &path, &Spawn::from_command(&sh(&dir)), SIZE, serde_json::json!({}))
            .unwrap();
        assert!(matches!(opened, Opened::Existing { .. }), "{opened:?}");
        let Handshake { snapshot, .. } = handshake(&paths.socket, true).unwrap();
        assert_eq!(snapshot.terminals.len(), 1);

        let again = attach_to(&paths);
        again.close_all();
        again.flush();
        drop((first, client, again));
        server.join().unwrap().unwrap();
    }

    /// A relay is running while it holds its lock, and only then: a pid left
    /// in its metadata says nothing once it has gone.
    #[test]
    fn a_relay_is_known_by_its_lock_and_ends_when_asked() {
        let (paths, server) = start("lock");
        let meta = read_meta(&paths.meta).unwrap();
        assert_eq!(meta.version, VERSION);
        assert!(held(&paths.lock));
        let dir = paths.meta.parent().unwrap();
        assert!(running(dir, &meta));

        let shell_dir = temp_dir("relay-lock-ws");
        let path = shell_dir.to_string_lossy().to_string();
        let terminals = attach_to(&paths);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&shell_dir), tx).unwrap();

        // Asked over its socket, not signalled: it is a thread here.
        let mut stream = UnixStream::connect(&meta.socket).unwrap();
        write_frame(&mut stream, &Up::Shutdown, &[]).unwrap();
        read_until(&rx, "<exit");
        server.join().unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while held(&paths.lock) {
            assert!(Instant::now() < deadline, "the lock went with it");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!running(dir, &meta), "its pid is still this process, which is no relay");

        // Stale metadata naming a live, unrelated pid: not running.
        let stale = Meta { pid: std::process::id(), ..meta };
        write_meta(&paths.meta, &stale).unwrap();
        assert!(!running(dir, &stale));
        assert!(others(dir).is_empty(), "nothing is listed");
        assert!(!paths.meta.exists(), "and what it left is cleared away");
        drop(terminals);
    }

    /// An observer sees another build's terminals as they are now, with what
    /// their programs reported since their app last saved.
    #[test]
    fn another_build_is_seen_as_it_is_now() {
        let (paths, server) = start("observe");
        let dir = temp_dir("relay-observe-ws");
        let path = dir.to_string_lossy().to_string();
        let app = attach_to(&paths);
        let (tx, _rx) = sink();
        app.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        app.write(
            &path,
            b"sh -c \"printf '\\033]7501;state=working:app=t\\007'; sleep 0.6; printf '\\033]7501;state=done:app=t\\007'; sleep 30\"\n",
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while app.list()[0].status.as_deref() != Some("working") {
            assert!(Instant::now() < deadline, "never working");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Its app goes, saying it works, and then the turn ends.
        app.keep_view();
        drop(app);
        std::thread::sleep(Duration::from_millis(1000));

        let Handshake { snapshot, .. } = handshake(&paths.socket, true).unwrap();
        assert!(snapshot.terminals[0].backlog.is_empty(), "an observer gets no backlog");
        let info = crate::terminal::describe(&snapshot.terminals[0]);
        assert_eq!(info.status.as_deref(), Some("done"));
        assert!(info.ready);

        let app = attach_to(&paths);
        app.close_all();
        app.flush();
        drop(app);
        server.join().unwrap().unwrap();
    }
}
