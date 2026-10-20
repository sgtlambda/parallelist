//! Background work the window reports in its task box: refreshing a project,
//! creating a workspace, removing one, deleting a branch.
//!
//! The registry lives here rather than in the webview so a task and its log
//! outlive the page: a webview reload in the middle of `pnpm install` finds
//! the task still running with everything it has printed so far. Tasks cannot
//! be cancelled — that is a decision, not an omission.

use serde::Serialize;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// A task keeps its newest lines. A setup step can print tens of thousands;
/// the webview trims to the same bound, so the two stay in step.
pub const MAX_LINES: usize = 5000;

/// Finished creates, removals and branch deletions kept as history.
const HISTORY: usize = 20;

/// Dismissed failed refreshes kept as history. Every poll of a project whose
/// directory has vanished fails, so this needs a bound of its own.
const FAILED_REFRESHES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    Refresh,
    Create,
    Remove,
    DeleteBranch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Queued,
    Running,
    Done,
    Failed,
}

/// What the webview needs to draw a task somewhere other than the task box —
/// a pending card, a card marked for removal — and to act on it once done.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskContext {
    pub branch: Option<String>,
    pub path: Option<String>,
    /// `worktree` or `clone`, for a create.
    pub mode: Option<String>,
    pub pr_url: Option<String>,
    /// For a create: the "open in" targets to run once it is done. Kept here
    /// rather than in the webview so they still run after a reload mid-create;
    /// the webview runs them, as it does every terminal target.
    pub open_targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHeader {
    pub id: String,
    pub project_id: String,
    pub kind: TaskKind,
    pub title: String,
    pub status: TaskStatus,
    /// Wants the user's eye: set when a task fails, or by the webview when a
    /// finished create's card is not on screen. Cleared by dismissing.
    pub attention: bool,
    /// Milliseconds since the epoch.
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    #[serde(flatten)]
    pub context: TaskContext,
    /// Bumped on every header change, registry-wide. The webview subscribes
    /// before it lists, so an event can arrive older than the listing; this
    /// is how it tells.
    pub rev: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    /// Position in the task's log since it started, never reused. A keyed
    /// line is rewritten in place under the same number.
    pub n: u64,
    pub text: String,
    pub error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    #[serde(flatten)]
    pub header: TaskHeader,
    pub log: VecDeque<LogLine>,
}

pub enum TaskEvent<'a> {
    Header(&'a TaskHeader),
    Line { task_id: &'a str, line: &'a LogLine },
    Removed(&'a str),
}

struct Entry {
    task: Task,
    next_line: u64,
    /// Line number per key, so a counter ("12/16") rewrites its line.
    keyed: HashMap<String, u64>,
}

#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    rev: u64,
}

impl Registry {
    fn entry(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.task.header.id == id)
    }
}

/// The work a task does. It is handed a log to write to, and its result
/// becomes the task's status; an error is appended to the log.
pub type Work = Box<dyn FnOnce(&mut dyn FnMut(&str, bool)) -> Result<(), String> + Send>;

pub struct Tasks {
    registry: Mutex<Registry>,
    emit: Box<dyn Fn(TaskEvent) + Send + Sync>,
    lanes: Lanes,
}

impl Tasks {
    /// `emit` is called with the registry locked, so events arrive in the
    /// order the changes were made. It must not call back into the registry.
    pub fn new(emit: impl Fn(TaskEvent) + Send + Sync + 'static) -> Self {
        Tasks {
            registry: Mutex::default(),
            emit: Box::new(emit),
            lanes: Lanes::default(),
        }
    }

    pub fn add(
        &self,
        project_id: &str,
        kind: TaskKind,
        title: String,
        context: TaskContext,
        status: TaskStatus,
    ) -> String {
        let mut reg = self.registry.lock().unwrap();
        reg.rev += 1;
        let now = now_ms();
        let header = TaskHeader {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            kind,
            title,
            status,
            attention: false,
            created_at: now,
            started_at: (status == TaskStatus::Running).then_some(now),
            finished_at: None,
            context,
            rev: reg.rev,
        };
        (self.emit)(TaskEvent::Header(&header));
        let id = header.id.clone();
        reg.entries.push(Entry {
            task: Task {
                header,
                log: VecDeque::new(),
            },
            next_line: 0,
            keyed: HashMap::new(),
        });
        id
    }

