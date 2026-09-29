//! Discovering, creating and removing workspaces.
//!
//! A "workspace" is any directory on disk holding a branch of a repo: the
//! primary clone, a linked worktree, or a second full clone. The distinction
//! matters exactly twice — when creating one (git won't check a branch out in
//! two worktrees) and when removing one — so the rest of the app treats them
//! identically.

use crate::config::{ProjectConfig, SetupStep};
use crate::git::{self, Workspace, WorkspaceKind};
use crate::proc::{run, run_shell_streaming};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::{Path, PathBuf};

/// Everything on disk that belongs to this repo.
///
/// The per-workspace `git status` is the slow part here — a few hundred
/// milliseconds each on a large working tree — so the workspaces are inspected
/// on a small pool of threads and `report` is called as each one lands.
///
/// `remote_branches` is passed in rather than recomputed: the caller has
/// already listed the refs, and doing it twice was costing a redundant pair of
/// `for-each-ref` calls on every refresh.
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

pub fn scan(
    project: &ProjectConfig,
    base: &str,
    remote_branches: &HashSet<String>,
    report: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Workspace>, String> {
    let primary = PathBuf::from(&project.path);
    let (clones, orphans) = siblings(project)?;

    /// A workspace we know about but haven't inspected the working tree of yet.
    struct Entry {
        path: PathBuf,
        branch: Option<String>,
        kind: WorkspaceKind,
        owner: Option<String>,
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = HashSet::new();

    for clone in &clones {
        let owner = clone.to_string_lossy().to_string();
        for (path, branch, _head) in git::worktrees(clone).unwrap_or_default() {
            if !seen.insert(path.clone()) {
                continue;
            }
            let is_root = &path == clone;
            let kind = if path == primary {
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
                            let (tree, unpushed) = if entry.kind == WorkspaceKind::Orphan {
                                (git::WorkingTree::default(), 0)
                            } else {
                                (
                                    git::working_tree(&entry.path),
                                    git::unpushed_count(&entry.path),
                                )
                            };
                            report(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                            let gone = entry
                                .branch
                                .as_ref()
                                .map(|b| b != base && !remote_branches.contains(b))
                                .unwrap_or(entry.kind == WorkspaceKind::Orphan);
                            Workspace {
                                path: entry.path.to_string_lossy().to_string(),
                                kind: entry.kind,
                                branch: entry.branch.clone(),
                                owner: entry.owner.clone(),
                                modified: tree.modified,
                                untracked: tree.untracked,
                                unpushed,
                                branch_gone: gone,
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

    match req.mode.as_str() {
        "worktree" => create_worktree(&primary, base, req, &mut log)?,
        "clone" => create_clone(&primary, req, &mut log)?,
        other => return Err(format!("unknown workspace mode '{other}'")),
    }

    run_setup_steps(project, &primary, &target, &req.branch, &req.steps, &mut log);
    log_step(&mut log, "Done");
    Ok(req.path.clone())
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
    log_step(log, &format!("Pointing origin at {origin}"));
    run("git", &["remote", "set-url", "origin", &origin], Some(&target))?
        .text()?;
    run("git", &["fetch", "origin", "--prune", "--quiet"], Some(&target))?.text()?;

    let branch = &req.branch;
    log_step(log, &format!("Checking out {branch}"));
    let remote_ref = format!("origin/{branch}");
    let workspace = if req.new_branch {
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalImpact {
    pub path: String,
    pub kind: WorkspaceKind,
    pub branch: Option<String>,
    pub modified: u32,
    pub untracked: u32,
    pub unpushed: u32,
    pub stashes: u32,
}

pub fn removal_impact(path: &Path, kind: WorkspaceKind, branch: Option<String>) -> RemovalImpact {
    let is_git = git::is_git_repo(path);
    let tree = if is_git {
        git::working_tree(path)
    } else {
        git::WorkingTree::default()
    };
    RemovalImpact {
        path: path.to_string_lossy().to_string(),
        kind,
        branch,
        modified: tree.modified,
        untracked: tree.untracked,
        unpushed: if is_git { git::unpushed_count(path) } else { 0 },
        stashes: if is_git { git::stash_count(path) } else { 0 },
    }
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
        let dir = std::env::temp_dir().join(format!("switcheroo-del-{}", std::process::id()));
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
        let root = std::env::temp_dir().join(format!("switcheroo-own-{}", std::process::id()));
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
