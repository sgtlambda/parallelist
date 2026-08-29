pub mod checkout;
pub mod config;
pub mod gh;
pub mod git;
pub mod proc;

use checkout::{BranchConflict, CreateRequest, RemovalImpact};
use config::{AppConfig, ProjectConfig};
use git::{AheadBehind, BranchInfo, Checkout, CheckoutKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    config: Mutex<AppConfig>,
    dir: PathBuf,
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
}

/// Emitted while a refresh is in flight so the window can say what it is
/// waiting on. A full refresh is several seconds of network and `git status`,
/// which is far too long to show nothing.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    project_id: String,
    /// Stable id; the frontend owns the wording.
    phase: &'static str,
    /// Optional extra, e.g. "12 of 16".
    detail: String,
    /// False when the phase has finished. Phases overlap, so the frontend
    /// tracks a set rather than a single current step.
    active: bool,
}

/// Anything that can be told what stage a refresh has reached.
pub type Report<'a> = &'a (dyn Fn(&'static str, String, bool) + Sync);

fn progress_reporter(
    app: AppHandle,
    project_id: String,
) -> impl Fn(&'static str, String, bool) + Sync {
    move |phase, detail, active| {
        let _ = app.emit(
            "progress",
            ProgressEvent {
                project_id: project_id.clone(),
                phase,
                detail,
                active,
            },
        );
    }
}

/// One refresh of everything the three panes need.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub project: ProjectConfig,
    pub base_branch: String,
    pub slug: Option<String>,
    pub branches: Vec<BranchInfo>,
    pub checkouts: Vec<Checkout>,
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

#[tauri::command]
async fn refresh(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    fetch: bool,
) -> Result<Overview, String> {
    let repo = state.project(&id)?;
    let report = progress_reporter(app, id);
    tauri::async_runtime::spawn_blocking(move || build_overview(repo, fetch, &report))
        .await
        .map_err(|e| e.to_string())?
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

    type GitSide = Result<(Vec<BranchInfo>, Vec<Checkout>, Vec<String>), String>;
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

            report("checkouts", String::new(), true);
            let checkouts = checkout::scan(&project, &base_branch, &remote_branches, &|done, total| {
                report("checkouts", format!("{done} of {total}"), true)
            })?;
            report("checkouts", String::new(), false);

            // A secondary clone is an independent repository, so a branch
            // checked out there may not exist in the primary at all. Without
            // this those checkouts have no row to attach to and vanish from the
            // list while still being counted under Cleanup.
            //
            // Only branches that are actually checked out get added. Merging
            // every ref from every clone would drag in that clone's stale
            // origin/* snapshot and resurrect hundreds of dead branches.
            let mut extra: Vec<BranchInfo> = Vec::new();
            for checkout in &checkouts {
                let Some(name) = checkout.branch.as_deref() else {
                    continue;
                };
                if branches.iter().any(|b| b.name == name) || extra.iter().any(|b| b.name == name) {
                    continue;
                }
                let Some((sha, when, author, subject)) =
                    git::head_commit(&PathBuf::from(&checkout.path))
                else {
                    continue;
                };
                extra.push(BranchInfo {
                    name: name.to_string(),
                    has_remote: false,
                    has_local: true,
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

            Ok((branches, checkouts, warnings))
        });

        (
            git.join().unwrap_or_else(|_| Err("git work panicked".into())),
            pulls.join().unwrap_or_else(|_| Err("gh work panicked".into())),
        )
    });

    let (branches, checkouts, mut warnings) = git_side?;
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
        checkouts,
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
fn plan_checkout(
    state: State<AppState>,
    id: String,
    branch: String,
    pr: Option<u64>,
) -> Result<CreatePlan, String> {
    let repo = state.project(&id)?;
    Ok(CreatePlan {
        suggested_path: checkout::suggest_path(&repo, &branch, pr),
        conflict: checkout::conflicting_worktree(&repo, &branch),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LogEvent {
    job_id: String,
    line: String,
    error: bool,
}

#[tauri::command]
async fn create_checkout(
    app: AppHandle,
    state: State<'_, AppState>,
    job_id: String,
    request: CreateRequest,
) -> Result<String, String> {
    let repo = state.project(&request.project_id)?;
    let base = repo
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&PathBuf::from(&repo.path)));

    tauri::async_runtime::spawn_blocking(move || {
        checkout::create(&repo, &base, &request, |line, error| {
            let _ = app.emit(
                "setup-log",
                LogEvent {
                    job_id: job_id.clone(),
                    line: line.to_string(),
                    error,
                },
            );
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn removal_impacts(
    state: State<'_, AppState>,
    id: String,
    paths: Vec<String>,
) -> Result<Vec<RemovalImpact>, String> {
    let repo = state.project(&id)?;
    let base = repo
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&PathBuf::from(&repo.path)));

    tauri::async_runtime::spawn_blocking(move || {
        let remote_branches = git::remote_branch_names(&PathBuf::from(&repo.path));
        let known = checkout::scan(&repo, &base, &remote_branches, &|_, _| {}).unwrap_or_default();
        paths
            .into_iter()
            .map(|p| {
                let found = known.iter().find(|c| c.path == p);
                let kind = found.map(|c| c.kind).unwrap_or(CheckoutKind::Orphan);
                let branch = found.and_then(|c| c.branch.clone());
                checkout::removal_impact(&PathBuf::from(&p), kind, branch)
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemovalRequest {
    path: String,
    kind: CheckoutKind,
    #[serde(default)]
    delete_branch: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemovalResult {
    path: String,
    error: Option<String>,
}

#[tauri::command]
async fn remove_checkouts(
    state: State<'_, AppState>,
    id: String,
    items: Vec<RemovalRequest>,
) -> Result<Vec<RemovalResult>, String> {
    let repo = state.project(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        items
            .into_iter()
            .map(|item| {
                // One failure shouldn't abandon the rest of a bulk cleanup, so
                // each result is reported independently.
                let error =
                    checkout::remove(&repo, &PathBuf::from(&item.path), item.kind, item.delete_branch)
                        .err();
                RemovalResult {
                    path: item.path,
                    error,
                }
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
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
            app.manage(AppState {
                config: Mutex::new(config),
                dir,
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
            plan_checkout,
            create_checkout,
            removal_impacts,
            remove_checkouts,
            open_in,
            copy_text,
        ])
        .run(tauri::generate_context!())
        .expect("error while running switcheroo");
}
