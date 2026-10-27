pub mod workspace;
pub mod config;
pub mod gh;
pub mod git;
pub mod link;
pub mod proc;
pub mod status;
pub mod tasks;
pub mod terminal;
pub mod transcript;

use workspace::{BranchConflict, CreateRequest, RemovalImpact};
use config::{AppConfig, ProjectConfig};
use git::{AheadBehind, BranchInfo, Workspace, WorkspaceKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use proc::parallel_map;
use tasks::{Coalesced, Task, TaskContext, TaskEvent, TaskKind, TaskStatus, Tasks};
use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem};
use tauri::{Emitter, Manager, State};

type RefreshSlot = Coalesced<Result<Overview, String>>;

struct AppState {
    config: Mutex<AppConfig>,
    dir: PathBuf,
    tasks: Arc<Tasks>,
    /// One per project, so overlapping refreshes of it share runs.
    refreshes: Mutex<HashMap<String, Arc<RefreshSlot>>>,
    terminals: Arc<terminal::Terminals>,
}

impl AppState {
    fn config(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    /// The stored project with its `.parallelist.json` overrides applied.
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

/// Which of the paths dragged onto the window are folders. Asked as the drag
/// enters, since the event says nothing more than the paths: folders can be
/// dropped anywhere, anything else only on a terminal.
#[tauri::command]
fn are_folders(paths: Vec<String>) -> Vec<bool> {
    paths.iter().map(|p| Path::new(p).is_dir()).collect()
}

/// Where a folder dropped on the window, or picked to add, leads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Landing {
    project_id: String,
    /// The branch to select: the one the folder has checked out, unless it is
    /// a main workspace, where the project is left on whatever was selected.
    branch: Option<String>,
    /// Added as a project just now.
    added: bool,
}

/// What a folder is to the app, before anything is added.
enum Folder {
    Known(Landing),
    /// Belongs to no project: this clone is the one to add.
    New { clone: PathBuf, branch: Option<String> },
}

fn locate_folder(projects: &[ProjectConfig], path: &Path) -> Result<Folder, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let known = |(project_id, entry): (String, workspace::Entry)| {
        Folder::Known(Landing {
            project_id,
            branch: if entry.kind == WorkspaceKind::Primary { None } else { entry.branch },
            added: false,
        })
    };
    let repo = git::workspace_root(path);
    let origin = repo.as_ref().and_then(|(_, clone)| git::origin_url_from_config(clone));
    // A folder inside a workspace stands for that workspace. One git knows
    // nothing about may still be an orphan.
    let dir = repo.as_ref().map_or(path, |(root, _)| root.as_path());
    if let Some(hit) = workspace::locate(projects, dir, origin.as_deref()) {
        return Ok(known(hit));
    }
    let (root, clone) = repo.ok_or_else(|| format!("{} is not a git repository", path.display()))?;
    if clone == root {
        // As typed rather than as git resolved it, when it is the root.
        let clone = if workspace::resolve(path) == root { path.to_path_buf() } else { root };
        return Ok(Folder::New { clone, branch: None });
    }
    // A worktree no project lists: its clone may still be known, else the
    // clone is what gets added, as the picker asks for a main workspace.
    let branch = git::current_branch(&root);
    if let Some((project_id, _)) = workspace::locate(projects, &clone, origin.as_deref()) {
        return Ok(Folder::Known(Landing { project_id, branch, added: false }));
    }
    Ok(Folder::New { clone, branch })
}

