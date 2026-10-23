//! Discovering, creating and removing workspaces.
//!
//! A "workspace" is any directory on disk holding a branch of a repo: the
//! primary clone, a linked worktree, or a second full clone. The distinction
//! matters exactly twice — when creating one (git won't check a branch out in
//! two worktrees) and when removing one — so the rest of the app treats them
//! identically.

use crate::config::{ProjectConfig, SetupStep};
use crate::git::{self, Workspace, WorkspaceKind};
use crate::status::{self, Status};
use crate::proc::{run, run_shell_streaming};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::{Path, PathBuf};

/// Every directory beside the project that belongs to it: the clones (the main
/// workspace first, then siblings pointing at the same remote) and the orphans
/// — directories named like ours that are no longer git repos at all.
///
/// Read from `.git/config` rather than by spawning git, so scanning a folder
/// with a hundred projects in it stays instant.
fn siblings(project: &ProjectConfig) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    let primary = PathBuf::from(&project.path);
    let parent = primary
        .parent()
        .ok_or_else(|| "project has no parent directory".to_string())?
        .to_path_buf();
    let origin = git::origin_url(&primary)?;
    let dir_name = primary
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut clones = vec![primary.clone()];
    let mut orphans = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&parent) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || path == primary {
                continue;
            }
            match git::origin_url_from_config(&path) {
                Some(url) if git::same_remote(&url, &origin) => clones.push(path),
                Some(_) => {}
                None => {
                    // Not a clone. Only flag it if the name says it was once
                    // ours, so we never point at an unrelated project.
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if name.starts_with(&format!("{dir_name}-")) && !git::is_git_repo(&path) {
                        orphans.push(path);
                    }
                }
            }
        }
    }
    Ok((clones, orphans))
}

/// The project's other clones: everything `siblings` finds but the main
/// workspace itself.
pub fn secondary_clones(project: &ProjectConfig) -> Vec<PathBuf> {
    siblings(project)
        .map(|(clones, _)| clones.into_iter().skip(1).collect())
        .unwrap_or_default()
}

/// A path in the form git reports it.
///
/// git resolves symlinks — on macOS `/var/...` comes back as `/private/var/...`
/// — while the paths in `config.json` are however the user typed them. Comparing
/// the two raw is a false mismatch, which here would mean deciding a worktree is
/// still registered when it is not. The leaf is kept as written so a directory
/// that has already been deleted still resolves.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => match std::fs::canonicalize(parent) {
            Ok(real) => real.join(name),
            Err(_) => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// Which clone a worktree is registered against.
///
/// Worktrees of a *secondary* clone are invisible to the main workspace: git
/// keeps a registry per clone, so `git worktree remove` run in the main
/// workspace fails with "is not a working tree" and `git worktree prune` there
/// has nothing to prune. The registration then outlives the directory, the
/// next scan reads it straight back out of `git worktree list`, and the
/// workspace appears to be unremovable.
///
/// The `.git` pointer file answers this without spawning anything. Once the
/// directory is gone — exactly the state the bug above leaves behind — the
/// only way left is to ask each clone what it still has registered.
fn owning_clone(project: &ProjectConfig, path: &Path) -> PathBuf {
    let primary = PathBuf::from(&project.path);
    if path == primary {
        return primary;
    }

    // "gitdir: /Code/repo-clone/.git/worktrees/repo-feature" -> /Code/repo-clone
    if let Ok(text) = std::fs::read_to_string(path.join(".git")) {
        if let Some(dir) = text.strip_prefix("gitdir:").map(str::trim) {
            if let Some(root) = Path::new(dir)
                .ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == ".git"))
                .and_then(Path::parent)
            {
                return root.to_path_buf();
            }
        }
    }

    let Ok((clones, _)) = siblings(project) else {
        return primary;
    };
    let wanted = resolve(path);
    clones
        .into_iter()
        .find(|clone| {
            git::worktrees(clone)
                .unwrap_or_default()
                .iter()
                .any(|(p, _, _)| resolve(p) == wanted)
        })
        .unwrap_or(primary)
}

/// A workspace on disk, before anything has been asked of its working tree.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub kind: WorkspaceKind,
    pub owner: Option<String>,
}

