//! Pseudo-terminals and the shells in them, and nothing about what their
//! programs report: starting a shell, reading what it prints, answering the
//! OSC 7501 probe, and noticing it exit. `terminal.rs` builds the statuses
//! on top of the events sent from here.
//!
//! A [`Host`] is where the ptys live. [`Local`] keeps them in the process
//! that made it: the relay (`relay.rs`), so shells outlive the app, or the
//! app itself when no relay can be had, and in tests. `relay::Client` is the
//! same interface over the relay's socket.
//!
//! The probe is answered here rather than in `terminal.rs`, so a program
//! started while no app is attached to the relay still gets its answer, and
//! with it reports a status for the rest of its life.

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// How much recent output a terminal keeps for replay on reattach. A few
/// screens of a noisy build; xterm keeps the real scrollback on its side.
pub const BACKLOG: usize = 512 * 1024;

/// How long to wait for the last output after the shell exits. A background
/// job the shell left behind can hold the pty open indefinitely, and the exit
/// still has to be reported.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

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

/// What a host reports about its ptys.
#[derive(Debug, Clone)]
pub enum Event {
    Output {
        id: String,
        data: Vec<u8>,
        /// The OSC 7501 payloads completed by this output, the probe left
        /// out: it has been answered.
        statuses: Vec<String>,
        /// The process leading the foreground after this output.
        leader: Option<i32>,
    },
    /// The shell is gone, and the last of its output has been sent.
    Exit { id: String, code: i32 },
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// A terminal that was running in the relay before this app attached.
#[derive(Debug, Clone, Default)]
pub struct Survivor {
    pub id: String,
    pub path: String,
    pub shell_pid: Option<i32>,
    /// The process leading its foreground now.
    pub leader: Option<i32>,
    pub backlog: Vec<u8>,
    /// What the app last stored for it (`Host::set_state`).
    pub state: Option<serde_json::Value>,
    /// Status payloads that arrived while no app was attached, oldest first,
    /// each with when, in epoch milliseconds.
    pub statuses: Vec<(u64, String)>,
}

/// Where ptys live. Ids are the caller's, unique for as long as the host
/// lives (the app uses UUIDs, since a relay outlives the app's counters).
pub trait Host: Send + Sync {
    /// Start a shell for `path`. Returns the shell's pid.
    fn open(&self, id: &str, path: &str, spawn: &Spawn, size: PtySize) -> Result<Option<i32>, String>;
    fn write(&self, id: &str, data: &[u8]) -> Result<(), String>;
    fn resize(&self, id: &str, size: PtySize) -> Result<(), String>;
    /// SIGHUP to the shell. The exit is reported as an event like any other.
    fn hang_up(&self, id: &str);
    /// The process leading the pty's foreground: the shell, or a program it
    /// started.
    fn leader(&self, id: &str) -> Option<i32>;
    /// Keep an opaque value with the pty, handed back as `Survivor::state`
    /// to the next app that attaches.
    fn set_state(&self, _id: &str, _state: serde_json::Value) {}
    /// Where events go. Set once; events before it are dropped.
    fn set_sink(&self, sink: Sink);
    /// Whether its shells outlive the app.
    fn persistent(&self) -> bool {
        false
    }
}

struct Pty {
    path: String,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    shell_pid: Option<i32>,
    backlog: Mutex<Vec<u8>>,
}

/// Ptys in this process.
pub struct Local {
    ptys: Arc<Mutex<HashMap<String, Arc<Pty>>>>,
    sink: Arc<OnceLock<Sink>>,
    /// Held while output is added to a backlog and sent on, so the relay can
    /// take every backlog and attach an app between two reads: nothing lands
    /// in a backlog without also going to whoever is attached.
    gate: Arc<Mutex<()>>,
    keep_backlog: bool,
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
        Local {
            ptys: Arc::default(),
            sink: Arc::default(),
            gate: Arc::default(),
            keep_backlog,
        }
    }