/// Go to the project or workspace a folder is, adding it as a project when it
/// is neither. A folder inside a workspace counts as that workspace, and a
/// worktree of a clone nobody added brings in the clone.
#[tauri::command]
async fn open_folder(state: State<'_, AppState>, path: String) -> Result<Landing, String> {
    let path = PathBuf::from(shellexpand_home(&path));
    let projects = state.config().projects;
    let folder = tauri::async_runtime::spawn_blocking(move || locate_folder(&projects, &path))
        .await
        .map_err(|e| e.to_string())??;
    let (clone, branch) = match folder {
        Folder::Known(landing) => return Ok(landing),
        Folder::New { clone, branch } => (clone, branch),
    };

    let mut cfg = state.config();
    // Added by another drop while git was being asked.
    let resolved = workspace::resolve(&clone);
    if let Some(existing) = cfg
        .projects
        .iter()
        .find(|p| workspace::resolve(Path::new(&p.path)) == resolved)
    {
        return Ok(Landing { project_id: existing.id.clone(), branch, added: false });
    }
    let name = clone
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".into());
    let project = ProjectConfig::new(clone, name);
    let project_id = project.id.clone();
    cfg.projects.push(project);
    state.persist(cfg)?;
    Ok(Landing { project_id, branch, added: true })
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
    let overview = tauri::async_runtime::spawn_blocking(move || {
        slot.request(fetch, |fetch| refresh_task(&tasks, project, fetch))
    })
    .await
    .map_err(|e| e.to_string())??;
    // The first to notice a workspace removed from inside its terminal,
    // perhaps, which may be all that terminal was waiting for.
    state.terminals.reap();
    Ok(overview)
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
                // Secondary clones too, alongside the main one. Each keeps its
                // own `origin/*`, which only a fetch in that clone moves, and
                // every status measured there — unpushed, behind — is against
                // it. Left alone, a clone's snapshot is as old as the last time
                // someone fetched in it by hand: weeks, in practice.
                let mut repos = vec![path.clone()];
                repos.extend(workspace::secondary_clones(&project));
                let results = parallel_map(repos, |repo| (repo.clone(), git::fetch(repo)));
                // Non-fatal: everything below still works from local refs, it
                // is just one cycle stale.
                for (repo, result) in results {
                    let Err(e) = result else { continue };
                    warnings.push(if repo == path {
                        format!("Couldn't reach origin — showing what's on disk. {e}")
                    } else {
                        format!("Couldn't fetch in {} — its status may be out of date. {e}", repo.display())
                    });
                }
                report("fetching", String::new(), false);
            }

            report("branches", String::new(), true);
            let mut branches = git::branches(&path, &base_branch)?;
            report("branches", String::new(), false);

            report("workspaces", String::new(), true);
            let workspaces = workspace::scan(&project, &base_branch, &|done, total| {
                report("workspaces", format!("{done}/{total}"), true)
            })?;
            report("workspaces", String::new(), false);

            add_clone_only_branches(&mut branches, &workspaces);

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

/// Rows for branches that only a secondary clone has. A clone is an
/// independent repository, so a branch checked out there may not exist in the
/// primary at all. Without this those workspaces have no row to attach to and
/// vanish from the list while still being counted under Cleanup.
///
/// Only branches that are actually checked out get added. Merging every ref
/// from every clone would drag in that clone's stale origin/* snapshot and
/// resurrect hundreds of dead branches.
fn add_clone_only_branches(branches: &mut Vec<BranchInfo>, workspaces: &[Workspace]) {
    let mut extra: Vec<BranchInfo> = Vec::new();
    for workspace in workspaces {
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
}

/// What is on disk right now, for a project nothing has been shown for yet:
/// local refs and `workspace::list`, without fetching, asking GitHub or
/// running a `git status`. A fraction of a second where a full refresh takes
/// several, so the list has something in it while that runs and replaces it.
///
/// Statuses come back `None` with no `status_error` — not yet asked, as
/// opposed to unknown. The frontend shows no badge for that but still treats
/// it as at risk (`atRisk` in `src/status.ts`).
pub fn peek_overview(project: ProjectConfig) -> Result<Overview, String> {
    let path = PathBuf::from(&project.path);
    if !git::is_git_repo(&path) {
        return Err(format!("{} is no longer a git repository", project.path));
    }
    let base_branch = project
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&path));
    let slug = git::origin_url(&path).ok().and_then(|u| git::origin_slug(&u));

    let mut branches = git::branches(&path, &base_branch)?;
    let workspaces: Vec<Workspace> = workspace::list(&project)?
        .into_iter()
        .map(|entry| Workspace {
            path: entry.path.to_string_lossy().to_string(),
            kind: entry.kind,
            branch: entry.branch,
            owner: entry.owner,
            status: None,
            status_error: None,
        })
        .collect();
    add_clone_only_branches(&mut branches, &workspaces);

    Ok(Overview {
        project,
        base_branch,
        slug,
        branches,
        workspaces,
        pull_requests: HashMap::new(),
        viewer: None,
        warnings: Vec::new(),
    })
}

