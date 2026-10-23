//! Thin wrappers around spawning external processes.
//!
//! Everything parallelist does to a repository goes through the real `git` and
//! `gh` binaries rather than a linked library: worktrees, hooks, credential
//! helpers and SSH config then behave exactly as they do in the user's shell.

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Ceiling for local commands. Generous: it exists to stop a wedged process
/// pinning the app forever, not to police slow ones.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Anything that talks to the network. A dead TCP connection produces no
/// error and no data — without this the whole refresh waits on it forever.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(45);

static LOGIN_PATH: OnceLock<Option<String>> = OnceLock::new();

/// `PATH` as the user's login shell sees it.
///
/// An app launched from Finder inherits launchd's minimal PATH, which has no
/// `/opt/homebrew/bin` — so `gh` doesn't exist and pull request data silently
/// disappears, while the same build launched from a terminal works fine. Ask
/// the shell once and use its answer for every child process.
pub fn login_path() -> Option<&'static str> {
    LOGIN_PATH
        .get_or_init(|| {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
            Command::new(shell)
                .arg("-lc")
                .arg("printf %s \"$PATH\"")
                .stdin(Stdio::null())
                .output()
                .ok()
                .filter(|out| out.status.success())
                .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .filter(|path| !path.is_empty())
        })
        .as_deref()
}

#[derive(Debug)]
pub struct Output {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == 0
    }

    /// Trimmed stdout, or an error carrying stderr when the command failed.
    pub fn text(self) -> Result<String, String> {
        if self.ok() {
            Ok(self.stdout.trim_end().to_string())
        } else {
            Err(if self.stderr.trim().is_empty() {
                format!("exited with status {}", self.status)
            } else {
                self.stderr.trim().to_string()
            })
        }
    }

    pub fn lines(&self) -> Vec<&str> {
        self.stdout.lines().filter(|l| !l.trim().is_empty()).collect()
    }
}

fn build(program: &str, args: &[&str], cwd: Option<&Path>) -> Command {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Some(path) = login_path() {
        cmd.env("PATH", path);
    }
    // Keep git's output machine-readable and never let it stop for input.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd.env("LC_ALL", "C");

    // Teach ssh to notice a connection that has died. Without keepalives a
    // half-open socket leaves `git fetch` blocked in packet_reader_read with
    // nothing to read and no error, indefinitely. BatchMode makes it fail
    // instead of waiting on a prompt no GUI app can answer. An explicitly
    // configured command always wins.
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=10 \
             -o ServerAliveInterval=10 -o ServerAliveCountMax=3",
        );
    }
    // Own process group, so a timeout can take the whole tree down. `git
    // fetch` spawns ssh, and killing git alone provably leaves that ssh alive
    // holding the dead socket.
    cmd.process_group(0);
    cmd
}

/// Quote a value for safe interpolation into a shell command.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Run a program to completion, capturing both streams.
pub fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<Output, String> {
    run_timeout(program, args, cwd, DEFAULT_TIMEOUT)
}

/// As [`run`], but kills the child if it outlives `timeout`.
pub fn run_timeout(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = build(program, args, cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {program}: {e}"))?;

    // Both pipes are drained on their own threads: waiting on the process
    // while a full pipe blocks it would deadlock regardless of any timeout.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);

    let status = wait_until(&mut child, timeout);
    let stdout = stdout.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = stderr.and_then(|h| h.join().ok()).unwrap_or_default();

    match status {
        Some(status) => Ok(Output {
            status: status.code().unwrap_or(-1),
            stdout,
            stderr,
        }),
        None => Err(format!(
            "{program} timed out after {}s",
            timeout.as_secs()
        )),
    }
}

/// Kill the child and everything it spawned.
fn kill_group(child: &mut Child) {
    let pid = child.id() as i32;
    // `build` put the child in its own group, so its pgid equals its pid.
    // SAFETY: killpg with a pid we own; a failure here is not actionable.
    let killed = unsafe { libc::killpg(pid, libc::SIGKILL) };
    if killed != 0 {
        let _ = child.kill();
    }
}

/// Run a program, writing `input` to its stdin.
///
/// stdin is written on its own thread: a program that emits more than a pipe
/// buffer's worth before finishing its read would otherwise deadlock against
/// our write.
pub fn run_with_input(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    input: &str,
) -> Result<Output, String> {
    use std::io::Write;

    let mut child = build(program, args, cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {program}: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload = input.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(payload.as_bytes());
        });
    }

    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let status = wait_until(&mut child, DEFAULT_TIMEOUT);

    Ok(Output {
        status: status.and_then(|s| s.code()).unwrap_or(-1),
        stdout: stdout.and_then(|h| h.join().ok()).unwrap_or_default(),
        stderr: stderr.and_then(|h| h.join().ok()).unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    })
}