/// Every workspace that belongs to this repo: each clone's worktrees (the
/// clone's own root among them), then the orphans. Cheap: one `git worktree
/// list` per clone and no `git status`.
pub fn list(project: &ProjectConfig) -> Result<Vec<Entry>, String> {
    let primary = PathBuf::from(&project.path);
    let (clones, orphans) = siblings(project)?;

    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = HashSet::new();

    // Resolved, because git reports paths with symlinks resolved and the
    // config has them as typed. See `resolve`.
    let main = resolve(&primary);
    for clone in &clones {
        let owner = clone.to_string_lossy().to_string();
        let root = resolve(clone);
        for (path, branch, _head) in git::worktrees(clone).unwrap_or_default() {
            if !seen.insert(path.clone()) {
                continue;
            }
            let is_root = resolve(&path) == root;
            let kind = if resolve(&path) == main {
                WorkspaceKind::Primary
            } else if is_root {
                WorkspaceKind::Clone
            } else {
                WorkspaceKind::Worktree
            };
            entries.push(Entry {
                path,
                branch,
                kind,
                owner: if is_root { None } else { Some(owner.clone()) },
            });
        }
    }

    for path in orphans {
        entries.push(Entry {
            path,
            branch: None,
            kind: WorkspaceKind::Orphan,
            owner: None,
        });
    }
    Ok(entries)
}

