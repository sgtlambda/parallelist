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

use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
}

impl Terminal {
    /// Send to the attached sink, if any; drop it if its receiver is gone.
    fn deliver(output: &mut Output, chunk: Chunk) {
        if let Some(sink) = &output.sink {
            if !sink(chunk) {
                output.sink = None;
            }
        }
    }
}

pub type ExitHook = Arc<dyn Fn(&TerminalInfo) + Send + Sync>;

#[derive(Default)]
pub struct Terminals {
    by_path: Arc<Mutex<HashMap<String, Arc<Terminal>>>>,
    /// Told about every exit, attached or not. A webview that reloaded marks
    /// running terminals from `list` before attaching to any of them, and a
    /// shell that exits meanwhile has no sink to report through.
    on_exit: Option<ExitHook>,
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
    pub fn new(on_exit: ExitHook) -> Self {
        Self {
            by_path: Arc::default(),
            on_exit: Some(on_exit),
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
        cmd.env("TERM_PROGRAM", "Switcheroo");
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

        let term = Arc::new(Terminal {
            info: TerminalInfo {
                path: key.clone(),
                project_id: project_id.to_string(),
            },
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            killer: Mutex::new(child.clone_killer()),
            output: Mutex::new(Output {
                backlog: Vec::new(),
                sink: Some(sink),
                exited: false,
            }),
        });
        map.insert(key.clone(), Arc::clone(&term));
        drop(map);

        let (drained_tx, drained_rx) = mpsc::channel::<()>();
        let reading = Arc::clone(&term);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut output = reading.output.lock().unwrap();
                        append_backlog(&mut output.backlog, &buf[..n]);
                        Terminal::deliver(&mut output, Chunk::Data(buf[..n].to_vec()));
                    }
                }
            }
            let _ = drained_tx.send(());
        });

        let by_path = Arc::clone(&self.by_path);
        let on_exit = self.on_exit.clone();
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
            if let Some(hook) = on_exit {
                hook(&term.info);
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

    pub fn write(&self, path: &str, data: &[u8]) -> Result<(), String> {
        let term = self.get(path)?;
        let mut writer = term.writer.lock().unwrap();
        writer.write_all(data).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
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
        // SIGHUP to the shell alone is enough: it leads the terminal's session,
        // and when it exits the kernel hangs up on the foreground job too
        // (`closing_hangs_up_on_the_foreground_job`; an explicit `killpg` of
        // that group was tried and made no difference). Jobs the user put in
        // the background are the shell's to hang up, as in any terminal.
        let _ = term.killer.lock().unwrap().kill();
        true
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        self.by_path
            .lock()
            .unwrap()
            .values()
            .map(|t| t.info.clone())
            .collect()
    }
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
        let dir = std::env::temp_dir().join(format!("switcheroo-term-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
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
        let missing = "/nonexistent/switcheroo-workspace";
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
    fn the_shell_gets_a_terminal_environment_and_not_the_launchers() {
        std::env::set_var("npm_config_prefix", "/tmp/leak");
        std::env::set_var("TERM_PROGRAM", "iTerm.app");
        let cmd = Terminals::shell_command("/tmp");
        assert!(cmd.get_env("npm_config_prefix").is_none());
        assert_eq!(cmd.get_env("TERM_PROGRAM").unwrap(), "Switcheroo");
        assert_eq!(cmd.get_env("TERM").unwrap(), "xterm-256color");
        assert!(cmd.is_default_prog(), "the user's login shell");
    }
}
