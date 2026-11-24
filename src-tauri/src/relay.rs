//! The relay: a small process that holds the ptys, so shells outlive the
//! app. Quitting, a crash, or installing a new build with `run.sh --build`
//! used to hang up on every shell, since the app held each pty's master side
//! and the kernel hangs up when that closes.
//!
//! The relay is deliberately simple: it is a `pty::Local` behind a Unix
//! socket. It starts shells, keeps each one's backlog, answers the OSC 7501
//! probe and passes bytes through. What the statuses mean (ready, waiting,
//! reaping a removed workspace's terminal) stays in `terminal.rs`, in the
//! app, so changing that needs no new relay. The app keeps an opaque value
//! with each pty (`Kept` in `terminal.rs`), and while no app is attached the
//! relay logs the status payloads that arrive, so the next app can work out
//! what happened meanwhile.
//!
//! One relay per build: the build's branch, and whether it is a dev build
//! (`build_label`). Reinstalling a build gets its terminals back; a worktree's
//! build has its own, as before, and two windows never share a shell. The
//! relay is the app's own binary started with `--relay`, in its own session,
//! and exits once it has no terminals and no app attached.
//!
//! A relay of another protocol version is not talked to: the app offers to
//! end its terminals and start over (`Attach::Outdated`). Its metadata file
//! says which version it is, which needs no protocol to read.
//!
//! Wire format: frames of a big-endian u32 header length, a u32 body length,
//! a JSON header, then the body as raw bytes (terminal output and input).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::PtySize;

use crate::pty::{self, Event, Host, Sink, Spawn, Survivor};
use crate::tasks::now_ms;

/// Bumped whenever a message changes shape. An app only talks to a relay of
/// its own version.
pub const VERSION: u32 = 1;

/// How long a relay waits for its first app before giving up.
const FIRST_CLIENT: Duration = Duration::from_secs(15);
/// How long a request may wait for its reply.
const REPLY: Duration = Duration::from_secs(10);
/// A relay stops writing to an app that hasn't read for this long, rather
/// than stall every shell behind it.
const STALLED: Duration = Duration::from_secs(10);
/// How many status payloads are kept per terminal while no app is attached.
const DETACHED_STATUSES: usize = 256;