    pub fn list(&self) -> Vec<Task> {
        let reg = self.registry.lock().unwrap();
        reg.entries.iter().map(|e| e.task.clone()).collect()
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        let mut reg = self.registry.lock().unwrap();
        reg.entry(id).map(|e| e.task.clone())
    }

    /// A create for this directory that has not finished. The second one
    /// would only fail after the first had made the directory.
    pub fn is_creating(&self, path: &str) -> bool {
        let reg = self.registry.lock().unwrap();
        reg.entries.iter().any(|e| {
            let h = &e.task.header;
            h.kind == TaskKind::Create
                && matches!(h.status, TaskStatus::Queued | TaskStatus::Running)
                && h.context.path.as_deref() == Some(path)
        })
    }

    pub fn line(&self, id: &str, text: &str, error: bool) {
        let mut reg = self.registry.lock().unwrap();
        self.write_line(&mut reg, id, None, text, error);
    }

    /// Write a line that later lines with the same key overwrite, so a
    /// progress counter is one line rather than one per tick.
    pub fn keyed_line(&self, id: &str, key: &str, text: &str, error: bool) {
        let mut reg = self.registry.lock().unwrap();
        self.write_line(&mut reg, id, Some(key), text, error);
    }

    fn write_line(&self, reg: &mut Registry, id: &str, key: Option<&str>, text: &str, error: bool) {
        let Some(entry) = reg.entry(id) else { return };
        if let Some(n) = key.and_then(|k| entry.keyed.get(k)).copied() {
            let first = entry.task.log.front().map(|l| l.n).unwrap_or(0);
            // Trimmed away by the cap: fall through and append it afresh.
            if let Some(line) = n.checked_sub(first).and_then(|i| entry.task.log.get_mut(i as usize)) {
                line.text = text.to_string();
                line.error = error;
                (self.emit)(TaskEvent::Line { task_id: id, line });
                return;
            }
        }
        let line = LogLine {
            n: entry.next_line,
            text: text.to_string(),
            error,
        };
        entry.next_line += 1;
        if let Some(k) = key {
            entry.keyed.insert(k.to_string(), line.n);
        }
        (self.emit)(TaskEvent::Line { task_id: id, line: &line });
        entry.task.log.push_back(line);
        if entry.task.log.len() > MAX_LINES {
            entry.task.log.pop_front();
        }
    }

    fn update(&self, reg: &mut Registry, id: &str, change: impl FnOnce(&mut TaskHeader)) {
        reg.rev += 1;
        let rev = reg.rev;
        if let Some(entry) = reg.entry(id) {
            change(&mut entry.task.header);
            entry.task.header.rev = rev;
            (self.emit)(TaskEvent::Header(&entry.task.header));
        }
    }

    pub fn start(&self, id: &str) {
        let mut reg = self.registry.lock().unwrap();
        self.update(&mut reg, id, |h| {
            h.status = TaskStatus::Running;
            h.started_at = Some(now_ms());
        });
    }

    pub fn finish(&self, id: &str, result: Result<(), String>) {
        let mut reg = self.registry.lock().unwrap();
        if let Err(message) = &result {
            self.write_line(&mut reg, id, None, message, true);
        }
        let failed = result.is_err();
        self.update(&mut reg, id, |h| {
            h.status = if failed { TaskStatus::Failed } else { TaskStatus::Done };
            h.finished_at = Some(now_ms());
            h.attention = failed;
        });

        // A refresh supersedes the ones before it: whether this one worked or
        // failed again, an older failure for the same project no longer needs
        // pointing out. It stays in the history.
        let finished = reg.entry(id).map(|e| (e.task.header.kind, e.task.header.project_id.clone()));
        if let Some((TaskKind::Refresh, project)) = finished {
            let stale: Vec<String> = reg
                .entries
                .iter()
                .map(|e| &e.task.header)
                .filter(|h| h.kind == TaskKind::Refresh && h.project_id == project)
                .filter(|h| h.attention && h.id != id)
                .map(|h| h.id.clone())
                .collect();
            for other in stale {
                self.update(&mut reg, &other, |h| h.attention = false);
            }
        }
        self.prune(&mut reg);
    }