#[tauri::command]
async fn peek(state: State<'_, AppState>, id: String) -> Result<Overview, String> {
    let project = state.project(&id)?;
    let overview = tauri::async_runtime::spawn_blocking(move || peek_overview(project))
        .await
        .map_err(|e| e.to_string())??;
    state.terminals.reap();
    Ok(overview)
}

/// Open issues, for the New workspace dialog to offer as branches to start.
#[tauri::command]
async fn list_issues(state: State<'_, AppState>, id: String) -> Result<Vec<gh::Issue>, String> {
    let project = state.project(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(&project.path);
        let slug = git::origin_url(&path)
            .ok()
            .and_then(|u| git::origin_slug(&u))
            .ok_or("origin is not a GitHub repository")?;
        gh::open_issues(&path, &slug)
    })
    .await
    .map_err(|e| e.to_string())?
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
            open_targets: request.open_targets.clone(),
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
        // Kind and branch only, so the listing rather than a full scan: every
        // path is inspected below anyway, and only the chosen ones.
        let known = workspace::list(&project).unwrap_or_default();
        parallel_map(paths, |path| {
            let found = known.iter().find(|w| w.path.to_string_lossy() == path.as_str());
            let kind = found.map(|w| w.kind).unwrap_or(WorkspaceKind::Orphan);
            let branch = found.and_then(|w| w.branch.clone());
            workspace::removal_impact(&PathBuf::from(path), kind, branch, &base)
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
            let terminals = Arc::clone(&state.terminals);
            state.tasks.spawn_in_lane(
                &lane,
                task.clone(),
                Box::new(move |log| {
                    // A shell in the directory — or a dev server still writing
                    // into it — goes before the directory does.
                    if terminals.close(&item.path) {
                        log("Closed its terminal", false);
                    }
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

fn pty_size(cols: u16, rows: u16) -> portable_pty::PtySize {
    portable_pty::PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Attach to the workspace's terminal, starting a shell if it has none, at
/// home on `branch` of the project (`terminal::Home`). Output arrives on
/// `channel` as raw bytes, then `{ "exit": code }` once.
#[tauri::command]
fn terminal_open(
    state: State<AppState>,
    project_id: String,
    branch: String,
    path: String,
    cols: u16,
    rows: u16,
    channel: tauri::ipc::Channel,
) -> Result<terminal::Opened, String> {
    use tauri::ipc::InvokeResponseBody;
    let sink: terminal::Sink = Box::new(move |chunk| {
        let body = match chunk {
            terminal::Chunk::Data(bytes) => InvokeResponseBody::Raw(bytes),
            terminal::Chunk::Exit(code) => {
                InvokeResponseBody::Json(serde_json::json!({ "exit": code }).to_string())
            }
        };
        channel.send(body).is_ok()
    });
    let home = terminal::Home {
        kind: workspace::kind_at(&state.project(&project_id)?, Path::new(&path)),
        project_id,
        branch,
    };
    let command = terminal::Terminals::shell_command(&path);
    state
        .terminals
        .open(home, &path, pty_size(cols, rows), command, sink)
}

#[tauri::command]
fn terminal_write(state: State<AppState>, path: String, data: String) -> Result<u64, String> {
    state.terminals.write(&path, data.as_bytes())
}

#[tauri::command]
fn terminal_resize(state: State<AppState>, path: String, cols: u16, rows: u16) -> Result<(), String> {
    state.terminals.resize(&path, pty_size(cols, rows))
}

#[tauri::command]
fn terminal_close(state: State<AppState>, path: String) {
    state.terminals.close(&path);
}

/// Mark a terminal unread or blocked by hand, from its branch's menu.
#[tauri::command]
fn terminal_mark(state: State<AppState>, path: String, mark: terminal::Mark) -> Result<(), String> {
    state.terminals.mark(&path, mark)
}

/// What the Claude Code in a workspace's terminal last asked of the user,
/// from its transcript, or `None` when nothing is running there that keeps
/// one.
#[tauri::command]
fn terminal_question(state: State<AppState>, path: String) -> Option<String> {
    transcript::question(state.terminals.foreground(&path)?)
}

/// Running terminals, for a webview that has just (re)loaded to mark.
#[tauri::command]
fn terminal_list(state: State<AppState>) -> Vec<terminal::TerminalInfo> {
    state.terminals.list()
}

/// The terminal on screen, if any.
#[tauri::command]
fn terminal_viewing(state: State<AppState>, path: Option<String>) {
    state.terminals.view(path.as_deref());
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

/// A link clicked in a terminal: Quick Look for a file, the default browser
/// or mail client for the rest (`link::follow`).
#[tauri::command]
async fn open_link(url: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || link::follow(&url))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
/// Run an open target. A built-in terminal one isn't run here: its command
/// comes back, substituted, for the frontend to type into the workspace's
/// terminal.
fn open_in(
    state: State<AppState>,
    target_id: String,
    ctx: OpenContext,
) -> Result<Option<String>, String> {
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
    if target.kind == config::OpenKind::Terminal {
        return Ok(Some(script));
    }

    let cwd = PathBuf::from(&ctx.path);
    let cwd = if cwd.is_dir() { Some(cwd) } else { None };
    proc::spawn_shell_detached(&script, cwd.as_deref()).map(|()| None)
}

fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Badge the Dock icon with how many terminals wait for the user
/// (`Terminals::waiting`), from the backend so it holds while the webview
/// reloads and counts terminals never shown. Zero clears it: macOS would
/// draw `Some(0)` as a "0".
fn badge_waiting(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let waiting = state.terminals.waiting() as i64;
    if let Some(window) = app.webview_windows().into_values().next() {
        let _ = window.set_badge_count((waiting > 0).then_some(waiting));
    }
}

const SETTINGS_MENU_ID: &str = "settings";
const ADD_BRANCH_MENU_ID: &str = "add-branch";
const NEW_WORKSPACE_MENU_ID: &str = "new-workspace";

/// The File menu's two ways into the New workspace dialog, kept so the
/// webview can enable them as the selection changes.
struct CreateMenu {
    add_branch: MenuItem<tauri::Wry>,
    new_workspace: MenuItem<tauri::Wry>,
}

/// Tauri's stock menu with "Settings…" (⌘,) added to the application menu,
/// where macOS puts it: right under "About", between separators. "Settings"
/// rather than "Preferences" since macOS 13 renamed its own. The File menu
/// gets "Add Branch…" (⌘N) and "New Workspace…" (⌘⇧N) above "Close Window",
/// the same pair as the branch list's "+ Add branch" and a branch's
/// "+ New workspace"; both start disabled until the webview says otherwise.
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let items = menu.items()?;
    if let Some(MenuItemKind::Submenu(app_menu)) = items.first() {
        let settings =
            MenuItem::with_id(app, SETTINGS_MENU_ID, "Settings…", true, Some("CmdOrCtrl+,"))?;
        // After "About" and the separator under it.
        app_menu.insert_items(&[&settings, &PredefinedMenuItem::separator(app)?], 2)?;
    }
    let file_menu = items.iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) if submenu.text().ok().as_deref() == Some("File") => {
            Some(submenu)
        }
        _ => None,
    });
    if let Some(file_menu) = file_menu {
        let add_branch =
            MenuItem::with_id(app, ADD_BRANCH_MENU_ID, "Add Branch…", false, Some("CmdOrCtrl+N"))?;
        let new_workspace = MenuItem::with_id(
            app,
            NEW_WORKSPACE_MENU_ID,
            "New Workspace…",
            false,
            Some("CmdOrCtrl+Shift+N"),
        )?;
        file_menu.insert_items(
            &[&add_branch, &new_workspace, &PredefinedMenuItem::separator(app)?],
            0,
        )?;
        app.manage(CreateMenu { add_branch, new_workspace });
    }
    Ok(menu)
}

/// Enable the File menu's "Add Branch…" (a project is selected) and
/// "New Workspace…" (a branch is).
#[tauri::command]
fn set_create_menu(app: tauri::AppHandle, add_branch: bool, new_workspace: bool) {
    if let Some(menu) = app.try_state::<CreateMenu>() {
        let _ = menu.add_branch.set_enabled(add_branch);
        let _ = menu.new_workspace.set_enabled(new_workspace);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .menu(app_menu)
        .on_menu_event(|app, event| {
            if event.id() == SETTINGS_MENU_ID {
                let _ = app.emit("open-settings", ());
            } else if event.id() == ADD_BRANCH_MENU_ID {
                let _ = app.emit("open-create", false);
            } else if event.id() == NEW_WORKSPACE_MENU_ID {
                let _ = app.emit("open-create", true);
            }
        })
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let config = AppConfig::load(&dir);
            // One shell spawn; do it off the critical path so the window
            // isn't waiting on it.
            std::thread::spawn(|| {
                proc::login_path();
            });
            let handle = app.handle().clone();
            let terminal_handle = app.handle().clone();
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
                terminals: Arc::new(terminal::Terminals::new(Arc::new(move |event| {
                    let _ = match event {
                        terminal::TerminalEvent::Exit(info) => {
                            terminal_handle.emit("terminal-exit", &info.path)
                        }
                        terminal::TerminalEvent::Status(info) => {
                            terminal_handle.emit("terminal-status", info)
                        }
                    };
                    badge_waiting(&terminal_handle);
                }))),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            open_folder,
            are_folders,
            remove_project,
            select_project,
            refresh,
            peek,
            ahead_behind,
            list_issues,
            plan_workspace,
            create_workspace,
            removal_impacts,
            workspace_sizes,
            remove_workspaces,
            branch_impacts,
            delete_branches,
            open_in,
            open_link,
            copy_text,
            list_tasks,
            get_task,
            set_task_attention,
            set_create_menu,
            terminal_open,
            terminal_write,
            terminal_resize,
            terminal_close,
            terminal_list,
            terminal_viewing,
            terminal_mark,
            terminal_question,
        ])
        .run(tauri::generate_context!())
        .expect("error while running parallelist");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proc::run;

    /// A secondary clone's `origin/*` only moves when something fetches in
    /// that clone, and every status measured there is against it.
    #[test]
    fn a_fetching_refresh_fetches_secondary_clones_too() {
        let root = std::env::temp_dir().join(format!("parallelist-fetch-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let g = |cwd: &Path, args: &[&str]| {
            let out = run("git", args, Some(cwd)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
            out.stdout.trim().to_string()
        };

        let remote = root.join("origin.git");
        std::fs::create_dir_all(&remote).unwrap();
        g(&remote, &["init", "--quiet", "--bare", "-b", "main", "."]);
        let url = remote.to_string_lossy().to_string();
        // Siblings in one directory, so the scan finds the clone; the pusher
        // lives elsewhere so it doesn't.
        std::fs::create_dir_all(root.join("code")).unwrap();
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        let pusher = root.join("elsewhere/pusher");
        g(&root, &["clone", "--quiet", &url, &pusher.to_string_lossy()]);
        g(&pusher, &["config", "user.email", "test@example.com"]);
        g(&pusher, &["config", "user.name", "Test"]);
        g(&pusher, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
        g(&pusher, &["push", "--quiet", "-u", "origin", "main"]);

        let primary = root.join("code/repo");
        let clone = root.join("code/repo-b");
        g(&root, &["clone", "--quiet", &url, &primary.to_string_lossy()]);
        g(&root, &["clone", "--quiet", &url, &clone.to_string_lossy()]);

        g(&pusher, &["commit", "--quiet", "--allow-empty", "-m", "news"]);
        g(&pusher, &["push", "--quiet", "origin", "main"]);
        let news = g(&pusher, &["rev-parse", "HEAD"]);

        let project = ProjectConfig::new(primary.clone(), "t".into());
        let overview = build_overview(project, true, &|_, _, _| {}).expect("builds");
        // Under /var, which git reports as /private/var: the kinds are only
        // right if the paths are compared resolved.
        let kinds: Vec<_> = overview.workspaces.iter().map(|w| w.kind).collect();
        assert_eq!(kinds, [WorkspaceKind::Primary, WorkspaceKind::Clone]);
        let fetch_warnings: Vec<_> = overview.warnings.iter().filter(|w| w.contains("fetch")).collect();
        assert!(fetch_warnings.is_empty(), "{fetch_warnings:?}");

        assert_eq!(g(&clone, &["rev-parse", "origin/main"]), news, "the clone was not fetched");
        let behind = overview
            .workspaces
            .iter()
            .find(|w| w.kind == WorkspaceKind::Clone)
            .and_then(|w| w.status)
            .map(|s| s.behind);
        assert_eq!(behind, Some(1), "measured against the fresh snapshot");

        std::fs::remove_dir_all(&root).ok();
    }
}
