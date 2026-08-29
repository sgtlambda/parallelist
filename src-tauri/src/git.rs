//! Everything Switcheroo knows about a repository, derived from `git`.

use crate::proc::{run, run_timeout, Output, NETWORK_TIMEOUT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn git(args: &[&str], cwd: &Path) -> Result<Output, String> {
    run("git", args, Some(cwd))
}

/// A branch as the app thinks of it: primarily something that exists on
/// origin, with whatever local state we happen to have attached to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub has_remote: bool,
    pub has_local: bool,
    pub sha: String,
    /// Unix seconds of the last commit, used for the recency sort.
    pub committed_at: i64,
    pub author: String,
    pub subject: String,
    /// Reachable from the base branch, i.e. already merged.
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AheadBehind {
    pub ahead: u32,
    pub behind: u32,
}

/// How a checkout is backed. Both are equal citizens in the UI; a clone is
/// what you fall back to when git refuses to check a branch out twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckoutKind {
    Primary,
    Worktree,
    Clone,
    /// A directory that looks like it belongs to this repo but isn't a git
    /// checkout at all — left behind by a manual `rm -rf .git` or a failed
    /// worktree removal.
    Orphan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkout {
    pub path: String,
    pub kind: CheckoutKind,
    /// `None` for detached HEAD and orphans.
    pub branch: Option<String>,
    /// Which clone a worktree hangs off. `None` for a clone's own root.
    pub owner: Option<String>,
    pub dirty: u32,
    pub unpushed: u32,
    /// The branch is gone from origin, or merged into the base branch.
    pub branch_gone: bool,
}

pub fn is_git_repo(path: &Path) -> bool {
    path.join(".git").exists()
}

pub fn origin_url(repo: &Path) -> Result<String, String> {
    git(&["remote", "get-url", "origin"], repo)?.text()
}

/// `owner/name` for `gh`, from any of the URL shapes git accepts.
pub fn origin_slug(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/').trim_end_matches(".git");
    let after_host = if let Some(idx) = trimmed.find("://") {
        // https://github.com/owner/name
        let rest = &trimmed[idx + 3..];
        rest.split_once('/').map(|(_, p)| p)?
    } else if let Some((_, path)) = trimmed.split_once(':') {
        // git@github.com:owner/name
        path
    } else {
        return None;
    };
    let mut parts = after_host.rsplit('/');
    let name = parts.next()?;
    let owner = parts.next()?;
    Some(format!("{owner}/{name}"))
}

/// The branch everything is measured against. Prefers `origin/HEAD`, then the
/// usual suspects, then whatever the primary clone has checked out.
pub fn detect_base_branch(repo: &Path) -> String {
    if let Ok(out) = git(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"], repo) {
        if out.ok() {
            if let Some(name) = out.stdout.trim().strip_prefix("origin/") {
                return name.to_string();
            }
        }
    }
    for candidate in ["staging", "main", "master", "develop"] {
        let refname = format!("refs/remotes/origin/{candidate}");
        if let Ok(out) = git(&["show-ref", "--verify", "--quiet", &refname], repo) {
            if out.ok() {
                return candidate.to_string();
            }
        }
    }
    git(&["branch", "--show-current"], repo)
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "main".to_string())
}

pub fn fetch(repo: &Path) -> Result<(), String> {
    // --prune keeps branches deleted on origin out of the list, which is what
    // makes "merged and gone" a reliable cleanup signal.
    run_timeout(
        "git",
        &["fetch", "--all", "--prune", "--quiet"],
        Some(repo),
        NETWORK_TIMEOUT,
    )?
    .text()
    .map(|_| ())
}

const REF_SEP: char = '\t';

