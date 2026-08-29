pub mod checkout;
pub mod config;
pub mod gh;
pub mod git;
pub mod proc;

use checkout::{BranchConflict, CreateRequest, RemovalImpact};
use config::{AppConfig, RepoConfig};
use git::{AheadBehind, BranchInfo, Checkout, CheckoutKind};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

    /// The stored repo with its `.switcheroo.json` overrides applied.
    fn repo(&self, id: &str) -> Result<RepoConfig, String> {
        self.config()
            .repo(id)
            .map(|r| r.resolved())
            .ok_or_else(|| format!("unknown repo '{id}'"))
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
    repo_id: String,
    /// Stable id; the frontend owns the wording.
    phase: &'static str,
    /// Optional extra, e.g. "12 of 16".
    detail: String,
}

/// Anything that can be told what stage a refresh has reached.
pub type Report<'a> = &'a (dyn Fn(&'static str, String) + Sync);

fn progress_reporter(app: AppHandle, repo_id: String) -> impl Fn(&'static str, String) + Sync {
    move |phase, detail| {
        let _ = app.emit(
            "progress",
            ProgressEvent {
                repo_id: repo_id.clone(),
                phase,
                detail,
            },
        );
    }
}

/// One refresh of everything the three panes need.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub repo: RepoConfig,
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
fn add_repo(state: State<AppState>, path: String) -> Result<RepoConfig, String> {
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
        .repos
        .iter()
        .find(|r| Path::new(&r.path) == path)
        .cloned()
    {
        return Ok(existing);
    }
    let repo = RepoConfig::new(path, name);
    cfg.repos.push(repo.clone());
    cfg.selected_repo_id = Some(repo.id.clone());
    state.persist(cfg)?;
    Ok(repo)
}

#[tauri::command]
fn remove_repo(state: State<AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config();
    cfg.repos.retain(|r| r.id != id);
    if cfg.selected_repo_id.as_deref() == Some(id.as_str()) {
        cfg.selected_repo_id = cfg.repos.first().map(|r| r.id.clone());
    }
    state.persist(cfg)
}

#[tauri::command]
fn select_repo(state: State<AppState>, id: Option<String>) -> Result<(), String> {
    let mut cfg = state.config();
    cfg.selected_repo_id = id;
    state.persist(cfg)
}

#[tauri::command]
async fn fetch_repo(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    let repo = state.repo(&id)?;
    let report = progress_reporter(app, id);
    tauri::async_runtime::spawn_blocking(move || {
        report("fetching", String::new());
        let result = git::fetch(&PathBuf::from(&repo.path));
        report("done", String::new());
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn overview(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<Overview, String> {
    let repo = state.repo(&id)?;
    let report = progress_reporter(app, id);
    tauri::async_runtime::spawn_blocking(move || build_overview(repo, &report))
        .await
        .map_err(|e| e.to_string())?
}

pub fn build_overview(repo: RepoConfig, report: Report) -> Result<Overview, String> {
    let path = PathBuf::from(&repo.path);
    if !git::is_git_repo(&path) {
        return Err(format!("{} is no longer a git repository", repo.path));
    }
    let mut warnings = Vec::new();

    let base_branch = repo
        .base_branch
        .clone()
        .unwrap_or_else(|| git::detect_base_branch(&path));

    report("branches", String::new());
    let branches = git::branches(&path, &base_branch)?;
    let remote_branches: HashSet<String> = branches
        .iter()
        .filter(|b| b.has_remote)
        .map(|b| b.name.clone())
        .collect();

    report("checkouts", String::new());
    let checkouts = checkout::scan(&repo, &base_branch, &remote_branches, &|done, total| {
        report("checkouts", format!("{done} of {total}"))
    })?;

    // A secondary clone is an independent repository, so a branch checked out
    // there may not exist in the primary at all — `app-b` on a branch the
    // primary has never seen. Without this those checkouts have no row to
    // attach to and disappear from the list entirely, while still being
    // counted under Cleanup.
    //
    // Only branches that are actually checked out get added. Merging every ref
    // from every clone would drag in each clone's stale `origin/*` copies and
    // resurrect hundreds of long-dead branches.
    let mut branches = branches;
    let mut extra: Vec<BranchInfo> = Vec::new();
    for checkout in &checkouts {
        let Some(name) = checkout.branch.as_deref() else {
            continue;
        };
        if branches.iter().any(|b| b.name == name)
            || extra.iter().any(|b| b.name == name)
        {
            continue;
        }
        let Some((sha, when, author, subject)) = git::head_commit(&PathBuf::from(&checkout.path))
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

    report("pullRequests", String::new());
    let slug = git::origin_url(&path).ok().and_then(|u| git::origin_slug(&u));
    let mut pull_requests = HashMap::new();
    let mut viewer = None;
    match (&slug, gh::is_available()) {
        (Some(slug), true) => match gh::pull_requests(&path, slug) {
            Ok(map) => {
                pull_requests = map;
                viewer = gh::viewer_login().map(str::to_string);
            }
            Err(e) => warnings.push(format!("Could not read pull requests: {e}")),
        },
        (Some(_), false) => {
            warnings.push("gh CLI not found — pull request info is unavailable.".into())
        }
        (None, _) => warnings.push("No GitHub remote detected — pull request info is off.".into()),
    }
    report("done", String::new());

    Ok(Overview {
        repo,
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
    let repo = state.repo(&id)?;
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
    let repo = state.repo(&id)?;
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
    let repo = state.repo(&request.repo_id)?;
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
    let repo = state.repo(&id)?;
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
    let repo = state.repo(&id)?;
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
    repo: String,
    #[serde(default)]
    pr_url: String,
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[tauri::command]
fn open_in(state: State<AppState>, target_id: String, ctx: OpenContext) -> Result<(), String> {
    let cfg = state.config();
    let target = cfg
        .open_targets
        .iter()
        .find(|t| t.id == target_id)
        .ok_or_else(|| format!("unknown open target '{target_id}'"))?;

    // Values are shell-quoted on substitution so a path with spaces (or worse)
    // can't break out of the user's template.
    let script = target
        .command
        .replace("{path}", &sh_quote(&ctx.path))
        .replace("{branch}", &sh_quote(&ctx.branch))
        .replace("{repo}", &sh_quote(&ctx.repo))
        .replace("{prUrl}", &sh_quote(&ctx.pr_url));

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
            add_repo,
            remove_repo,
            select_repo,
            fetch_repo,
            overview,
            ahead_behind,
            plan_checkout,
            create_checkout,
            removal_impacts,
            remove_checkouts,
            open_in,
        ])
        .run(tauri::generate_context!())
        .expect("error while running switcheroo");
}