// ------------------------------------------------------------------ wire

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Up {
    /// `observe`: only list the terminals, and take hang-ups; the app that
    /// owns the relay stays attached.
    Hello { version: u32, observe: bool },
    Open { req: u64, id: String, path: String, spawn: Spawn, cols: u16, rows: u16 },
    /// The body is the input.
    Write { id: String },
    Resize { id: String, cols: u16, rows: u16 },
    Hangup { id: String },
    SetState { id: String, state: serde_json::Value },
    Leader { req: u64, id: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Down {
    /// One per running terminal, before `Welcome`. The body is its backlog
    /// (empty to an observer).
    Survivor {
        id: String,
        path: String,
        shell_pid: Option<i32>,
        leader: Option<i32>,
        state: Option<serde_json::Value>,
        statuses: Vec<(u64, String)>,
    },
    Welcome { version: u32, pid: u32 },
    Opened { req: u64, pid: Option<i32>, error: Option<String> },
    Leader { req: u64, pid: Option<i32> },
    /// The body is the output.
    Output { id: String, statuses: Vec<String>, leader: Option<i32> },
    Exit { id: String, code: i32 },
}

const MAX_HEADER: usize = 16 << 20;
const MAX_BODY: usize = 64 << 20;

fn write_frame(w: &mut impl Write, head: &impl Serialize, body: &[u8]) -> io::Result<()> {
    let head = serde_json::to_vec(head)?;
    let mut buf = Vec::with_capacity(8 + head.len() + body.len());
    buf.extend_from_slice(&(head.len() as u32).to_be_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_be_bytes());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
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

/// Where one build's relay keeps its socket, metadata and log.
#[derive(Debug, Clone)]
pub struct Paths {
    pub socket: PathBuf,
    pub meta: PathBuf,
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

fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

// ---------------------------------------------------------------- server

type Writer = Arc<Mutex<UnixStream>>;

struct Relay {
    local: pty::Local,
    /// The app attached, if any. Output goes here, under the gate.
    client: Mutex<Option<Writer>>,
    states: Mutex<HashMap<String, serde_json::Value>>,
    /// Status payloads that arrived while no app was attached.
    detached: Mutex<HashMap<String, Vec<(u64, String)>>>,
    connections: AtomicUsize,
    had_client: AtomicBool,
}

impl Relay {
    fn on_event(&self, event: Event) {
        let mut client = self.client.lock().unwrap();
        if let Event::Exit { id, .. } = &event {
            self.states.lock().unwrap().remove(id);
            self.detached.lock().unwrap().remove(id);
        }
        if let Some(writer) = client.as_ref() {
            let sent = match event {
                Event::Output { id, data, statuses, leader } => write_frame(
                    &mut *writer.lock().unwrap(),
                    &Down::Output { id, statuses, leader },
                    &data,
                ),
                Event::Exit { id, code } => {
                    write_frame(&mut *writer.lock().unwrap(), &Down::Exit { id, code }, &[])
                }
            };
            if sent.is_err() {
                // Gone, or stalled: the next app gets the backlog instead.
                if let Ok(stream) = writer.lock() {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                }
                *client = None;
            }
        } else if let Event::Output { id, statuses, .. } = event {
            if !statuses.is_empty() {
                let mut detached = self.detached.lock().unwrap();
                let log = detached.entry(id).or_default();
                let at = now_ms();
                log.extend(statuses.into_iter().map(|p| (at, p)));
                if log.len() > DETACHED_STATUSES {
                    let excess = log.len() - DETACHED_STATUSES;
                    log.drain(..excess);
                }
            }
        }
    }

    /// Every terminal, as the app wants them on attaching. An observer gets
    /// neither backlogs nor the statuses kept for the next app.
    fn survivors(&self, observe: bool) -> Vec<Down> {
        let states = self.states.lock().unwrap();
        let mut detached = self.detached.lock().unwrap();
        self.local
            .snapshot()
            .into_iter()
            .map(|s| Down::Survivor {
                state: states.get(&s.id).cloned(),
                statuses: if observe {
                    detached.get(&s.id).cloned().unwrap_or_default()
                } else {
                    detached.remove(&s.id).unwrap_or_default()
                },
                id: s.id,
                path: s.path,
                shell_pid: s.shell_pid,
                leader: s.leader,
            })
            .collect()
    }

    fn serve_connection(self: &Arc<Self>, stream: UnixStream) -> io::Result<()> {
        stream.set_write_timeout(Some(STALLED))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let writer: Writer = Arc::new(Mutex::new(stream));
        let (hello, _) = read_frame::<Up>(&mut reader)?;
        let Up::Hello { version, observe } = hello else {
            return Ok(());
        };
        let welcome = Down::Welcome {
            version: VERSION,
            pid: std::process::id(),
        };
        if version != VERSION {
            // It can read our version from the reply, and goes.
            return write_frame(&mut *writer.lock().unwrap(), &welcome, &[]);
        }

        {
            // Under the gate, so no output lands in a backlog without also
            // reaching the app, and none reaches it twice.
            let gate = self.local.gate();
            let _gate = gate.lock().unwrap();
            let mut backlogs: HashMap<String, Vec<u8>> = if observe {
                HashMap::new()
            } else {
                self.local.snapshot().into_iter().map(|s| (s.id, s.backlog)).collect()
            };
            let mut w = writer.lock().unwrap();
            for survivor in self.survivors(observe) {
                let body = match &survivor {
                    Down::Survivor { id, .. } => backlogs.remove(id).unwrap_or_default(),
                    _ => Vec::new(),
                };
                write_frame(&mut *w, &survivor, &body)?;
            }
            write_frame(&mut *w, &welcome, &[])?;
            drop(w);
            if !observe {
                let mut client = self.client.lock().unwrap();
                // Newest wins: the one before is a window that is gone, or
                // about to be.
                if let Some(old) = client.replace(Arc::clone(&writer)) {
                    let _ = old.lock().unwrap().shutdown(std::net::Shutdown::Both);
                }
                self.had_client.store(true, Ordering::SeqCst);
            }
        }

        let result = self.requests(&mut reader, &writer, observe);
        let mut client = self.client.lock().unwrap();
        if client.as_ref().is_some_and(|c| Arc::ptr_eq(c, &writer)) {
            *client = None;
        }
        result
    }

    fn requests(&self, reader: &mut impl Read, writer: &Writer, observe: bool) -> io::Result<()> {
        loop {
            let (message, body) = read_frame::<Up>(reader)?;
            match message {
                Up::Hangup { id } => self.local.hang_up(&id),
                _ if observe => {}
                Up::Hello { .. } => {}
                Up::Open { req, id, path, spawn, cols, rows } => {
                    let (pid, error) = match self.local.open(&id, &path, &spawn, size(cols, rows)) {
                        Ok(pid) => (pid, None),
                        Err(e) => (None, Some(e)),
                    };
                    write_frame(&mut *writer.lock().unwrap(), &Down::Opened { req, pid, error }, &[])?;
                }
                Up::Write { id } => {
                    let _ = self.local.write(&id, &body);
                }
                Up::Resize { id, cols, rows } => {
                    let _ = self.local.resize(&id, size(cols, rows));
                }
                Up::SetState { id, state } => {
                    self.states.lock().unwrap().insert(id, state);
                }
                Up::Leader { req, id } => {
                    let pid = self.local.leader(&id);
                    write_frame(&mut *writer.lock().unwrap(), &Down::Leader { req, pid }, &[])?;
                }
            }
        }
    }

    /// Nothing to keep running for: no terminal, no app, and either an app
    /// has been and gone or none came in time.
    fn idle(&self, started: Instant) -> bool {
        self.local.is_empty()
            && self.connections.load(Ordering::SeqCst) == 0
            && (self.had_client.load(Ordering::SeqCst) || started.elapsed() > FIRST_CLIENT)
    }
}

/// Run a relay on `socket` until it has nothing left to do. Returns at once
/// if another relay already answers there.
pub fn serve(paths: &Paths, build: &str) -> io::Result<()> {
    if UnixStream::connect(&paths.socket).is_ok() {
        return Ok(());
    }
    if let Some(dir) = paths.socket.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&paths.socket);
    let listener = UnixListener::bind(&paths.socket)?;
    listener.set_nonblocking(true)?;
    let meta = Meta {
        version: VERSION,
        pid: std::process::id(),
        build: build.to_string(),
        socket: paths.socket.clone(),
    };
    std::fs::write(&paths.meta, serde_json::to_vec(&meta)?)?;

    let relay = Arc::new(Relay {
        local: pty::Local::new(true),
        client: Mutex::default(),
        states: Mutex::default(),
        detached: Mutex::default(),
        connections: AtomicUsize::new(0),
        had_client: AtomicBool::new(false),
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
                    relay.connections.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if relay.idle(started) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
    // Only ours: a newer relay may have taken the name over.
    if read_meta(&paths.meta).is_some_and(|m| m.pid == std::process::id()) {
        let _ = std::fs::remove_file(&paths.meta);
        let _ = std::fs::remove_file(&paths.socket);
    }
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

/// Connects again, starting a relay first if need be.
type Redial = Box<dyn Fn() -> io::Result<(UnixStream, BufReader<UnixStream>)> + Send + Sync>;

struct Conn {
    writer: Mutex<UnixStream>,
    reader: Mutex<Option<BufReader<UnixStream>>>,
    alive: AtomicBool,
}

/// A relay, as a `pty::Host` for `terminal.rs`.
pub struct Client {
    conn: Mutex<Arc<Conn>>,
    /// Connects again, starting a relay first if need be, after the one
    /// attached to has died. None in tests.
    redial: Option<Redial>,
    sink: Mutex<Option<Sink>>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Down>>>>,
    next: AtomicU64,
    /// The ptys open in the relay, so they can be reported gone if it dies.
    ids: Arc<Mutex<HashSet<String>>>,
}

/// Say hello and read the terminals the relay already has, up to its
/// welcome. Errors with the relay's version if it isn't ours.
fn handshake(
    socket: &Path,
    observe: bool,
) -> io::Result<(UnixStream, BufReader<UnixStream>, Vec<Survivor>)> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_write_timeout(Some(REPLY))?;
    write_frame(&mut stream, &Up::Hello { version: VERSION, observe }, &[])?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut survivors = Vec::new();
    loop {
        match read_frame::<Down>(&mut reader)? {
            (Down::Survivor { id, path, shell_pid, leader, state, statuses }, backlog) => {
                survivors.push(Survivor { id, path, shell_pid, leader, backlog, state, statuses })
            }
            (Down::Welcome { version, .. }, _) if version != VERSION => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("relay speaks version {version}"),
                ))
            }
            (Down::Welcome { .. }, _) => return Ok((stream, reader, survivors)),
            _ => {}
        }
    }
}

impl Client {
    /// Attach to the relay at `socket`. Returns the terminals it already had.
    pub fn connect(socket: &Path) -> io::Result<(Arc<Client>, Vec<Survivor>)> {
        let (stream, reader, survivors) = handshake(socket, false)?;
        let client = Client::new(stream, reader, None);
        client.ids.lock().unwrap().extend(survivors.iter().map(|s| s.id.clone()));
        Ok((Arc::new(client), survivors))
    }

    fn new(
        stream: UnixStream,
        reader: BufReader<UnixStream>,
        redial: Option<Redial>,
    ) -> Client {
        Client {
            conn: Mutex::new(Arc::new(Conn {
                writer: Mutex::new(stream),
                reader: Mutex::new(Some(reader)),
                alive: AtomicBool::new(true),
            })),
            redial,
            sink: Mutex::default(),
            pending: Arc::default(),
            next: AtomicU64::new(1),
            ids: Arc::default(),
        }
    }

    /// Read what the relay sends until it goes, then report every terminal
    /// it held as exited: its shells were hung up with it.
    fn start_reading(&self, conn: &Arc<Conn>, sink: Sink) {
        let Some(mut reader) = conn.reader.lock().unwrap().take() else {
            return;
        };
        let conn = Arc::clone(conn);
        let pending = Arc::clone(&self.pending);
        let ids = Arc::clone(&self.ids);
        std::thread::spawn(move || {
            while let Ok((message, body)) = read_frame::<Down>(&mut reader) {
                match message {
                    Down::Opened { req, .. } | Down::Leader { req, .. } => {
                        if let Some(tx) = pending.lock().unwrap().remove(&req) {
                            let _ = tx.send(message);
                        }
                    }
                    Down::Output { id, statuses, leader } => {
                        sink(Event::Output { id, data: body, statuses, leader })
                    }
                    Down::Exit { id, code } => {
                        ids.lock().unwrap().remove(&id);
                        sink(Event::Exit { id, code });
                    }
                    Down::Survivor { .. } | Down::Welcome { .. } => {}
                }
            }
            conn.alive.store(false, Ordering::SeqCst);
            pending.lock().unwrap().clear();
            let gone: Vec<String> = ids.lock().unwrap().drain().collect();
            for id in gone {
                sink(Event::Exit { id, code: -1 });
            }
        });
    }

    /// The connection, dialled again if the relay died and can be restarted.
    fn conn(&self) -> Result<Arc<Conn>, String> {
        let mut conn = self.conn.lock().unwrap();
        if conn.alive.load(Ordering::SeqCst) {
            return Ok(Arc::clone(&conn));
        }
        let redial = self.redial.as_ref().ok_or("the terminal relay is gone")?;
        let (stream, reader) = redial().map_err(|e| format!("could not restart the terminal relay: {e}"))?;
        let fresh = Arc::new(Conn {
            writer: Mutex::new(stream),
            reader: Mutex::new(Some(reader)),
            alive: AtomicBool::new(true),
        });
        if let Some(sink) = self.sink.lock().unwrap().clone() {
            self.start_reading(&fresh, sink);
        }
        *conn = Arc::clone(&fresh);
        Ok(fresh)
    }

    fn send(&self, message: &Up, body: &[u8]) -> Result<(), String> {
        let conn = self.conn()?;
        let mut writer = conn.writer.lock().unwrap();
        write_frame(&mut *writer, message, body).map_err(|e| format!("terminal relay: {e}"))
    }

    fn request(&self, make: impl FnOnce(u64) -> Up) -> Result<Down, String> {
        let req = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(req, tx);
        if let Err(e) = self.send(&make(req), &[]) {
            self.pending.lock().unwrap().remove(&req);
            return Err(e);
        }
        rx.recv_timeout(REPLY).map_err(|_| {
            self.pending.lock().unwrap().remove(&req);
            "the terminal relay did not answer".to_string()
        })
    }
}

impl Host for Client {
    fn open(&self, id: &str, path: &str, spawn: &Spawn, size: PtySize) -> Result<Option<i32>, String> {
        // Known before the relay answers, so an exit that beats the answer,
        // or the relay dying, is reported.
        self.ids.lock().unwrap().insert(id.to_string());
        let reply = self.request(|req| Up::Open {
            req,
            id: id.to_string(),
            path: path.to_string(),
            spawn: spawn.clone(),
            cols: size.cols,
            rows: size.rows,
        });
        match reply {
            Ok(Down::Opened { pid, error: None, .. }) => Ok(pid),
            Ok(Down::Opened { error: Some(e), .. }) | Err(e) => {
                self.ids.lock().unwrap().remove(id);
                Err(e)
            }
            Ok(_) => Err("unexpected reply from the terminal relay".into()),
        }
    }

    fn write(&self, id: &str, data: &[u8]) -> Result<(), String> {
        self.send(&Up::Write { id: id.to_string() }, data)
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

    fn leader(&self, id: &str) -> Option<i32> {
        match self.request(|req| Up::Leader { req, id: id.to_string() }) {
            Ok(Down::Leader { pid, .. }) => pid,
            _ => None,
        }
    }

    fn set_state(&self, id: &str, state: serde_json::Value) {
        let _ = self.send(&Up::SetState { id: id.to_string(), state }, &[]);
    }

    fn set_sink(&self, sink: Sink) {
        *self.sink.lock().unwrap() = Some(Arc::clone(&sink));
        let conn = Arc::clone(&self.conn.lock().unwrap());
        self.start_reading(&conn, sink);
    }

    fn persistent(&self) -> bool {
        true
    }
}

impl Drop for Client {
    /// Let the relay go on without us. The reader thread holds its own copy
    /// of the socket, so dropping ours alone would not close it.
    fn drop(&mut self) {
        if let Ok(conn) = self.conn.lock() {
            if let Ok(writer) = conn.writer.lock() {
                let _ = writer.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

// ------------------------------------------------------------ attaching

/// What the app found when it looked for its relay.
pub enum Attach {
    Ready(Arc<Client>, Vec<Survivor>),
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
    let log = std::fs::File::options().create(true).append(true).open(&paths.log)?;
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

/// Connect to the build's relay, starting one if there is none, and wait
/// for it to answer.
fn dial(dir: &Path, paths: &Paths, build: &str) -> io::Result<(UnixStream, BufReader<UnixStream>, Vec<Survivor>)> {
    if let Ok(found) = handshake(&paths.socket, false) {
        return Ok(found);
    }
    spawn_relay(dir, paths, build)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match handshake(&paths.socket, false) {
            Ok(found) => return Ok(found),
            Err(e) if Instant::now() > deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(30)),
        }
    }
}

/// Find this build's relay in `dir` (the app's config directory's
/// `relays`), or start one.
pub fn attach(dir: &Path) -> Attach {
    let build = build_label();
    let paths = Paths::new(dir, &build);
    if let Some(meta) = read_meta(&paths.meta) {
        if !alive(meta.pid) {
            let _ = std::fs::remove_file(&paths.meta);
            let _ = std::fs::remove_file(&meta.socket);
        } else if meta.version != VERSION {
            return Attach::Outdated(meta);
        }
    }
    match dial(dir, &paths, &build) {
        Ok((stream, reader, survivors)) => {
            let redial_dir = dir.to_path_buf();
            let redial: Redial = Box::new(move || {
                    let paths = Paths::new(&redial_dir, &build);
                    dial(&redial_dir, &paths, &build).map(|(s, r, _)| (s, r))
                });
            let client = Client::new(stream, reader, Some(redial));
            client.ids.lock().unwrap().extend(survivors.iter().map(|s| s.id.clone()));
            Attach::Ready(Arc::new(client), survivors)
        }
        Err(e) => Attach::Failed(e.to_string()),
    }
}

/// End every terminal in a relay, whatever its version: it holds their
/// ptys, and they hang up as it goes.
pub fn terminate(meta: &Meta) {
    if alive(meta.pid) {
        unsafe {
            libc::kill(meta.pid as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(meta.pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

/// Another build's relay, for the Terminals dialog.
pub struct Other {
    pub meta: Meta,
    /// Its terminals, if it speaks our version; `None` if it doesn't.
    pub terminals: Option<Vec<Survivor>>,
}

/// The relays of every other build that are still running.
pub fn others(dir: &Path) -> Vec<Other> {
    let own = Paths::new(dir, &build_label()).meta;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json") && *p != own)
        .filter_map(|p| read_meta(&p))
        .filter(|meta| alive(meta.pid))
        .map(|meta| {
            let terminals = (meta.version == VERSION)
                .then(|| handshake(&meta.socket, true).ok().map(|(_, _, s)| s))
                .flatten();
            Other { meta, terminals }
        })
        .collect()
}

/// Hang up on some of another build's terminals, by id.
pub fn hang_up_in(meta: &Meta, ids: &[String]) -> io::Result<()> {
    let (mut stream, _, _) = handshake(&meta.socket, true)?;
    for id in ids {
        write_frame(&mut stream, &Up::Hangup { id: id.clone() }, &[])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::tests::{home, read_until, sh, sink, temp_dir, SIZE};
    use crate::terminal::Terminals;

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

    #[test]
    fn a_shell_outlives_its_app_and_the_next_one_takes_it_back() {
        let (paths, server) = start("outlive");
        let dir = temp_dir("relay-outlive-ws");
        let path = dir.to_string_lossy().to_string();

        let (client, survivors) = Client::connect(&paths.socket).unwrap();
        assert!(survivors.is_empty());
        let first = Terminals::with_host(None, client, survivors);
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

        let (client, survivors) = Client::connect(&paths.socket).unwrap();
        assert_eq!(survivors.len(), 1, "the shell is still running");
        let backlog = String::from_utf8_lossy(&survivors[0].backlog).to_string();
        assert!(backlog.contains("first-42") && backlog.contains("later-2"), "{backlog:?}");
        let second = Terminals::with_host(None, client, survivors);
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
    }

    #[test]
    fn the_app_sees_its_shells_go_with_the_relay() {
        let (paths, _server) = start("dies");
        let dir = temp_dir("relay-dies-ws");
        let path = dir.to_string_lossy().to_string();
        let (client, survivors) = Client::connect(&paths.socket).unwrap();
        let terminals = Terminals::with_host(None, client, survivors);
        let (tx, rx) = sink();
        terminals.open(home(), &path, SIZE, sh(&dir), tx).unwrap();
        terminals.write(&path, b"echo up\n").unwrap();
        read_until(&rx, "up");
        let meta = read_meta(&paths.meta).unwrap();
        // The relay in this test is a thread, so it can't be killed; take
        // its terminals down the way a dying relay's would go instead.
        let (mut observer, _, found) = handshake(&paths.socket, true).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].backlog.is_empty(), "an observer gets no backlog");
        write_frame(&mut observer, &Up::Hangup { id: found[0].id.clone() }, &[]).unwrap();
        read_until(&rx, "<exit");
        assert!(terminals.list().is_empty());
        assert_eq!(meta.version, VERSION);
    }
}
