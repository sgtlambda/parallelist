//! Discovering, creating and removing checkouts.
//!
//! A "checkout" is any directory on disk holding a branch of a repo: the
//! primary clone, a linked worktree, or a second full clone. The distinction
//! matters exactly twice — when creating one (git won't check a branch out in
//! two worktrees) and when removing one — so the rest of the app treats them
//! identically.

use crate::config::{RepoConfig, SetupStep, StepKind};
use crate::git::{self, Checkout, CheckoutKind};
use crate::proc::{run, run_shell_streaming};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::path::{Path, PathBuf};

/// Everything on disk that belongs to this repo.
///
/// The per-checkout `git status` is the slow part here — a few hundred
/// milliseconds each on a large working tree — so the checkouts are inspected
/// on a small pool of threads and `report` is called as each one lands.
///
/// `remote_branches` is passed in rather than recomputed: the caller has
/// already listed the refs, and doing it twice was costing a redundant pair of
/// `for-each-ref` calls on every refresh.
pub fn scan(
    repo: &RepoConfig,
    base: &str,
    remote_branches: &HashSet<String>,
    report: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<Checkout>, String> {
    let primary = PathBuf::from(&repo.path);
    let parent = primary
        .parent()
        .ok_or_else(|| "repo has no parent directory".to_string())?
        .to_path_buf();
    let origin = git::origin_url(&primary)?;
    let dir_name = primary
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // Sibling directories that are full clones of the same remote. Read from
    // .git/config rather than by spawning git, so scanning a folder with a
    // hundred projects in it stays instant.
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

    /// A checkout we know about but haven't inspected the working tree of yet.
    struct Entry {
        path: PathBuf,
        branch: Option<String>,
        kind: CheckoutKind,
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
                CheckoutKind::Primary
            } else if is_root {
                CheckoutKind::Clone
            } else {
                CheckoutKind::Worktree
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
            kind: CheckoutKind::Orphan,
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

    let mut out: Vec<Checkout> = std::thread::scope(|scope| {
        let handles: Vec<_> = entries
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(|| {
                    chunk
                        .iter()
                        .map(|entry| {
                            // An orphan has no git dir, so there is nothing to
                            // ask git about and no process worth spawning.
                            let (dirty, unpushed) = if entry.kind == CheckoutKind::Orphan {
                                (0, 0)
                            } else {
                                (
                                    git::dirty_count(&entry.path),
                                    git::unpushed_count(&entry.path),
                                )
                            };
                            report(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                            let gone = entry
                                .branch
                                .as_ref()
                                .map(|b| b != base && !remote_branches.contains(b))
                                .unwrap_or(entry.kind == CheckoutKind::Orphan);
                            Checkout {
                                path: entry.path.to_string_lossy().to_string(),
                                kind: entry.kind,
                                branch: entry.branch.clone(),
                                owner: entry.owner.clone(),
                                dirty,
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
pub fn suggest_path(repo: &RepoConfig, branch: &str, pr: Option<u64>) -> String {
    let primary = PathBuf::from(&repo.path);
    let parent = primary
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let repo_dir = primary
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| repo.name.clone());

    let base = repo
        .path_template
        .replace("{parent}", &parent)
        .replace("{repo}", &repo_dir)
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
    pub repo_id: String,
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
    /// The checkout already holding it.
    pub path: String,
}

/// Git refuses to check one branch out in two worktrees of the same clone.
/// Finding the existing one lets the create dialog say so up front and offer
/// a clone instead, rather than failing after the user hits create.
pub fn conflicting_worktree(repo: &RepoConfig, branch: &str) -> Option<BranchConflict> {
    let primary = PathBuf::from(&repo.path);
    git::worktrees(&primary)
        .ok()?
        .into_iter()
        .find(|(_, b, _)| b.as_deref() == Some(branch))
        .map(|(path, _, _)| BranchConflict {
            branch: branch.to_string(),
            path: path.to_string_lossy().to_string(),
        })
}

fn log_step<F: FnMut(&str, bool)>(log: &mut F, msg: &str) {
    log(&format!("\u{25b8} {msg}"), false);
}

/// Create the checkout and run the selected setup steps, streaming everything
/// through `log`. Returns the created path.
pub fn create<F: FnMut(&str, bool)>(
    repo: &RepoConfig,
    base: &str,
    req: &CreateRequest,
    mut log: F,
) -> Result<String, String> {
    let primary = PathBuf::from(&repo.path);
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
        other => return Err(format!("unknown checkout mode '{other}'")),
    }

    run_setup_steps(repo, &primary, &target, &req.steps, &mut log);
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
    log_step(log, "Cloning from primary clone (hardlinked objects)");
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
    let checkout = if req.new_branch {
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
    for line in checkout.stderr.lines() {
        log(line, !checkout.ok());
    }
    if !checkout.ok() {
        return Err(format!("checkout failed: {}", checkout.stderr.trim()));
    }
    Ok(())
}

fn run_setup_steps<F: FnMut(&str, bool)>(
    repo: &RepoConfig,
    primary: &Path,
    target: &Path,
    enabled: &[String],
    log: &mut F,
) {
    let steps: Vec<&SetupStep> = repo
        .setup_steps
        .iter()
        .filter(|s| enabled.contains(&s.id))
        .collect();
    if steps.is_empty() {
        return;
    }
    for step in steps {
        log_step(log, &step.label);
        match &step.kind {
            StepKind::CopyFiles { files } => {
                for file in files {
                    let from = primary.join(file);
                    let to = target.join(file);
                    if !from.exists() {
                        log(&format!("  skipped {file} (not in primary clone)"), false);
                        continue;
                    }
                    match std::fs::copy(&from, &to) {
                        Ok(_) => log(&format!("  copied {file}"), false),
                        Err(e) => log(&format!("  failed to copy {file}: {e}"), true),
                    }
                }
            }
            StepKind::Symlink { paths } => {
                for rel in paths {
                    let from = primary.join(rel);
                    let to = target.join(rel);
                    if !from.exists() {
                        log(&format!("  skipped {rel} (not in primary clone)"), false);
                        continue;
                    }
                    if to.exists() {
                        log(&format!("  skipped {rel} (already present)"), false);
                        continue;
                    }
                    match std::os::unix::fs::symlink(&from, &to) {
                        Ok(_) => log(&format!("  linked {rel}"), false),
                        Err(e) => log(&format!("  failed to link {rel}: {e}"), true),
                    }
                }
            }
            StepKind::Command { script } => {
                match run_shell_streaming(script, target, |line, is_err| {
                    log(&format!("  {line}"), is_err)
                }) {
                    Ok(0) => {}
                    Ok(code) => log(&format!("  exited with status {code}"), true),
                    Err(e) => log(&format!("  {e}"), true),
                }
            }
        }
    }
}

/// What removing a checkout would destroy. Shown in the confirm dialog; it
/// never blocks the removal, it just makes sure the cost is visible.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalImpact {
    pub path: String,
    pub kind: CheckoutKind,
    pub branch: Option<String>,
    pub dirty: u32,
    pub unpushed: u32,
    pub stashes: u32,
    pub size_bytes: u64,
}

pub fn removal_impact(path: &Path, kind: CheckoutKind, branch: Option<String>) -> RemovalImpact {
    let is_git = git::is_git_repo(path);
    RemovalImpact {
        path: path.to_string_lossy().to_string(),
        kind,
        branch,
        dirty: if is_git { git::dirty_count(path) } else { 0 },
        unpushed: if is_git { git::unpushed_count(path) } else { 0 },
        stashes: if is_git { git::stash_count(path) } else { 0 },
        size_bytes: dir_size(path),
    }
}

/// `du -sk` rather than a recursive walk: node_modules makes the walk slow,
/// and du already knows how to not double-count hardlinks.
fn dir_size(path: &Path) -> u64 {
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

pub fn remove(
    repo: &RepoConfig,
    path: &Path,
    kind: CheckoutKind,
    delete_branch: Option<String>,
) -> Result<(), String> {
    let primary = PathBuf::from(&repo.path);
    if kind == CheckoutKind::Primary {
        return Err("refusing to remove the primary clone".into());
    }
    let path_str = path.to_string_lossy().to_string();

    match kind {
        CheckoutKind::Worktree => {
            // --force because the impact dialog has already shown the user
            // exactly what is in there.
            let out = run(
                "git",
                &["worktree", "remove", "--force", &path_str],
                Some(&primary),
            )?;
            if !out.ok() {
                // The registration can outlive the directory; prune and retry
                // the plain delete rather than leaving a half-removed entry.
                std::fs::remove_dir_all(path).ok();
                run("git", &["worktree", "prune"], Some(&primary))?;
            }
        }
        CheckoutKind::Clone | CheckoutKind::Orphan => {
            std::fs::remove_dir_all(path).map_err(|e| format!("could not delete: {e}"))?;
        }
        CheckoutKind::Primary => unreachable!(),
    }

    if let Some(branch) = delete_branch {
        run("git", &["branch", "-D", &branch], Some(&primary))?;
    }
    run("git", &["worktree", "prune"], Some(&primary))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
