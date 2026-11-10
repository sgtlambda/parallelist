//! Watching the selected project's workspaces for changes made outside the app.
//!
//! A commit in a terminal, a branch switched in an IDE, a worktree added by
//! hand or a file saved all used to show up whenever the next poll came
//! round. Here they are noticed as they happen and handed on as one
//! `project-changed` per burst, which the webview answers with a local
//! refresh. The network half of a refresh — fetching, pull requests — cannot
//! be watched and stays on its timers.

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long the roots must stay silent before a burst counts as over.
const QUIET: Duration = Duration::from_millis(1000);

/// A burst that never goes quiet (a build writing for minutes) is reported
/// anyway after this long, so the list does not freeze for as long as it runs.
const MAX_WAIT: Duration = Duration::from_secs(10);

/// Pause after reporting, so back-to-back bursts become one later report
/// instead of a refresh each.
const COOL_DOWN: Duration = Duration::from_secs(3);

/// Directories whose contents are never the user's work. Their churn is the
/// bulk of the events in a working tree, and none of it moves a badge.
const NOISE: &[&str] = &[
    "node_modules",
    ".next",
    ".nuxt",
    ".turbo",
    ".parcel-cache",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
];

/// What in a `.git` directory says something a refresh reads has changed.
/// `objects/`, `logs/`, `hooks/` and the rest are written by almost every git
/// command and move nothing on their own.
const GIT_STATE: &[&str] = &[
    "HEAD",
    "index",
    "packed-refs",
    "refs",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "rebase-merge",
    "rebase-apply",
];

/// Whether a change at `path` (relative to the root being watched) is worth a
/// refresh.
pub fn relevant(path: &Path) -> bool {
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    let Some(&leaf) = parts.last() else {
        return true;
    };
    // Git's own lock files come and go around every write.
    if leaf.ends_with(".lock") {
        return false;
    }
    // Stray `.DS_Store` files are not uncommitted work (AGENTS.md, invariant 8).
    if leaf == ".DS_Store" || parts.iter().any(|p| NOISE.contains(p)) {
        return false;
    }
    let Some(dot_git) = parts.iter().position(|p| *p == ".git") else {
        return true;
    };
    match &parts[dot_git + 1..] {
        [] => true,
        // A worktree's own HEAD and index live in the clone's registry. The
        // entry itself appearing or going is a worktree added or removed.
        ["worktrees", rest @ ..] => match rest {
            [] | [_] => true,
            [_, file] => matches!(*file, "HEAD" | "index"),
            _ => false,
        },
        [first, ..] => GIT_STATE.contains(first),
    }
}

/// Watches one project at a time, the one on screen: switching projects
/// refreshes anyway, and the rest are read from the cache or a peek.
pub struct Watcher {
    state: Mutex<Current>,
    changes: Sender<String>,
}

#[derive(Default)]
struct Current {
    project: Option<String>,
    roots: Vec<PathBuf>,
    /// Held to keep it alive; dropping it stops the watch.
    watcher: Option<RecommendedWatcher>,
}

impl Watcher {
    /// `on_change` is called with a project's id once per burst of changes,
    /// from a thread of its own.
    pub fn new(on_change: impl Fn(&str) + Send + 'static) -> Self {
        let (changes, burst) = channel();
        std::thread::spawn(move || debounce(burst, on_change));
        Watcher {
            state: Mutex::default(),
            changes,
        }
    }

    /// Watch these directories — the workspaces of `project` — instead of
    /// whatever was watched before. A repeat of the same ones changes nothing.
    pub fn watch(&self, project: &str, roots: Vec<PathBuf>) {
        let mut current = self.state.lock().unwrap();
        if current.project.as_deref() == Some(project) && current.roots == roots {
            return;
        }
        current.watcher = None;
        current.project = Some(project.to_string());
        current.roots = roots.clone();

        let (id, changes, base) = (project.to_string(), self.changes.clone(), roots.clone());
        let handler = move |event: notify::Result<Event>| {
            let Ok(event) = event else { return };
            if event.kind.is_access() {
                return;
            }
            let hit = event.paths.iter().any(|path| {
                base.iter()
                    .filter_map(|root| path.strip_prefix(root).ok())
                    .any(relevant)
            });
            if hit {
                let _ = changes.send(id.clone());
            }
        };
        let Ok(mut watcher) = notify::recommended_watcher(handler) else {
            return;
        };
        for root in &roots {
            // A workspace that has just gone is not worth failing over.
            let _ = watcher.watch(root, RecursiveMode::Recursive);
        }
        current.watcher = Some(watcher);
    }
}