/// Remote and local refs merged into one list keyed by short branch name.
pub fn branches(repo: &Path, base: &str) -> Result<Vec<BranchInfo>, String> {
    let format = [
        "%(refname)",
        "%(objectname)",
        "%(committerdate:unix)",
        "%(authorname)",
        "%(contents:subject)",
    ]
    .join(&REF_SEP.to_string());
    let format_arg = format!("--format={format}");

    let out = git(
        &[
            "for-each-ref",
            &format_arg,
            "refs/heads",
            "refs/remotes/origin",
        ],
        repo,
    )?;
    if !out.ok() {
        return Err(out.stderr.trim().to_string());
    }

    let merged = merged_branches(repo, base).unwrap_or_default();
    let mut map: HashMap<String, BranchInfo> = HashMap::new();

    for line in out.lines() {
        // Subject is last so that a tab inside it can't shift the other fields.
        let mut parts = line.splitn(5, REF_SEP);
        let (Some(refname), Some(sha), Some(date), Some(author)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let subject = parts.next().unwrap_or("");

        let (name, remote) = if let Some(n) = refname.strip_prefix("refs/remotes/origin/") {
            if n == "HEAD" {
                continue;
            }
            (n, true)
        } else if let Some(n) = refname.strip_prefix("refs/heads/") {
            (n, false)
        } else {
            continue;
        };

        let entry = map.entry(name.to_string()).or_insert_with(|| BranchInfo {
            name: name.to_string(),
            has_remote: false,
            has_local: false,
            sha: String::new(),
            committed_at: 0,
            author: String::new(),
            subject: String::new(),
            merged: merged.contains(name),
        });

        if remote {
            entry.has_remote = true;
        } else {
            entry.has_local = true;
        }
        // The remote ref is the source of truth, so it always wins the shared
        // fields; a local-only branch fills them in as a fallback.
        if remote || entry.sha.is_empty() {
            entry.sha = sha.to_string();
            entry.committed_at = date.parse().unwrap_or(0);
            entry.author = author.to_string();
            entry.subject = subject.to_string();
        }
    }

    let mut list: Vec<BranchInfo> = map.into_values().collect();
    list.sort_by_key(|b| std::cmp::Reverse(b.committed_at));
    Ok(list)
}

/// Branch names already reachable from the base branch — one call for all of
/// them rather than a merge-base check per branch.
fn merged_branches(repo: &Path, base: &str) -> Result<std::collections::HashSet<String>, String> {
    let base_ref = format!("origin/{base}");
    let out = git(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "--merged",
            &base_ref,
            "refs/remotes/origin",
        ],
        repo,
    )?;
    Ok(out
        .lines()
        .into_iter()
        .filter_map(|l| l.strip_prefix("refs/remotes/origin/"))
        .filter(|n| *n != "HEAD" && *n != base)
        .map(|n| n.to_string())
        .collect())
}

/// Just the names of branches that exist on origin.
pub fn remote_branch_names(repo: &Path) -> std::collections::HashSet<String> {
    git(
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/remotes/origin",
        ],
        repo,
    )
    .map(|out| {
        out.lines()
            .into_iter()
            .filter_map(|l| l.strip_prefix("refs/remotes/origin/"))
            .filter(|n| *n != "HEAD")
            .map(|n| n.to_string())
            .collect()
    })
    .unwrap_or_default()
}

/// Commit metadata for whatever a checkout currently has at HEAD.
///
/// Used for branches that exist only inside a secondary clone, where the
/// primary has no ref to read them from.
pub fn head_commit(path: &Path) -> Option<(String, i64, String, String)> {
    let format = [
        "%H",
        "%ct",
        "%an",
        "%s",
    ]
    .join(&REF_SEP.to_string());
    let format_arg = format!("--format={format}");
    let out = git(&["log", "-1", &format_arg], path).ok()?;
    if !out.ok() {
        return None;
    }
    let line = out.stdout.lines().next()?;
    let mut parts = line.splitn(4, REF_SEP);
    let sha = parts.next()?.to_string();
    let when = parts.next()?.parse().ok()?;
    let author = parts.next()?.to_string();
    let subject = parts.next().unwrap_or("").to_string();
    Some((sha, when, author, subject))
}

pub fn ahead_behind(repo: &Path, base: &str, branch: &str) -> Option<AheadBehind> {
    let range = format!("origin/{base}...origin/{branch}");
    let out = git(&["rev-list", "--left-right", "--count", &range], repo).ok()?;
    if !out.ok() {
        return None;
    }
    let mut nums = out.stdout.split_whitespace();
    // left = commits only on base (we are behind), right = only on branch.
    let behind = nums.next()?.parse().ok()?;
    let ahead = nums.next()?.parse().ok()?;
    Some(AheadBehind { ahead, behind })
}