/// Waits for the child, killing it once the deadline passes. Returns `None` if
/// it had to be killed.
fn wait_until(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            kill_group(child);
            // Reap it so the pipes close and the drain threads can finish.
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Run a command through the user's login shell and stream every line to a
/// callback. Used for setup steps and "open in" targets, both of which are
/// user-authored command templates that expect a normal interactive `PATH`.
pub fn run_shell_streaming<F>(script: &str, cwd: &Path, mut on_line: F) -> Result<i32, String>
where
    F: FnMut(&str, bool),
{
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut child = Command::new(&shell)
        .arg("-lc")
        .arg(script)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start {shell}: {e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // stderr is drained on a side thread so a chatty build tool can't fill the
    // pipe buffer and deadlock while we're still reading stdout.
    let err_handle = stderr.map(|err| {
        std::thread::spawn(move || {
            BufReader::new(err)
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
        })
    });

    if let Some(out) = stdout {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            on_line(&line, false);
        }
    }

    if let Some(handle) = err_handle {
        if let Ok(lines) = handle.join() {
            for line in lines {
                on_line(&line, true);
            }
        }
    }

    let status = child.wait().map_err(|e| format!("wait failed: {e}"))?;
    Ok(status.code().unwrap_or(-1))
}

/// Detach a GUI application. We deliberately don't wait for it: `open -a Foo`
/// returns fast, but `phpstorm path` keeps the process alive for the session.
pub fn spawn_shell_detached(script: &str, cwd: Option<&Path>) -> Result<(), String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let mut cmd = Command::new(&shell);
    cmd.arg("-lc").arg(script);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to launch: {e}"))?;
    Ok(())
}

/// Run `f` over every item on a small pool, preserving order.
pub fn parallel_map<T, R>(items: Vec<T>, f: impl Fn(&T) -> R + Sync) -> Vec<R>
where
    T: Sync,
    R: Send,
{
    if items.is_empty() {
        return Vec::new();
    }
    let workers = items.len().min(8);
    let chunk = items.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|chunk| scope.spawn(|| chunk.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kills_a_command_that_outlives_its_timeout() {
        let started = Instant::now();
        let err = run_timeout("sleep", &["30"], None, Duration::from_millis(300))
            .expect_err("a 30s sleep must not satisfy a 300ms timeout");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        // The point is that it returns promptly rather than waiting it out.
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_timeout_takes_grandchildren_with_it() {
        // Mirrors `git fetch` spawning ssh: killing only the direct child
        // leaves the grandchild running.
        let marker = std::env::temp_dir().join(format!("parallelist-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!(
            "sh -c 'sleep 2; touch {}' & wait",
            marker.to_string_lossy()
        );

        let err = run_timeout("sh", &["-c", &script], None, Duration::from_millis(200))
            .expect_err("must time out");
        assert!(err.contains("timed out"), "unexpected error: {err}");

        // If the grandchild survived, it creates the marker a second from now.
        std::thread::sleep(Duration::from_secs(3));
        assert!(
            !marker.exists(),
            "grandchild outlived the timeout and still ran"
        );
    }

    #[test]
    fn returns_output_for_a_command_that_finishes() {
        let out = run_timeout("echo", &["hello"], None, Duration::from_secs(10)).unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn writes_stdin_through_to_the_child() {
        let out = run_with_input("cat", &[], None, "branch/name with spaces").unwrap();
        assert!(out.ok());
        assert_eq!(out.stdout, "branch/name with spaces");
    }

    #[test]
    fn reports_failure_with_stderr() {
        let err = run_timeout("git", &["rev-parse", "--verify", "nope^{commit}"], None, DEFAULT_TIMEOUT)
            .unwrap()
            .text()
            .expect_err("bad revision must fail");
        assert!(!err.is_empty());
    }
}