    pub fn set_attention(&self, id: &str, attention: bool) {
        let mut reg = self.registry.lock().unwrap();
        self.update(&mut reg, id, |h| h.attention = attention);
        self.prune(&mut reg);
    }

    /// Drop finished tasks past what the history keeps: the newest successful
    /// refresh per project, a bounded run of failed refreshes, and the newest
    /// [`HISTORY`] of everything else. Nothing running or asking for
    /// attention is ever dropped.
    fn prune(&self, reg: &mut Registry) {
        let mut finished: Vec<&TaskHeader> = reg
            .entries
            .iter()
            .map(|e| &e.task.header)
            .filter(|h| h.finished_at.is_some() && !h.attention)
            .collect();
        finished.sort_by_key(|h| Reverse(h.finished_at));

        let mut refreshed = HashSet::new();
        let (mut failed_refreshes, mut history) = (0, 0);
        let mut doomed = HashSet::new();
        for h in finished {
            let keep = match (h.kind, h.status) {
                (TaskKind::Refresh, TaskStatus::Done) => refreshed.insert(h.project_id.as_str()),
                (TaskKind::Refresh, _) => {
                    failed_refreshes += 1;
                    failed_refreshes <= FAILED_REFRESHES
                }
                _ => {
                    history += 1;
                    history <= HISTORY
                }
            };
            if !keep {
                doomed.insert(h.id.clone());
            }
        }
        if doomed.is_empty() {
            return;
        }
        reg.entries.retain(|e| !doomed.contains(&e.task.header.id));
        for id in &doomed {
            (self.emit)(TaskEvent::Removed(id));
        }
    }

    /// Run a task on a thread of its own, now.
    pub fn spawn(self: &Arc<Self>, id: String, work: Work) {
        let tasks = Arc::clone(self);
        std::thread::spawn(move || tasks.execute(&id, work));
    }

    /// Run a task once every task queued before it in `lane` has finished.
    /// Until then it shows as queued.
    pub fn spawn_in_lane(self: &Arc<Self>, lane: &str, id: String, work: Work) {
        let tasks = Arc::clone(self);
        self.lanes.push(lane, Box::new(move || tasks.execute(&id, work)));
    }