/// Turn a stream of changes into one call per burst: wait until it has been
/// quiet for `QUIET`, or `MAX_WAIT` since it began, then report each project
/// that changed.
fn debounce(burst: Receiver<String>, on_change: impl Fn(&str)) {
    while let Ok(first) = burst.recv() {
        let began = Instant::now();
        let mut changed = BTreeSet::from([first]);
        while let Some(left) = MAX_WAIT.checked_sub(began.elapsed()) {
            match burst.recv_timeout(QUIET.min(left)) {
                Ok(project) => {
                    changed.insert(project);
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        for project in &changed {
            on_change(project);
        }
        std::thread::sleep(COOL_DOWN);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(path: &str) -> bool {
        relevant(Path::new(path))
    }

    #[test]
    fn files_in_the_working_tree_count() {
        assert!(rel("src/main.rs"));
        assert!(rel("README.md"));
        assert!(rel("vendor/autoload.php"));
    }

    #[test]
    fn noise_does_not() {
        assert!(!rel(".DS_Store"));
        assert!(!rel("src/.DS_Store"));
        assert!(!rel("web/node_modules/left-pad/index.js"));
        assert!(!rel("app/__pycache__/x.pyc"));
    }

    #[test]
    fn git_state_counts_and_the_rest_of_git_does_not() {
        assert!(rel(".git/HEAD"));
        assert!(rel(".git/index"));
        assert!(rel(".git/refs/heads/feature"));
        assert!(rel(".git/refs/remotes/origin/main"));
        assert!(rel(".git/packed-refs"));
        assert!(rel(".git/rebase-merge/head-name"));
        assert!(!rel(".git/objects/ab/cdef"));
        assert!(!rel(".git/logs/HEAD"));
        assert!(!rel(".git/FETCH_HEAD"));
        assert!(!rel(".git/index.lock"));
        assert!(!rel(".git/refs/heads/feature.lock"));
    }

    #[test]
    fn a_worktrees_registry_entry_counts_for_its_head_and_index_only() {
        assert!(rel(".git/worktrees"));
        assert!(rel(".git/worktrees/fix-header"));
        assert!(rel(".git/worktrees/fix-header/HEAD"));
        assert!(rel(".git/worktrees/fix-header/index"));
        assert!(!rel(".git/worktrees/fix-header/logs/HEAD"));
        assert!(!rel(".git/worktrees/fix-header/gitdir"));
    }

    #[test]
    fn a_worktree_nested_in_another_workspace_is_read_by_its_own_git_dir() {
        assert!(rel(".worktrees/x/src/lib.rs"));
        assert!(rel(".git/worktrees/x/HEAD"));
        assert!(!rel(".worktrees/x/node_modules/y.js"));
    }

    #[test]
    fn a_burst_is_reported_once_when_it_goes_quiet() {
        let (changes, burst) = channel();
        let (seen_tx, seen) = channel();
        std::thread::spawn(move || debounce(burst, move |p| seen_tx.send(p.to_string()).unwrap()));

        for _ in 0..5 {
            changes.send("a".to_string()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
        }
        changes.send("b".to_string()).unwrap();

        assert_eq!(seen.recv_timeout(Duration::from_secs(3)).unwrap(), "a");
        assert_eq!(seen.recv_timeout(Duration::from_secs(1)).unwrap(), "b");
        assert!(seen.recv_timeout(Duration::from_millis(1500)).is_err(), "once each");
    }

    #[test]
    fn a_real_change_on_disk_is_noticed_and_noise_is_not() {
        let dir = std::env::temp_dir().join(format!("parallelist-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        let root = dir.canonicalize().unwrap();
        // FSEvents can still report what happened just before a stream began.
        std::thread::sleep(Duration::from_secs(1));

        let (seen_tx, seen) = channel();
        let watcher = Watcher::new(move |p| seen_tx.send(p.to_string()).unwrap());
        watcher.watch("p", vec![root.clone()]);
        std::thread::sleep(Duration::from_millis(500));

        std::fs::write(root.join("node_modules/x.js"), "x").unwrap();
        assert!(seen.recv_timeout(Duration::from_secs(3)).is_err(), "noise stays quiet");

        std::fs::write(root.join("a.txt"), "a").unwrap();
        assert_eq!(seen.recv_timeout(Duration::from_secs(5)).unwrap(), "p");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