/// One entry from `git worktree list`: where it is, the branch it holds (none
/// when detached) and its HEAD.
pub type WorktreeEntry = (PathBuf, Option<String>, Option<String>);

/// Worktrees registered against one clone, including the clone itself.
pub fn worktrees(clone: &Path) -> Result<Vec<WorktreeEntry>, String> {
    let out = git(&["worktree", "list", "--porcelain"], clone)?;
    if !out.ok() {
        return Err(out.stderr.trim().to_string());
    }
    let mut list = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut head: Option<String> = None;
    let mut branch: Option<String> = None;

    let flush = |path: &mut Option<PathBuf>, head: &mut Option<String>, branch: &mut Option<String>, list: &mut Vec<_>| {
        if let Some(p) = path.take() {
            list.push((p, branch.take(), head.take()));
        }
    };

    for line in out.stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            flush(&mut path, &mut head, &mut branch, &mut list);
            path = Some(PathBuf::from(p));
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = Some(h.to_string());
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = Some(b.trim_start_matches("refs/heads/").to_string());
        }
    }
    flush(&mut path, &mut head, &mut branch, &mut list);
    Ok(list)
}

/// Number of modified/untracked entries. Cheap enough to run per checkout.
pub fn dirty_count(path: &Path) -> u32 {
    git(&["status", "--porcelain"], path)
        .map(|o| o.lines().len() as u32)
        .unwrap_or(0)
}

/// Commits on this branch that origin doesn't have. Zero when there's no
/// upstream, since there is then nothing we can claim would be lost.
pub fn unpushed_count(path: &Path) -> u32 {
    git(&["rev-list", "--count", "@{u}..HEAD"], path)
        .ok()
        .filter(|o| o.ok())
        .and_then(|o| o.stdout.trim().parse().ok())
        .unwrap_or(0)
}

pub fn stash_count(path: &Path) -> u32 {
    git(&["stash", "list"], path)
        .map(|o| o.lines().len() as u32)
        .unwrap_or(0)
}

/// Read origin's URL straight out of `.git/config`.
///
/// This is how the sibling scan stays fast: a directory with a `.git` *file*
/// is a linked worktree (we will already have seen it via its owning clone),
/// and a directory with a `.git` *directory* is a clone we can identify
/// without spawning git for each of a hundred sibling folders.
pub fn origin_url_from_config(dir: &Path) -> Option<String> {
    let git_dir = dir.join(".git");
    if !git_dir.is_dir() {
        return None;
    }
    let text = std::fs::read_to_string(git_dir.join("config")).ok()?;
    let mut in_origin = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_origin = line.replace(' ', "") == "[remote\"origin\"]";
            continue;
        }
        if in_origin {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "url" {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

/// Two URLs pointing at the same repository, ignoring protocol and `.git`.
pub fn same_remote(a: &str, b: &str) -> bool {
    match (origin_slug(a), origin_slug(b)) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(&y),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_remote_url_shape() {
        for (url, want) in [
            ("git@github.com:acme/widget.git", "acme/widget"),
            ("git@github.com:acme/widget", "acme/widget"),
            ("https://github.com/acme/widget.git", "acme/widget"),
            ("https://github.com/acme/widget/", "acme/widget"),
            ("ssh://git@github.com/acme/widget.git", "acme/widget"),
        ] {
            assert_eq!(origin_slug(url).as_deref(), Some(want), "url: {url}");
        }
        assert_eq!(origin_slug("not-a-url"), None);
    }

    #[test]
    fn same_remote_ignores_protocol_and_suffix() {
        assert!(same_remote(
            "git@github.com:acme/widget.git",
            "https://github.com/acme/widget"
        ));
        assert!(!same_remote(
            "git@github.com:acme/widget.git",
            "git@github.com:acme/gadget.git"
        ));
    }
}