    fn execute(&self, id: &str, work: Work) {
        self.start(id);
        let mut log = |text: &str, error: bool| self.line(id, text, error);
        // A panic must not leave the task spinning forever in the box.
        let result = catch_unwind(AssertUnwindSafe(|| work(&mut log)))
            .unwrap_or_else(|_| Err("the task stopped unexpectedly".into()));
        self.finish(id, result);
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

type Job = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct LaneState {
    pending: HashMap<String, VecDeque<Job>>,
    /// Lanes with a worker thread draining them.
    busy: HashSet<String>,
}

/// Named FIFO queues, each drained by at most one thread at a time.
#[derive(Default, Clone)]
struct Lanes {
    state: Arc<Mutex<LaneState>>,
}

impl Lanes {
    fn push(&self, lane: &str, job: Job) {
        let mut state = self.state.lock().unwrap();
        state.pending.entry(lane.to_string()).or_default().push_back(job);
        if !state.busy.insert(lane.to_string()) {
            return;
        }
        drop(state);

        let shared = Arc::clone(&self.state);
        let lane = lane.to_string();
        std::thread::spawn(move || loop {
            let job = {
                let mut state = shared.lock().unwrap();
                match state.pending.get_mut(&lane).and_then(VecDeque::pop_front) {
                    Some(job) => job,
                    None => {
                        state.pending.remove(&lane);
                        state.busy.remove(&lane);
                        return;
                    }
                }
            };
            job();
        });
    }
}

struct Slot<T> {
    running: bool,
    /// Runs started so far; the current or most recent run is this number.
    started: u64,
    /// Asked for by a request that arrived mid-run, for the next run.
    queued_flag: bool,
    last: Option<(u64, T)>,
}

/// Collapses overlapping requests for the same work into as few runs as
/// possible without ever answering with stale data.
///
/// A request while nothing runs starts a run. A request while one is running
/// cannot use that run — it may have started before whatever prompted the
/// request — so it waits for the *next* run, and every request that arrives
/// during the same run shares that one. The flag (for a refresh: whether to
/// fetch) is or-ed across everyone sharing a run.
pub struct Coalesced<T> {
    slot: Mutex<Slot<T>>,
    wake: Condvar,
}

impl<T: Clone> Default for Coalesced<T> {
    fn default() -> Self {
        Coalesced {
            slot: Mutex::new(Slot {
                running: false,
                started: 0,
                queued_flag: false,
                last: None,
            }),
            wake: Condvar::new(),
        }
    }
}

impl<T: Clone> Coalesced<T> {
    pub fn request(&self, flag: bool, run: impl FnOnce(bool) -> T) -> T {
        let mut slot = self.slot.lock().unwrap();
        let wanted = slot.started + 1;
        if slot.running {
            slot.queued_flag |= flag;
        }
        loop {
            if let Some((run_no, value)) = &slot.last {
                if *run_no >= wanted {
                    return value.clone();
                }
            }
            if !slot.running {
                slot.running = true;
                slot.started += 1;
                let run_no = slot.started;
                let flag = flag || std::mem::take(&mut slot.queued_flag);
                drop(slot);

                let value = catch_unwind(AssertUnwindSafe(|| run(flag)));
                let mut slot = self.slot.lock().unwrap();
                slot.running = false;
                match value {
                    Ok(value) => {
                        slot.last = Some((run_no, value.clone()));
                        self.wake.notify_all();
                        return value;
                    }
                    Err(panic) => {
                        // Let a waiter take the next run instead of hanging.
                        self.wake.notify_all();
                        drop(slot);
                        std::panic::resume_unwind(panic);
                    }
                }
            }
            slot = self.wake.wait(slot).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn quiet() -> Arc<Tasks> {
        Arc::new(Tasks::new(|_| {}))
    }

    fn finished(tasks: &Tasks, project: &str, kind: TaskKind, result: Result<(), String>) -> String {
        let id = tasks.add(project, kind, "t".into(), TaskContext::default(), TaskStatus::Running);
        tasks.finish(&id, result);
        // Distinct finish times, so "newest" is well defined.
        std::thread::sleep(Duration::from_millis(2));
        id
    }

    fn ids(tasks: &Tasks) -> Vec<String> {
        tasks.list().into_iter().map(|t| t.header.id).collect()
    }

    #[test]
    fn keeps_only_the_newest_successful_refresh_per_project() {
        let tasks = quiet();
        let a1 = finished(&tasks, "a", TaskKind::Refresh, Ok(()));
        let b1 = finished(&tasks, "b", TaskKind::Refresh, Ok(()));
        let a2 = finished(&tasks, "a", TaskKind::Refresh, Ok(()));
        let kept = ids(&tasks);
        assert!(!kept.contains(&a1));
        assert!(kept.contains(&b1) && kept.contains(&a2));
    }

    #[test]
    fn a_failure_stays_until_dismissed_and_a_later_refresh_quiets_it() {
        let tasks = quiet();
        let failed = finished(&tasks, "a", TaskKind::Refresh, Err("gone".into()));
        let task = tasks.get(&failed).unwrap();
        assert!(task.header.attention);
        assert_eq!(task.log.back().unwrap().text, "gone");

        finished(&tasks, "b", TaskKind::Refresh, Ok(()));
        assert!(tasks.get(&failed).unwrap().header.attention, "another project's refresh");

        finished(&tasks, "a", TaskKind::Refresh, Ok(()));
        let task = tasks.get(&failed).expect("failed refreshes stay in history");
        assert!(!task.header.attention);
    }

    #[test]
    fn history_is_capped_but_attention_is_never_pruned() {
        let tasks = quiet();
        let flagged = finished(&tasks, "a", TaskKind::Create, Ok(()));
        tasks.set_attention(&flagged, true);
        let first = finished(&tasks, "a", TaskKind::Remove, Ok(()));
        for _ in 0..HISTORY {
            finished(&tasks, "a", TaskKind::Remove, Ok(()));
        }
        let kept = ids(&tasks);
        assert!(kept.contains(&flagged));
        assert!(!kept.contains(&first));
        assert_eq!(kept.len(), HISTORY + 1);

        tasks.set_attention(&flagged, false);
        assert!(!ids(&tasks).contains(&flagged), "dismissed, and older than the whole history");
    }

    #[test]
    fn keyed_lines_rewrite_in_place() {
        let tasks = quiet();
        let id = tasks.add("a", TaskKind::Refresh, "t".into(), TaskContext::default(), TaskStatus::Running);
        tasks.keyed_line(&id, "scan", "1/3", false);
        tasks.line(&id, "other", false);
        tasks.keyed_line(&id, "scan", "3/3", false);
        let log: Vec<_> = tasks.get(&id).unwrap().log.into_iter().map(|l| (l.n, l.text)).collect();
        assert_eq!(log, vec![(0, "3/3".to_string()), (1, "other".to_string())]);
    }

    #[test]
    fn the_log_keeps_its_newest_lines() {
        let tasks = quiet();
        let id = tasks.add("a", TaskKind::Create, "t".into(), TaskContext::default(), TaskStatus::Running);
        for i in 0..MAX_LINES + 10 {
            tasks.line(&id, &i.to_string(), false);
        }
        let log = tasks.get(&id).unwrap().log;
        assert_eq!(log.len(), MAX_LINES);
        assert_eq!(log.front().unwrap().n, 10);
    }

    #[test]
    fn a_lane_runs_its_tasks_one_at_a_time_in_order() {
        let tasks = quiet();
        let active = Arc::new(AtomicUsize::new(0));
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut all = Vec::new();
        for i in 0..4 {
            let id = tasks.add("a", TaskKind::Remove, "t".into(), TaskContext::default(), TaskStatus::Queued);
            all.push(id.clone());
            let (active, order) = (Arc::clone(&active), Arc::clone(&order));
            tasks.spawn_in_lane(
                "remove:a",
                id,
                Box::new(move |_| {
                    assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0, "overlapped");
                    std::thread::sleep(Duration::from_millis(10));
                    order.lock().unwrap().push(i);
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }),
            );
        }
        for _ in 0..200 {
            if all.iter().all(|id| tasks.get(id).unwrap().header.status == TaskStatus::Done) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*order.lock().unwrap(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn overlapping_requests_share_one_follow_up_run() {
        let coalesced = Arc::new(Coalesced::<u64>::default());
        let runs = Arc::new(AtomicUsize::new(0));
        let fetched = Arc::new(Mutex::new(Vec::new()));

        let run = |runs: Arc<AtomicUsize>, fetched: Arc<Mutex<Vec<bool>>>| {
            move |flag: bool| {
                let n = runs.fetch_add(1, Ordering::SeqCst) as u64 + 1;
                fetched.lock().unwrap().push(flag);
                std::thread::sleep(Duration::from_millis(60));
                n
            }
        };

        let first = {
            let (c, r, f) = (Arc::clone(&coalesced), Arc::clone(&runs), Arc::clone(&fetched));
            std::thread::spawn(move || c.request(false, run(r, f)))
        };
        std::thread::sleep(Duration::from_millis(15));
        let followers: Vec<_> = [false, true, false]
            .into_iter()
            .map(|flag| {
                let (c, r, f) = (Arc::clone(&coalesced), Arc::clone(&runs), Arc::clone(&fetched));
                std::thread::spawn(move || c.request(flag, run(r, f)))
            })
            .collect();

        assert_eq!(first.join().unwrap(), 1);
        for follower in followers {
            assert_eq!(follower.join().unwrap(), 2, "waited for the run after the one in flight");
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2);
        assert_eq!(*fetched.lock().unwrap(), vec![false, true], "flags are or-ed into the shared run");
    }
}