/// Every workspace with its status.
///
/// The per-workspace `git status` is the slow part — a few hundred
/// milliseconds each on a large working tree — so the workspaces are inspected
/// on a small pool of threads and `report` is called as each one lands.
pub fn scan(
    project: &ProjectConfig,
    base: &str,
    report: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Workspace>, String> {
    let entries = list(project)?;
    let total = entries.len();
    if total == 0 {
        return Ok(Vec::new());
    }

    let done = AtomicUsize::new(0);
    let workers = total.min(8);
    let chunk_size = total.div_ceil(workers);

    let mut out: Vec<Workspace> = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .map(|entry| {
                            // An orphan has no git dir, so there is nothing to
                            // ask git about and no process worth spawning.
                            let (status, status_error) = if entry.kind == WorkspaceKind::Orphan {
                                (None, None)
                            } else {
                                match status::inspect(&entry.path, base) {
                                    Ok(s) => (Some(s), None),
                                    Err(e) => (None, Some(e)),
                                }
                            };
                            report(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                            Workspace {
                                path: entry.path.to_string_lossy().to_string(),
                                kind: entry.kind,
                                branch: entry.branch.clone(),
                                owner: entry.owner.clone(),
                                status,
                                status_error,
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();

        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Branch name to directory slug: `fix/login-redirect` -> `login-redirect`,
/// prefixed with the PR number when there is one.
pub fn slugify(branch: &str, pr: Option<u64>) -> String {
    let tail = branch.rsplit('/').next().unwrap_or(branch);
    let mut slug = String::new();
    let mut last_dash = true;
    for ch in tail.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    match pr {
        Some(n) => format!("{n}-{slug}"),
        None => slug,
    }
}

/// Fill in a repo's path template, then walk it out of the way of anything
/// that already exists using the `-b`, `-c` suffixes.
pub fn suggest_path(project: &ProjectConfig, branch: &str, pr: Option<u64>) -> String {
    let primary = PathBuf::from(&project.path);
    let parent = primary
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let project_dir = primary
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| project.name.clone());

    let base = project
        .path_template
        .replace("{parent}", &parent)
        .replace("{project}", &project_dir)
        .replace("{slug}", &slugify(branch, pr))
        .replace("{branch}", branch)
        .replace("{pr}", &pr.map(|n| n.to_string()).unwrap_or_default());

    if !Path::new(&base).exists() {
        return base;
    }
    for suffix in ["b", "c", "d", "e", "f", "g"] {
        let candidate = format!("{base}-{suffix}");
        if !Path::new(&candidate).exists() {
            return candidate;
        }
    }
    format!("{base}-{}", std::process::id())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    pub project_id: String,
    pub branch: String,
    pub path: String,
    /// `worktree` or `clone`. Anything else is rejected.
    pub mode: String,
    /// Create the branch rather than checking out an existing one.
    #[serde(default)]
    pub new_branch: bool,
    /// Start point for a new branch; defaults to the repo's base branch.
    #[serde(default)]
    pub from: Option<String>,
    /// Ids of the setup steps the user ticked.
    #[serde(default)]
    pub steps: Vec<String>,
    /// Ids of the "open in" targets to run once it is set up. Only carried
    /// on the task: the webview runs them.
    #[serde(default)]
    pub open_targets: Vec<String>,
}

/// Why a worktree isn't possible for this branch, if it isn't.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchConflict {
    pub branch: String,
    /// The workspace already holding it.
    pub path: String,
}

/// Git refuses to check one branch out in two worktrees of the same clone.
/// Finding the existing one lets the create dialog say so up front and offer
/// a clone instead, rather than failing after the user hits create.
pub fn conflicting_worktree(project: &ProjectConfig, branch: &str) -> Option<BranchConflict> {
    let primary = PathBuf::from(&project.path);
    git::worktrees(&primary)
        .ok()?
        .into_iter()
        .find(|(_, b, _)| b.as_deref() == Some(branch))
        .map(|(path, _, _)| BranchConflict {
            branch: branch.to_string(),
            path: path.to_string_lossy().to_string(),
        })
}

pub fn log_step<F: FnMut(&str, bool) + ?Sized>(log: &mut F, msg: &str) {
    log(&format!("\u{25b8} {msg}"), false);
}

/// Create the workspace and run the selected setup steps, streaming everything
/// through `log`. Returns the created path.
pub fn create<F: FnMut(&str, bool)>(
    project: &ProjectConfig,
    base: &str,
    req: &CreateRequest,
    mut log: F,
) -> Result<String, String> {
    let primary = PathBuf::from(&project.path);
    let target = PathBuf::from(&req.path);
    if target.exists() {
        return Err(format!("{} already exists", req.path));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let made = match req.mode.as_str() {
        "worktree" => create_worktree(&primary, base, req, &mut log),
        "clone" => create_clone(&primary, req, &mut log),
        other => return Err(format!("unknown workspace mode '{other}'")),
    };
    if let Err(e) = made {
        abandon(&primary, &target, &req.mode, &mut log);
        return Err(e);
    }

    run_setup_steps(project, &primary, &target, &req.branch, &req.steps, &mut log);
    log_step(&mut log, "Done");
    Ok(req.path.clone())
}

/// Take away whatever a failed create left at `target`.
///
/// The directory is ours: `create` refuses a path that already exists. Left
/// behind, a half-made clone — `.git` and nothing else, never checked out —
/// shows up as a workspace with every file deleted, and it occupies the
/// suggested path, so the retry lands on `-b`, then `-c`. That trail was found
/// on disk, three clones deep.
fn abandon<F: FnMut(&str, bool)>(primary: &Path, target: &Path, mode: &str, log: &mut F) {
    if !target.exists() {
        return;
    }
    log_step(log, "Removing the unfinished workspace");
    if mode == "worktree" {
        let path = target.to_string_lossy();
        let removed = run("git", &["worktree", "remove", "--force", &path], Some(primary))
            .map(|o| o.ok())
            .unwrap_or(false);
        if removed {
            return;
        }
    }
    if let Err(e) = std::fs::remove_dir_all(target) {
        log(&format!("could not delete {}: {e}", target.display()), true);
    }
    if mode == "worktree" {
        let _ = run("git", &["worktree", "prune"], Some(primary));
    }
}

fn create_worktree<F: FnMut(&str, bool)>(
    primary: &Path,
    base: &str,
    req: &CreateRequest,
    log: &mut F,
) -> Result<(), String> {
    let branch = &req.branch;
    let local_exists = run(
        "git",
        &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch}")],
        Some(primary),
    )?
    .ok();
    let remote_ref = format!("origin/{branch}");
    let remote_exists = run(
        "git",
        &["show-ref", "--verify", "--quiet", &format!("refs/remotes/{remote_ref}")],
        Some(primary),
    )?
    .ok();

    let start = req.from.clone().unwrap_or_else(|| format!("origin/{base}"));
    let args: Vec<String> = if req.new_branch || (!local_exists && !remote_exists) {
        // A brand new branch. --no-track deliberately: without it git sets the
        // upstream to the start point, and a later bare `git push` lands on
        // the branch it was cut from.
        log_step(log, &format!("Creating branch {branch} from {start}"));
        vec![
            "worktree".into(),
            "add".into(),
            req.path.clone(),
            "-b".into(),
            branch.clone(),
            "--no-track".into(),
            start,
        ]
    } else if local_exists {
        log_step(log, &format!("Adding worktree for local branch {branch}"));
        vec!["worktree".into(), "add".into(), req.path.clone(), branch.clone()]
    } else {
        // Remote-only branch: create the local branch tracking its own remote
        // counterpart, which is the upstream you actually want here.
        log_step(log, &format!("Adding worktree tracking {remote_ref}"));
        vec![
            "worktree".into(),
            "add".into(),
            "--track".into(),
            "-b".into(),
            branch.clone(),
            req.path.clone(),
            remote_ref,
        ]
    };

    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = run("git", &argv, Some(primary))?;
    for line in out.stderr.lines().chain(out.stdout.lines()) {
        log(line, !out.ok());
    }
    if !out.ok() {
        return Err(format!("git worktree add failed: {}", out.stderr.trim()));
    }
    Ok(())
}

fn create_clone<F: FnMut(&str, bool)>(
    primary: &Path,
    req: &CreateRequest,
    log: &mut F,
) -> Result<(), String> {
    let origin = git::origin_url(primary)?;
    let primary_str = primary.to_string_lossy().to_string();

    // Clone from the primary with --local: git hardlinks the object store, so
    // this costs almost nothing on disk and finishes in seconds even on a big
    // repo. Origin is then pointed back at the real remote so pushing works.
    log_step(log, "Cloning from the main workspace (hardlinked objects)");
    let out = run(
        "git",
        &["clone", "--local", "--no-checkout", &primary_str, &req.path],
        None,
    )?;
    for line in out.stderr.lines() {
        log(line, !out.ok());
    }
    if !out.ok() {
        return Err(format!("git clone failed: {}", out.stderr.trim()));
    }

    let target = PathBuf::from(&req.path);
    let branch = &req.branch;

    // A branch that exists only in the main workspace. Straight after the
    // clone it is visible here as `origin/<branch>` — the main workspace was
    // the origin — but repointing origin and fetching with --prune deletes
    // that ref, and with it the only way to reach the branch. Take it now.
    // No upstream: there is nothing on origin to track.
    let local_only = !req.new_branch && !has_ref(primary, &format!("refs/remotes/origin/{branch}"))?
        && has_ref(primary, &format!("refs/heads/{branch}"))?;
    if local_only {
        log_step(log, &format!("Taking {branch} from the main workspace"));
        let start = format!("refs/remotes/origin/{branch}");
        let out = run("git", &["branch", "--no-track", branch, &start], Some(&target))?;
        log_output(log, &out);
        if !out.ok() {
            return Err(format!("could not take {branch} from the main workspace: {}", out.stderr.trim()));
        }
    }

    log_step(log, &format!("Pointing origin at {origin}"));
    run("git", &["remote", "set-url", "origin", &origin], Some(&target))?
        .text()?;
    run("git", &["fetch", "origin", "--prune", "--quiet"], Some(&target))?.text()?;

    log_step(log, &format!("Checking out {branch}"));
    let remote_ref = format!("origin/{branch}");
    let workspace = if local_only {
        run("git", &["checkout", branch], Some(&target))?
    } else if req.new_branch {
        let start = req.from.clone().unwrap_or_else(|| "HEAD".to_string());
        run(
            "git",
            &["checkout", "--no-track", "-b", branch, &start],
            Some(&target),
        )?
    } else {
        run(
            "git",
            &["checkout", "-B", branch, "--track", &remote_ref],
            Some(&target),
        )?
    };
    for line in workspace.stderr.lines() {
        log(line, !workspace.ok());
    }
    if !workspace.ok() {
        return Err(format!("workspace failed: {}", workspace.stderr.trim()));
    }
    Ok(())
}

fn has_ref(repo: &Path, refname: &str) -> Result<bool, String> {
    Ok(run("git", &["show-ref", "--verify", "--quiet", refname], Some(repo))?.ok())
}

fn run_setup_steps<F: FnMut(&str, bool)>(
    project: &ProjectConfig,
    primary: &Path,
    target: &Path,
    branch: &str,
    enabled: &[String],
    log: &mut F,
) {
    let steps: Vec<&SetupStep> = project
        .setup_steps
        .iter()
        .filter(|s| enabled.contains(&s.id))
        .collect();

    for step in steps {
        if step.script.trim().is_empty() {
            continue;
        }
        log_step(log, &step.label);
        // Values are shell-quoted on substitution, so a path with spaces
        // cannot break out of the user's command.
        let script = step
            .script
            .replace("{primary}", &crate::proc::sh_quote(&primary.to_string_lossy()))
            .replace("{path}", &crate::proc::sh_quote(&target.to_string_lossy()))
            .replace("{branch}", &crate::proc::sh_quote(branch));

        match run_shell_streaming(&script, target, |line, is_err| {
            log(&format!("  {line}"), is_err)
        }) {
            Ok(0) => {}
            Ok(code) => log(&format!("  exited with status {code}"), true),
            Err(e) => log(&format!("  {e}"), true),
        }
    }
}

/// What removing a workspace would destroy. Shown in the confirm dialog; it
/// never blocks the removal, it just makes sure the cost is visible.
///
/// Counted per kind, because the kinds lose different things. A worktree's
/// branch, stashes and other refs live in the clone it hangs off and outlive
/// it; a clone takes all of its own with it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalImpact {
    pub path: String,
    pub kind: WorkspaceKind,
    pub branch: Option<String>,
    /// The working tree, as the workspace card shows it. `None` for an orphan
    /// or when git couldn't be asked.
    pub status: Option<Status>,
    /// Why some of this couldn't be measured. Set means "unknown", not "safe".
    pub error: Option<String>,
    /// Commits that exist nowhere else once this is gone: not on origin in
    /// any form, and on no other local branch that survives the removal.
    pub commits: u32,
    /// A worktree on a branch only loses `commits` if that branch is deleted
    /// along with it. Everything else loses them regardless.
    pub commits_need_branch_delete: bool,
    /// Stashes that go with it. Always 0 for a worktree: `refs/stash` belongs
    /// to the clone, and stashing from a worktree puts it there.
    pub stashes: u32,
    /// Worktrees registered against a clone. Its removal strands them: their
    /// `.git` points into the deleted directory.
    pub dependents: Vec<String>,
}

pub fn removal_impact(
    path: &Path,
    kind: WorkspaceKind,
    branch: Option<String>,
    base: &str,
) -> RemovalImpact {
    let mut impact = RemovalImpact {
        path: path.to_string_lossy().to_string(),
        kind,
        branch: branch.clone(),
        status: None,
        error: None,
        commits: 0,
        commits_need_branch_delete: kind == WorkspaceKind::Worktree && branch.is_some(),
        stashes: 0,
        dependents: Vec::new(),
    };
    if kind == WorkspaceKind::Orphan {
        return impact;
    }
    let measured = (|| -> Result<(), String> {
        impact.status = Some(status::inspect(path, base)?);
        match kind {
            WorkspaceKind::Clone => {
                impact.commits = status::unpushed_anywhere(path, base)?.len() as u32;
                impact.stashes = status::stash_count(path)?;
                let own = resolve(path);
                impact.dependents = git::worktrees(path)?
                    .into_iter()
                    .map(|(p, _, _)| p)
                    .filter(|p| resolve(p) != own)
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
            }
            _ => {
                let upstream = branch.as_deref().and_then(|b| status::origin_upstream(path, b));
                let unpushed = status::unpushed(path, "HEAD", upstream.as_deref(), base)?;
                if !unpushed.is_empty() {
                    let lost = status::held_only_by(path, "HEAD", branch.as_deref())?;
                    impact.commits = unpushed.intersection(&lost).count() as u32;
                }
            }
        }
        Ok(())
    })();
    if let Err(e) = measured {
        impact.error = Some(e);
    }
    impact
}

/// `du -sk` rather than a recursive walk: node_modules makes the walk slow,
/// and du already knows how to not double-count hardlinks.
///
/// Still slow in absolute terms — ~5s on a full clone with vendor and
/// node_modules — which is why it is measured separately from the rest of the
/// removal impact rather than holding up the list.
pub fn dir_size(path: &Path) -> u64 {
    run("du", &["-sk", &path.to_string_lossy()], None)
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| {
            o.stdout
                .split_whitespace()
                .next()
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

pub fn remove<F: FnMut(&str, bool)>(
    project: &ProjectConfig,
    path: &Path,
    kind: WorkspaceKind,
    delete_branch: Option<String>,
    mut log: F,
) -> Result<(), String> {
    let primary = PathBuf::from(&project.path);
    if kind == WorkspaceKind::Primary {
        return Err("refusing to remove the main workspace".into());
    }
    let path_str = path.to_string_lossy().to_string();
    // Not `primary`: a worktree of a secondary clone is registered there and
    // nowhere else, and git commands aimed at the wrong clone quietly do
    // nothing. See `owning_clone`.
    let owner = owning_clone(project, path);

    match kind {
        WorkspaceKind::Worktree => {
            log_step(&mut log, "Removing the worktree");
            // --force because the impact dialog has already shown the user
            // exactly what is in there.
            let out = run(
                "git",
                &["worktree", "remove", "--force", &path_str],
                Some(&owner),
            )?;
            log_output(&mut log, &out);
            if !out.ok() {
                // The registration can outlive the directory; prune and retry
                // the plain delete rather than leaving a half-removed entry.
                log_step(&mut log, "Deleting the directory and pruning the worktree");
                if path.exists() {
                    std::fs::remove_dir_all(path).map_err(|e| format!("could not delete: {e}"))?;
                }
                let out = run("git", &["worktree", "prune"], Some(&owner))?;
                log_output(&mut log, &out);
                // Deleting the directory and swallowing the rest used to report
                // success while the registration survived, so the workspace came
                // back on the next scan and the task log claimed it had gone.
                // A surviving entry is a failure and has to say so.
                if git::worktrees(&owner)
                    .unwrap_or_default()
                    .iter()
                    .any(|(p, _, _)| resolve(p) == resolve(path))
                {
                    return Err(format!(
                        "{path_str} is still registered as a worktree of {}",
                        owner.to_string_lossy()
                    ));
                }
            }
        }
        WorkspaceKind::Clone | WorkspaceKind::Orphan => {
            log_step(&mut log, "Deleting the directory");
            std::fs::remove_dir_all(path).map_err(|e| format!("could not delete: {e}"))?;
        }
        WorkspaceKind::Primary => unreachable!(),
    }

    // The owner holds the branch and the registry we just changed — unless the
    // owner *was* what we deleted (removing a whole clone), in which case the
    // main workspace is the only one left to tidy.
    let home = if owner.is_dir() { &owner } else { &primary };
    if let Some(branch) = delete_branch {
        log_step(&mut log, &format!("Deleting branch {branch}"));
        let out = run("git", &["branch", "-D", &branch], Some(home))?;
        log_output(&mut log, &out);
    }
    run("git", &["worktree", "prune"], Some(home))?;
    log_step(&mut log, "Done");
    Ok(())
}

/// Everything a finished git command printed, flagged as errors if it failed.
fn log_output<F: FnMut(&str, bool)>(log: &mut F, out: &crate::proc::Output) {
    for line in out.stderr.lines().chain(out.stdout.lines()) {
        if !line.trim().is_empty() {
            log(line, !out.ok());
        }
    }
}

/// Delete a local branch from the primary clone.
///
/// `-D` rather than `-d`, for the same reason the worktree removal forces:
/// the confirm dialog has already listed the commits only this branch holds,
/// which is a sharper answer than git's own merge check. A pre-rebase backup
/// is "unmerged" by that check while holding nothing origin doesn't have.
///
/// The base branch is refused here rather than only in the UI. Nothing offers
/// it — it has a remote, so it never looks like a leftover — but it is the one
/// deletion that would take a re-clone to notice.
pub fn delete_branch(primary: &Path, base: &str, branch: &str) -> Result<(), String> {
    if branch == base {
        return Err(format!("refusing to delete the base branch '{base}'"));
    }
    let out = run("git", &["branch", "-D", branch], Some(primary))?;
    if !out.ok() {
        // git's own message already names the worktree still holding it.
        return Err(out.stderr.trim().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repo on `main` with one extra branch, and no origin: `delete_branch`
    /// only ever touches local refs.
    fn repo_with_branch(dir: &Path, branch: &str) {
        let g = |args: &[&str]| {
            let out = run("git", args, Some(dir)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
        };
        g(&["init", "--quiet", "-b", "main", "."]);
        g(&["config", "user.email", "test@example.com"]);
        g(&["config", "user.name", "Test"]);
        std::fs::write(dir.join("a"), "a").unwrap();
        g(&["add", "-A"]);
        g(&["commit", "--quiet", "-m", "base"]);
        g(&["branch", branch]);
    }

    #[test]
    fn deletes_a_branch_but_never_the_base_or_a_checked_out_one() {
        let dir = std::env::temp_dir().join(format!("parallelist-del-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        repo_with_branch(&dir, "leftover");

        // The base branch is refused before git is even asked.
        let err = delete_branch(&dir, "main", "main").unwrap_err();
        assert!(err.contains("refusing"), "{err}");

        // A checked-out branch is refused by git, and its message — "used by
        // worktree at <path>", which names what still holds it — is what
        // surfaces to the dialog.
        let checkout = |branch: &str| {
            assert!(run("git", &["checkout", "--quiet", branch], Some(&dir))
                .unwrap()
                .ok());
        };
        checkout("leftover");
        let err = delete_branch(&dir, "main", "leftover").unwrap_err();
        assert!(err.contains("worktree at"), "{err}");
        checkout("main");

        delete_branch(&dir, "main", "leftover").expect("deletes a leftover");
        let refs = run("git", &["branch", "--format=%(refname:short)"], Some(&dir))
            .unwrap()
            .stdout;
        assert!(!refs.contains("leftover"), "still there: {refs}");

        // Gone already: git's error is surfaced rather than swallowed.
        assert!(delete_branch(&dir, "main", "leftover").is_err());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn removes_a_worktree_that_hangs_off_a_secondary_clone() {
        let root = std::env::temp_dir().join(format!("parallelist-own-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();

        let g = |args: &[&str], cwd: &Path| {
            let out = run("git", args, Some(cwd)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
        };

        // A bare "remote" so the two clones look related to `siblings`.
        let remote = root.join("origin.git");
        std::fs::create_dir_all(&remote).unwrap();
        g(&["init", "--quiet", "--bare", "-b", "main", "."], &remote);

        let primary = root.join("repo");
        std::fs::create_dir_all(&primary).unwrap();
        repo_with_branch(&primary, "feature");
        let url = remote.to_string_lossy().to_string();
        g(&["remote", "add", "origin", &url], &primary);

        // The secondary clone, which is where the worktree will live.
        let clone = root.join("repo-clone");
        g(
            &["clone", "--quiet", "--local", &primary.to_string_lossy(), &clone.to_string_lossy()],
            &root,
        );
        g(&["remote", "set-url", "origin", &url], &clone);

        let worktree = root.join("repo-feature");
        g(
            &["worktree", "add", "--quiet", &worktree.to_string_lossy(), "feature"],
            &clone,
        );

        let project = ProjectConfig {
            id: "t".into(),
            name: "t".into(),
            path: primary.to_string_lossy().to_string(),
            base_branch: Some("main".into()),
            path_template: "{parent}/{project}-{slug}".into(),
            setup_steps: Vec::new(),
            open_targets: Vec::new(),
        };

        // The main workspace has never heard of this worktree, so aiming git
        // there is the bug: it fails, and pruning there prunes nothing.
        assert_eq!(resolve(&owning_clone(&project, &worktree)), resolve(&clone));

        remove(&project, &worktree, WorkspaceKind::Worktree, None, |_, _| {})
            .expect("removes the worktree");

        assert!(!worktree.exists(), "directory survived");
        let registered = git::worktrees(&clone).unwrap();
        assert!(
            !registered.iter().any(|(p, _, _)| resolve(p) == resolve(&worktree)),
            "registration survived, so the workspace comes back: {registered:?}"
        );

        // And again with the directory already gone, which is the state the
        // old code left behind: the path still resolves to its clone.
        let stale = root.join("repo-stale");
        g(
            &["worktree", "add", "--quiet", &stale.to_string_lossy(), "-b", "stale"],
            &clone,
        );
        std::fs::remove_dir_all(&stale).unwrap();
        assert_eq!(resolve(&owning_clone(&project, &stale)), resolve(&clone));
        remove(&project, &stale, WorkspaceKind::Worktree, None, |_, _| {})
            .expect("clears a registration whose directory is gone");
        assert!(
            !git::worktrees(&clone).unwrap().iter().any(|(p, _, _)| resolve(p) == resolve(&stale)),
            "stale registration survived"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    /// Each kind loses different things: a worktree's stash and branch live
    /// in its clone, a clone takes everything of its own with it.
    #[test]
    fn counts_what_each_kind_of_removal_actually_loses() {
        let root = std::env::temp_dir().join(format!("parallelist-impact-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let g = |cwd: &Path, args: &[&str]| {
            let out = run("git", args, Some(cwd)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
        };
        let commit = |cwd: &Path, name: &str| {
            std::fs::write(cwd.join(name), name).unwrap();
            g(cwd, &["add", "-A"]);
            g(cwd, &["commit", "--quiet", "-m", name]);
        };

        let remote = root.join("origin.git");
        std::fs::create_dir_all(&remote).unwrap();
        g(&remote, &["init", "--quiet", "--bare", "-b", "main", "."]);
        let primary = root.join("repo");
        g(&root, &["clone", "--quiet", &remote.to_string_lossy(), "repo"]);
        g(&primary, &["config", "user.email", "test@example.com"]);
        g(&primary, &["config", "user.name", "Test"]);
        commit(&primary, "base");
        g(&primary, &["push", "--quiet", "-u", "origin", "main"]);

        // A worktree on a never-pushed branch with two commits and a stash.
        let worktree = root.join("repo-feature");
        g(&primary, &["worktree", "add", "--quiet", "--no-track", "-b", "feature", &worktree.to_string_lossy(), "origin/main"]);
        commit(&worktree, "one");
        commit(&worktree, "two");
        std::fs::write(worktree.join("one"), "stashed").unwrap();
        g(&worktree, &["stash", "--quiet"]);

        let impact = removal_impact(&worktree, WorkspaceKind::Worktree, Some("feature".into()), "main");
        assert_eq!(impact.error, None);
        assert_eq!(impact.commits, 2);
        assert!(impact.commits_need_branch_delete, "the branch keeps them");
        assert_eq!(impact.stashes, 0, "the stash lives in the clone and survives");

        // A second branch holding the older commit: only the newer is lost.
        g(&primary, &["branch", "backup", "feature~1"]);
        let impact = removal_impact(&worktree, WorkspaceKind::Worktree, Some("feature".into()), "main");
        assert_eq!(impact.commits, 1);

        // Detached, the commits are lost whatever happens to branches.
        g(&worktree, &["checkout", "--quiet", "--detach"]);
        commit(&worktree, "three");
        let impact = removal_impact(&worktree, WorkspaceKind::Worktree, None, "main");
        assert_eq!(impact.commits, 1);
        assert!(!impact.commits_need_branch_delete);

        // A clone: its other branches, its stash and its worktrees go too.
        let clone = root.join("repo-b");
        g(&root, &["clone", "--quiet", &remote.to_string_lossy(), "repo-b"]);
        g(&clone, &["config", "user.email", "test@example.com"]);
        g(&clone, &["config", "user.name", "Test"]);
        g(&clone, &["checkout", "--quiet", "--no-track", "-b", "side"]);
        commit(&clone, "side-work");
        g(&clone, &["checkout", "--quiet", "main"]);
        commit(&clone, "main-work");
        std::fs::write(clone.join("base"), "stashed").unwrap();
        g(&clone, &["stash", "--quiet"]);
        let hanging = root.join("repo-b-wt");
        g(&clone, &["worktree", "add", "--quiet", "--detach", &hanging.to_string_lossy()]);

        let impact = removal_impact(&clone, WorkspaceKind::Clone, Some("main".into()), "main");
        assert_eq!(impact.error, None);
        assert_eq!(impact.commits, 2, "main-work and side-work");
        assert_eq!(impact.stashes, 1);
        assert_eq!(impact.dependents.len(), 1, "{:?}", impact.dependents);

        // Unknown is not clean.
        let gone = removal_impact(&root.join("missing"), WorkspaceKind::Worktree, None, "main");
        assert!(gone.error.is_some());

        std::fs::remove_dir_all(&root).ok();
    }

    /// A main workspace with origin, a pushed `main`, and `local-only`: a
    /// branch with a commit of its own that was never pushed.
    fn project_with_local_only_branch(root: &Path) -> ProjectConfig {
        let g = |cwd: &Path, args: &[&str]| {
            let out = run("git", args, Some(cwd)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
        };
        std::fs::remove_dir_all(root).ok();
        std::fs::create_dir_all(root.join("origin.git")).unwrap();
        g(&root.join("origin.git"), &["init", "--quiet", "--bare", "-b", "main", "."]);
        let primary = root.join("repo");
        g(root, &["clone", "--quiet", &root.join("origin.git").to_string_lossy(), "repo"]);
        g(&primary, &["config", "user.email", "test@example.com"]);
        g(&primary, &["config", "user.name", "Test"]);
        std::fs::write(primary.join("a"), "a").unwrap();
        g(&primary, &["add", "-A"]);
        g(&primary, &["commit", "--quiet", "-m", "base"]);
        g(&primary, &["push", "--quiet", "-u", "origin", "main"]);
        g(&primary, &["checkout", "--quiet", "--no-track", "-b", "local-only"]);
        std::fs::write(primary.join("b"), "b").unwrap();
        g(&primary, &["add", "-A"]);
        g(&primary, &["commit", "--quiet", "-m", "never pushed"]);
        g(&primary, &["checkout", "--quiet", "main"]);
        ProjectConfig {
            id: "t".into(),
            name: "t".into(),
            path: primary.to_string_lossy().to_string(),
            base_branch: Some("main".into()),
            path_template: "{parent}/{project}-{slug}".into(),
            setup_steps: Vec::new(),
            open_targets: Vec::new(),
        }
    }

    fn request(root: &Path, dir: &str, branch: &str, mode: &str) -> CreateRequest {
        CreateRequest {
            project_id: "t".into(),
            branch: branch.into(),
            path: root.join(dir).to_string_lossy().to_string(),
            mode: mode.into(),
            new_branch: false,
            from: None,
            steps: Vec::new(),
            open_targets: Vec::new(),
        }
    }

    /// Repointing origin and pruning used to delete the only ref the clone
    /// had to such a branch, so the checkout failed and left `.git` alone in
    /// the directory.
    #[test]
    fn clones_a_branch_that_was_never_pushed() {
        let root = std::env::temp_dir().join(format!("parallelist-localclone-{}", std::process::id()));
        let project = project_with_local_only_branch(&root);
        let req = request(&root, "repo-b", "local-only", "clone");

        create(&project, "main", &req, |_, _| {}).expect("creates the clone");
        let clone = root.join("repo-b");
        assert!(clone.join("b").exists(), "the branch's own commit is checked out");
        let current = run("git", &["branch", "--show-current"], Some(&clone)).unwrap().stdout;
        assert_eq!(current.trim(), "local-only");
        let status = status::inspect(&clone, "main").unwrap();
        assert_eq!((status.modified, status.unpushed), (0, 1), "{status:?}");

        std::fs::remove_dir_all(&root).ok();
    }

    /// Whatever fails, the directory goes with it: left behind it reads as a
    /// workspace with every file deleted and takes the path from the retry.
    #[test]
    fn a_failed_create_leaves_nothing_behind() {
        let root = std::env::temp_dir().join(format!("parallelist-failcreate-{}", std::process::id()));
        let project = project_with_local_only_branch(&root);

        // A clone of a branch that exists nowhere fails after `git clone`
        // has already made the directory.
        let req = request(&root, "repo-nope", "no-such-branch", "clone");
        let mut log = Vec::new();
        assert!(create(&project, "main", &req, |line, _| log.push(line.to_string())).is_err());
        assert!(!root.join("repo-nope").exists(), "left behind; log: {log:?}");

        // A worktree of the branch the main workspace already has.
        let req = request(&root, "repo-main", "main", "worktree");
        assert!(create(&project, "main", &req, |_, _| {}).is_err());
        assert!(!root.join("repo-main").exists());
        let registered = git::worktrees(&root.join("repo")).unwrap();
        assert_eq!(registered.len(), 1, "{registered:?}");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn slugifies_branch_names() {
        assert_eq!(slugify("fix/login-redirect", None), "login-redirect");
        // The PR number leads, which is how these directories get named.
        assert_eq!(slugify("fix/login-redirect", Some(1042)), "1042-login-redirect");
        // Only the last segment, lowercased, with runs of punctuation collapsed.
        assert_eq!(slugify("feature/optional-API-keys", None), "optional-api-keys");
        assert_eq!(slugify("release/v1.2.3", None), "v1-2-3");
        assert_eq!(slugify("weird//name--here", None), "name-here");
    }
}
