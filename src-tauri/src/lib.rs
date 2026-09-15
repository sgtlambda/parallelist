pub mod workspace;
pub mod config;
pub mod gh;
pub mod git;
pub mod proc;
pub mod tasks;

use workspace::{BranchConflict, CreateRequest, RemovalImpact};
use config::{AppConfig, ProjectConfig};
use git::{AheadBehind, BranchInfo, Workspace, WorkspaceKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tasks::{Coalesced, Task, TaskContext, TaskEvent, TaskKind, TaskStatus, Tasks};
use tauri::{Emitter, Manager, State};

type RefreshSlot = Coalesced<Result<Overview, String>>;

struct AppState {
    config: Mutex<AppConfig>,
    dir: PathBuf,
    tasks: Arc<Tasks>,
    /// One per project, so overlapping refreshes of it share runs.
    refreshes: Mutex<HashMap<String, Arc<RefreshSlot>>>,
}

impl AppState {
    fn config(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    /// The stored project with its `.switcheroo.json` overrides applied.
    fn project(&self, id: &str) -> Result<ProjectConfig, String> {
        self.config()
            .project(id)
            .map(|r| r.resolved())
            .ok_or_else(|| format!("unknown project '{id}'"))
    }

    fn persist(&self, next: AppConfig) -> Result<(), String> {
        next.save(&self.dir)?;
        *self.config.lock().unwrap() = next;
        Ok(())
    }

    fn refresh_slot(&self, id: &str) -> Arc<RefreshSlot> {
        let mut slots = self.refreshes.lock().unwrap();
        Arc::clone(slots.entry(id.to_string()).or_default())
    }
}

/// Anything that can be told what stage a refresh has reached.
pub type Report<'a> = &'a (dyn Fn(&'static str, String, bool) + Sync);

/// One refresh of everything the three panes need.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub project: ProjectConfig,
    pub base_branch: String,
    pub slug: Option<String>,
    pub branches: Vec<BranchInfo>,
    pub workspaces: Vec<Workspace>,
    pub pull_requests: HashMap<String, gh::PullRequest>,
    /// Logged-in GitHub user, for the "assigned to me" filter.
    pub viewer: Option<String>,
    /// Non-fatal problems (no `gh`, not logged in, fetch failed) surfaced as a
    /// banner instead of an error dialog, so the rest still renders.
    pub warnings: Vec<String>,
}

#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state.config()
}

#[tauri::command]
fn save_config(state: State<AppState>, config: AppConfig) -> Result<(), String> {
    state.persist(config)
}

#[tauri::command]
fn add_project(state: State<AppState>, path: String) -> Result<ProjectConfig, String> {
    let path = PathBuf::from(shellexpand_home(&path));
    if !git::is_git_repo(&path) {
        return Err(format!("{} is not a git repository", path.display()));
    }
    if path.join(".git").is_file() {
        return Err("that is a worktree; add the main clone instead".into());
    }
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".into());

    let mut cfg = state.config();
    if let Some(existing) = cfg
        .projects
        .iter()
        .find(|r| Path::new(&r.path) == path)
        .cloned()
    {
        return Ok(existing);
    }
    let repo = ProjectConfig::new(path, name);
    cfg.projects.push(repo.clone());
    cfg.selected_project_id = Some(repo.id.clone());
    state.persist(cfg)?;
    Ok(repo)
}

#[tauri::command]
fn remove_project(state: State<AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config();
    cfg.projects.retain(|r| r.id != id);
    if cfg.selected_project_id.as_deref() == Some(id.as_str()) {
        cfg.selected_project_id = cfg.projects.first().map(|r| r.id.clone());
    }
    state.persist(cfg)
}

#[tauri::command]
fn select_project(state: State<AppState>, id: Option<String>) -> Result<(), String> {
    let mut cfg = state.config();
    cfg.selected_project_id = id;
    state.persist(cfg)
}

/// Refresh a project. A request that lands while a refresh of the same
/// project is running waits for the run after it, shared with every other
/// request that arrived meanwhile — the background poll, the focus check and a
/// finished task can all ask at once without stacking up.
#[tauri::command]
async fn refresh(state: State<'_, AppState>, id: String, fetch: bool) -> Result<Overview, String> {
    let project = state.project(&id)?;
    let slot = state.refresh_slot(&id);
    let tasks = Arc::clone(&state.tasks);
    tauri::async_runtime::spawn_blocking(move || {
        slot.request(fetch, |fetch| refresh_task(&tasks, project, fetch))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Wording for each refresh phase: while it runs, and once it is done.
fn phase_labels(phase: &str) -> (&'static str, &'static str) {
    match phase {
        "fetching" => ("Fetching from origin", "Fetched from origin"),
        "branches" => ("Reading branches", "Read branches"),
        "workspaces" => ("Checking working trees", "Checked working trees"),
        "pullRequests" => ("Loading pull requests", "Loaded pull requests"),
        _ => ("Working", "Done"),
    }
}

/// One refresh run as a task. Each phase is a single log line that its
/// counter rewrites in place, then its timing once it is done. The phases
/// overlap — pull requests load alongside the git work — so they can't share
/// one line.
fn refresh_task(tasks: &Tasks, project: ProjectConfig, fetch: bool) -> Result<Overview, String> {
    let id = tasks.add(
        &project.id,
        TaskKind::Refresh,
        format!("Refreshing {}", project.name),
        TaskContext::default(),
        TaskStatus::Running,
    );
    let phases: Mutex<HashMap<&'static str, (Instant, String)>> = Mutex::default();
    let report = |phase: &'static str, detail: String, active: bool| {
        let (running, done) = phase_labels(phase);
        let mut phases = phases.lock().unwrap();
        if active {
            let entry = phases
                .entry(phase)
                .or_insert_with(|| (Instant::now(), String::new()));
            let text = if detail.is_empty() {
                format!("{running}…")
            } else {
                format!("{running} {detail}")
            };
            entry.1 = detail;
            tasks.keyed_line(&id, phase, &text, false);
        } else if let Some((started, detail)) = phases.remove(phase) {
            let detail = if detail.is_empty() { String::new() } else { format!(" {detail}") };
            let secs = started.elapsed().as_secs_f32();
            tasks.keyed_line(&id, phase, &format!("{done}{detail} · {secs:.1}s"), false);
        }
    };

    let result = build_overview(project, fetch, &report);
    match &result {
        Ok(overview) => {
            // Non-fatal, and already a banner, but the log is where someone
            // looking into a slow or odd refresh will look.
            for warning in &overview.warnings {
                tasks.line(&id, warning, true);
            }
            tasks.finish(&id, Ok(()));
        }
        Err(message) => tasks.finish(&id, Err(message.clone())),
    }
    result
}

/// Gather everything the three panes need.
///
/// The GitHub queries touch the network and nothing in the git working tree,
/// so they run concurrently with fetch → branches → scan rather than after it.
/// That is most of the wall-clock: the two halves cost roughly the same.
pub fn build_overview(project: ProjectConfig, fetch: bool, report: Report) -> Result<Overview, String> {
    let path = PathBuf::from(&project.path);
    if !git::is_git_repo(&path) {
        return Err(format!("{} is no longer a git repository", project.path));
    }

    let base_branch = project
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&path));
    let slug = git::origin_url(&path).ok().and_then(|u| git::origin_slug(&u));

    type GitSide = Result<(Vec<BranchInfo>, Vec<Workspace>, Vec<String>), String>;
    type PrSide = Result<(HashMap<String, gh::PullRequest>, Option<String>), String>;

    let (git_side, pr_side): (GitSide, PrSide) = std::thread::scope(|scope| {
        let pulls = scope.spawn(|| -> PrSide {
            let Some(slug) = slug.as_deref() else {
                return Err("No GitHub remote detected — pull request info is off.".into());
            };
            if !gh::is_available() {
                return Err("gh CLI not found — pull request info is unavailable.".into());
            }
            report("pullRequests", String::new(), true);
            let result = gh::pull_requests(&path, slug)
                .map(|prs| (prs, gh::viewer_login().map(str::to_string)))
                .map_err(|e| format!("Could not read pull requests: {e}"));
            report("pullRequests", String::new(), false);
            result
        });

        let git = scope.spawn(|| -> GitSide {
            let mut warnings = Vec::new();
            if fetch {
                report("fetching", String::new(), true);
                // Non-fatal: everything below still works from local refs, it
                // is just one cycle stale.
                if let Err(e) = git::fetch(&path) {
                    warnings.push(format!("Couldn't reach origin — showing what's on disk. {e}"));
                }
                report("fetching", String::new(), false);
            }

            report("branches", String::new(), true);
            let mut branches = git::branches(&path, &base_branch)?;
            let remote_branches: std::collections::HashSet<String> = branches
                .iter()
                .filter(|b| b.has_remote)
                .map(|b| b.name.clone())
                .collect();
            report("branches", String::new(), false);

            report("workspaces", String::new(), true);
            let workspaces = workspace::scan(&project, &base_branch, &remote_branches, &|done, total| {
                report("workspaces", format!("{done}/{total}"), true)
            })?;
            report("workspaces", String::new(), false);

            // A secondary clone is an independent repository, so a branch
            // checked out there may not exist in the primary at all. Without
            // this those workspaces have no row to attach to and vanish from the
            // list while still being counted under Cleanup.
            //
            // Only branches that are actually checked out get added. Merging
            // every ref from every clone would drag in that clone's stale
            // origin/* snapshot and resurrect hundreds of dead branches.
            let mut extra: Vec<BranchInfo> = Vec::new();
            for workspace in &workspaces {
                let Some(name) = workspace.branch.as_deref() else {
                    continue;
                };
                if branches.iter().any(|b| b.name == name) || extra.iter().any(|b| b.name == name) {
                    continue;
                }
                let Some((sha, when, author, subject)) =
                    git::head_commit(&PathBuf::from(&workspace.path))
                else {
                    continue;
                };
                // A clone keeps its own remote-tracking ref, which survives the
                // branch being deleted on origin — the only evidence left that
                // it was ever pushed.
                let was_pushed = crate::proc::run(
                    "git",
                    &[
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/remotes/origin/{name}"),
                    ],
                    Some(&PathBuf::from(&workspace.path)),
                )
                .map(|out| out.ok())
                .unwrap_or(false);

                extra.push(BranchInfo {
                    name: name.to_string(),
                    has_remote: false,
                    has_local: true,
                    was_pushed,
                    sha,
                    committed_at: when,
                    author,
                    subject,
                    merged: false,
                });
            }
            if !extra.is_empty() {
                branches.extend(extra);
                branches.sort_by_key(|b| std::cmp::Reverse(b.committed_at));
            }

            Ok((branches, workspaces, warnings))
        });

        (
            git.join().unwrap_or_else(|_| Err("git work panicked".into())),
            pulls.join().unwrap_or_else(|_| Err("gh work panicked".into())),
        )
    });

    let (branches, workspaces, mut warnings) = git_side?;
    let (pull_requests, viewer) = match pr_side {
        Ok(value) => value,
        Err(message) => {
            warnings.push(message);
            (HashMap::new(), None)
        }
    };

    Ok(Overview {
        project,
        base_branch,
        slug,
        branches,
        workspaces,
        pull_requests,
        viewer,
        warnings,
    })
}

/// Ahead/behind for just the branches currently on screen. Each one costs a
/// `rev-list`, so the full 200-branch list never pays for it.
#[tauri::command]
async fn ahead_behind(
    state: State<'_, AppState>,
    id: String,
    base: String,
    branches: Vec<String>,
) -> Result<HashMap<String, AheadBehind>, String> {
    let repo = state.project(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&repo.path);
        let mut out = HashMap::new();
        for branch in branches {
            if let Some(counts) = git::ahead_behind(&path, &base, &branch) {
                out.insert(branch, counts);
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlan {
    suggested_path: String,
    /// Set when a worktree is impossible because the branch is checked out
    /// elsewhere — the dialog then offers a clone instead.
    conflict: Option<BranchConflict>,
}

#[tauri::command]
fn plan_workspace(
    state: State<AppState>,
    id: String,
    branch: String,
    pr: Option<u64>,
) -> Result<CreatePlan, String> {
    let repo = state.project(&id)?;
    Ok(CreatePlan {
        suggested_path: workspace::suggest_path(&repo, &branch, pr),
        conflict: workspace::conflicting_worktree(&repo, &branch),
    })
}

/// Start creating a workspace and return its task id straight away. Setup
/// steps run for minutes; the dialog closes as soon as this returns.
#[tauri::command]
fn create_workspace(
    state: State<AppState>,
    request: CreateRequest,
    pr_url: Option<String>,
) -> Result<String, String> {
    let project = state.project(&request.project_id)?;
    if state.tasks.is_creating(&request.path) {
        return Err(format!("{} is already being created", request.path));
    }
    let id = state.tasks.add(
        &project.id,
        TaskKind::Create,
        format!("Creating {}", request.branch),
        TaskContext {
            branch: Some(request.branch.clone()),
            path: Some(request.path.clone()),
            mode: Some(request.mode.clone()),
            pr_url,
        },
        TaskStatus::Queued,
    );
    state.tasks.spawn(
        id.clone(),
        Box::new(move |log| {
            let base = project
                .base_branch
                .clone()
                .unwrap_or_else(|| git::detect_base_branch(&PathBuf::from(&project.path)));
            workspace::create(&project, &base, &request, log).map(|_| ())
        }),
    );
    Ok(id)
}

/// What removing each of these would destroy, minus the size on disk.
///
/// Sizes come from [`workspace_sizes`] instead: `du` takes seconds on a large
/// working tree, and making the dialog wait for it before showing anything is
/// what made cleanup unusable.
#[tauri::command]
async fn removal_impacts(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> Result<Vec<RemovalImpact>, String> {
    let project = state.project(&id)?;
    let base = project
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&PathBuf::from(&project.path)));

    tauri::async_runtime::spawn_blocking(move || {
        let remote_branches = git::remote_branch_names(&PathBuf::from(&project.path));
        let known = workspace::scan(&project, &base, &remote_branches, &|_, _| {}).unwrap_or_default();
        parallel_map(paths, |path| {
            let found = known.iter().find(|w| w.path == *path);
            let kind = found.map(|w| w.kind).unwrap_or(WorkspaceKind::Orphan);
            let branch = found.and_then(|w| w.branch.clone());
            workspace::removal_impact(&PathBuf::from(path), kind, branch)
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Size on disk per path. Measured on a pool because `du` is slow and the
/// paths are independent.
#[tauri::command]
async fn workspace_sizes(paths: Vec<String>) -> Result<HashMap<String, u64>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        parallel_map(paths.clone(), |path| {
            (path.clone(), workspace::dir_size(&PathBuf::from(path)))
        })
        .into_iter()
        .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Run `f` over every item on a small pool, preserving order.
fn parallel_map<T, R>(items: Vec<T>, f: impl Fn(&T) -> R + Sync) -> Vec<R>
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

/// What deleting a branch would cost: the commits no other ref holds.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BranchImpact {
    branch: String,
    /// Newest first, capped at `COMMIT_PREVIEW`; `commit_count` is the truth.
    commits: Vec<git::Commit>,
    commit_count: u32,
}

/// Enough commits to recognise what a branch was, without turning the cleanup
/// list into a log viewer. The count next to them is never capped.
const COMMIT_PREVIEW: usize = 5;

/// Read-only: what each branch would take with it if deleted.
#[tauri::command]
async fn branch_impacts(
    state: State<'_, AppState>,
    id: String,
    branches: Vec<String>,
) -> Result<Vec<BranchImpact>, String> {
    let project = state.project(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let primary = PathBuf::from(&project.path);
        parallel_map(branches, |branch| {
            let (commits, commit_count) = git::unique_commits(&primary, branch, COMMIT_PREVIEW);
            BranchImpact {
                branch: branch.clone(),
                commits,
                commit_count,
            }
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Removals and branch deletions for one project run one at a time: they all
/// write refs and worktree registrations in the same clone, where concurrent
/// `git branch -D` calls trip over `packed-refs.lock`, and parallel `rm -rf`s
/// of `vendor/` only fight over the disk.
fn removal_lane(project_id: &str) -> String {
    format!("remove:{project_id}")
}

/// Queue one task per branch and return their ids.
#[tauri::command]
fn delete_branches(
    state: State<AppState>,
    id: String,
    branches: Vec<String>,
) -> Result<Vec<String>, String> {
    let project = state.project(&id)?;
    let lane = removal_lane(&project.id);
    Ok(branches
        .into_iter()
        .map(|branch| {
            let task = state.tasks.add(
                &project.id,
                TaskKind::DeleteBranch,
                format!("Deleting branch {branch}"),
                TaskContext {
                    branch: Some(branch.clone()),
                    ..TaskContext::default()
                },
                TaskStatus::Queued,
            );
            let project = project.clone();
            state.tasks.spawn_in_lane(
                &lane,
                task.clone(),
                Box::new(move |log| {
                    let primary = PathBuf::from(&project.path);
                    let base = project
                        .base_branch
                        .clone()
                        .unwrap_or_else(|| git::detect_base_branch(&primary));
                    workspace::log_step(log, &format!("git branch -D {branch}"));
                    workspace::delete_branch(&primary, &base, &branch)
                }),
            );
            task
        })
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemovalRequest {
    path: String,
    kind: WorkspaceKind,
    #[serde(default)]
    delete_branch: Option<String>,
}

/// Queue one task per workspace and return their ids. Each failure is its
/// own task, so one refusal doesn't abandon the rest of a sweep.
#[tauri::command]
fn remove_workspaces(
    state: State<AppState>,
    id: String,
    items: Vec<RemovalRequest>,
) -> Result<Vec<String>, String> {
    let project = state.project(&id)?;
    let lane = removal_lane(&project.id);
    Ok(items
        .into_iter()
        .map(|item| {
            let name = Path::new(&item.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| item.path.clone());
            let task = state.tasks.add(
                &project.id,
                TaskKind::Remove,
                format!("Removing {name}"),
                TaskContext {
                    path: Some(item.path.clone()),
                    branch: item.delete_branch.clone(),
                    ..TaskContext::default()
                },
                TaskStatus::Queued,
            );
            let project = project.clone();
            state.tasks.spawn_in_lane(
                &lane,
                task.clone(),
                Box::new(move |log| {
                    workspace::remove(&project, Path::new(&item.path), item.kind, item.delete_branch, log)
                }),
            );
            task
        })
        .collect())
}

/// Every task the registry holds, logs included. The window calls this once
/// at load and follows the events from there.
#[tauri::command]
fn list_tasks(state: State<AppState>) -> Vec<Task> {
    state.tasks.list()
}

/// One task afresh, for when the window notices it missed a log line.
#[tauri::command]
fn get_task(state: State<AppState>, id: String) -> Option<Task> {
    state.tasks.get(&id)
}

/// Flag a task for the user's attention, or dismiss it into the history.
#[tauri::command]
fn set_task_attention(state: State<AppState>, id: String, attention: bool) {
    state.tasks.set_attention(&id, attention);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskLinePayload<'a> {
    task_id: &'a str,
    line: &'a tasks::LogLine,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenContext {
    path: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    primary: String,
    #[serde(default)]
    pr_url: String,
}

/// Put text on the system clipboard.
///
/// `pbcopy` rather than a clipboard plugin: the app is macOS-only, it is always
/// present, and it leaves the dependency and permission surface unchanged.
#[tauri::command]
fn copy_text(text: String) -> Result<(), String> {
    let out = proc::run_with_input("pbcopy", &[], None, &text)?;
    if out.ok() {
        Ok(())
    } else {
        Err(format!("could not copy: {}", out.stderr.trim()))
    }
}

#[tauri::command]
fn open_in(state: State<AppState>, target_id: String, ctx: OpenContext) -> Result<(), String> {
    let cfg = state.config();
    // Project targets win on an id clash, so a project can override a global.
    let project_targets = cfg
        .projects
        .iter()
        .find(|p| p.path == ctx.primary)
        .map(|r| r.resolved().open_targets)
        .unwrap_or_default();
    let target = project_targets
        .iter()
        .chain(cfg.open_targets.iter())
        .find(|t| t.id == target_id)
        .ok_or_else(|| format!("unknown open target '{target_id}'"))?;

    // Values are shell-quoted on substitution so a path with spaces (or worse)
    // can't break out of the user's template.
    let script = target
        .command
        .replace("{path}", &proc::sh_quote(&ctx.path))
        .replace("{branch}", &proc::sh_quote(&ctx.branch))
        .replace("{primary}", &proc::sh_quote(&ctx.primary))
        .replace("{prUrl}", &proc::sh_quote(&ctx.pr_url));

    let cwd = PathBuf::from(&ctx.path);
    let cwd = if cwd.is_dir() { Some(cwd) } else { None };
    proc::spawn_shell_detached(&script, cwd.as_deref())
}

fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let config = AppConfig::load(&dir);
            // One shell spawn; do it off the critical path so the window
            // isn't waiting on it.
            std::thread::spawn(|| {
                proc::login_path();
            });
            let handle = app.handle().clone();
            let tasks = Arc::new(Tasks::new(move |event| {
                let _ = match event {
                    TaskEvent::Header(header) => handle.emit("task", header),
                    TaskEvent::Line { task_id, line } => {
                        handle.emit("task-line", TaskLinePayload { task_id, line })
                    }
                    TaskEvent::Removed(id) => handle.emit("task-removed", id),
                };
            }));
            app.manage(AppState {
                config: Mutex::new(config),
                dir,
                tasks,
                refreshes: Mutex::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            add_project,
            remove_project,
            select_project,
            refresh,
            ahead_behind,
            plan_workspace,
            create_workspace,
            removal_impacts,
            workspace_sizes,
            remove_workspaces,
            branch_impacts,
            delete_branches,
            open_in,
            copy_text,
            list_tasks,
            get_task,
            set_task_attention,
        ])
        .run(tauri::generate_context!())
        .expect("error while running switcheroo");
}