    pub fn gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.gate)
    }

    pub fn is_empty(&self) -> bool {
        self.ptys.lock().unwrap().is_empty()
    }

    /// Every pty, with its backlog. Take the gate first for a snapshot that
    /// no output slips past.
    pub fn snapshot(&self) -> Vec<Survivor> {
        self.ptys
            .lock()
            .unwrap()
            .iter()
            .map(|(id, pty)| Survivor {
                id: id.clone(),
                path: pty.path.clone(),
                shell_pid: pty.shell_pid,
                leader: pty.master.lock().unwrap().process_group_leader(),
                backlog: pty.backlog.lock().unwrap().clone(),
                ..Survivor::default()
            })
            .collect()
    }

    fn get(&self, id: &str) -> Result<Arc<Pty>, String> {
        self.ptys
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no terminal {id}"))
    }

    fn emit(sink: &OnceLock<Sink>, event: Event) {
        if let Some(sink) = sink.get() {
            sink(event);
        }
    }
}

impl Host for Local {
    fn open(&self, id: &str, path: &str, spawn: &Spawn, size: PtySize) -> Result<Option<i32>, String> {
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
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let shell_pid = child.process_id().map(|pid| pid as i32);
        let pty = Arc::new(Pty {
            path: path.to_string(),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(child.clone_killer()),
            shell_pid,
            backlog: Mutex::default(),
        });
        self.ptys.lock().unwrap().insert(id.to_string(), Arc::clone(&pty));

        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        let reading = Arc::clone(&pty);
        let sink = Arc::clone(&self.sink);
        let gate = Arc::clone(&self.gate);
        let keep_backlog = self.keep_backlog;
        let reader_id = id.to_string();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            let mut scanner = StatusScanner::default();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let data = &buf[..n];
                        let mut statuses = scanner.feed(data);
                        if statuses.iter().any(|p| p.starts_with('?')) {
                            // The probe. Anything starting with `?` is taken
                            // as support.
                            let mut writer = reading.writer.lock().unwrap();
                            let _ = writer.write_all(b"\x1b]7501;?\x07");
                            let _ = writer.flush();
                            statuses.retain(|p| !p.starts_with('?'));
                        }
                        let leader = reading.master.lock().unwrap().process_group_leader();
                        let _gate = gate.lock().unwrap();
                        if keep_backlog {
                            append_backlog(&mut reading.backlog.lock().unwrap(), data);
                        }
                        Local::emit(
                            &sink,
                            Event::Output {
                                id: reader_id.clone(),
                                data: data.to_vec(),
                                statuses,
                                leader,
                            },
                        );
                    }
                }
            }
            let _ = drained_tx.send(());
        });

        let ptys = Arc::clone(&self.ptys);
        let sink = Arc::clone(&self.sink);
        let gate = Arc::clone(&self.gate);
        let id = id.to_string();
        std::thread::spawn(move || {
            let code = child.wait().map(|status| status.exit_code() as i32).unwrap_or(-1);
            let _ = drained_rx.recv_timeout(DRAIN_GRACE);
            let _gate = gate.lock().unwrap();
            let mut map = ptys.lock().unwrap();
            if map.get(&id).is_some_and(|p| Arc::ptr_eq(p, &pty)) {
                map.remove(&id);
            }
            drop(map);
            Local::emit(&sink, Event::Exit { id, code });
        });

        Ok(shell_pid)
    }

    fn write(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let pty = self.get(id)?;
        let mut writer = pty.writer.lock().unwrap();
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }

    fn resize(&self, id: &str, size: PtySize) -> Result<(), String> {
        let pty = self.get(id)?;
        let master = pty.master.lock().unwrap();
        master.resize(size).map_err(|e| e.to_string())
    }

    /// SIGHUP to the shell alone is enough: it leads the terminal's session,
    /// and when it exits the kernel hangs up on the foreground job too
    /// (`closing_hangs_up_on_the_foreground_job`; an explicit `killpg` of
    /// that group was tried and made no difference). Jobs the user put in
    /// the background are the shell's to hang up, as in any terminal.
    fn hang_up(&self, id: &str) {
        if let Ok(pty) = self.get(id) {
            let _ = pty.killer.lock().unwrap().kill();
        }
    }

    fn leader(&self, id: &str) -> Option<i32> {
        self.get(id).ok()?.master.lock().unwrap().process_group_leader()
    }

    fn set_sink(&self, sink: Sink) {
        let _ = self.sink.set(sink);
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
}
