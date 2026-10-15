//! What a workspace holds that origin doesn't.
//!
//! Every warning the app shows about a workspace comes from here: the badges
//! in the branch list and on the workspace card, and the "would lose" column
//! in Cleanup. They have to be right in both directions. A false alarm teaches
//! the user to ignore the badge; a miss lets Cleanup delete work while calling
//! it free. The old `@{u}..HEAD` count managed both on the same machine — six
//! "unpushed" commits that origin had under new hashes after a rebase, and
//! zero for branches that had never been pushed at all.

use crate::proc::run;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// One workspace's state, as the badges show it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Tracked files with changes, staged or not, conflicts included.
    pub modified: u32,
    /// Untracked files and directories. Kept apart from `modified`: a stray
    /// `.DS_Store` is not work anyone can lose, and an IDE shows that tree as
    /// clean.
    pub untracked: u32,
    /// Commits at HEAD that origin has in no form. See [`unpushed`].
    pub unpushed: u32,
    /// Commits its upstream on origin has that HEAD lacks, rewritten copies
    /// of HEAD's own commits not counted. Zero without such an upstream.
    pub behind: u32,
}

fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    run("git", args, Some(path))?.text()
}

fn count(path: &Path, args: &[&str]) -> Result<u32, String> {
    let out = git(path, args)?;
    out.trim()
        .parse()
        .map_err(|_| format!("unexpected output from git {}: {out}", args.join(" ")))
}

fn lines(text: &str) -> HashSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Read a workspace's state.
///
/// An error means git couldn't be asked — a missing directory, a corrupt
/// index, a timeout — and the caller must say "unknown". Reporting zeros
/// instead is how a broken workspace used to look perfectly clean.
pub fn inspect(path: &Path, base: &str) -> Result<Status, String> {
    if !path.is_dir() {
        return Err("the directory is missing".into());
    }
    let porcelain = parse_porcelain(&git(
        path,
        &["status", "--porcelain=v2", "--branch", "-z"],
    )?);

    // Only an upstream on origin vouches for anything. One set to a local
    // branch says nothing about what has been pushed. `branch.ab` is absent
    // when the upstream is configured but its ref is gone.
    let tracked = porcelain
        .upstream
        .as_deref()
        .filter(|u| u.starts_with("origin/"))
        .and(porcelain.ahead_behind);

    let unpushed = match tracked {
        // Nothing past the upstream, and the upstream is on origin.
        Some((0, _)) => 0,
        _ => unpushed(path, "HEAD", tracked.map(|_| "@{upstream}"), base)?.len() as u32,
    };
    let behind = match tracked {
        // Diverged: the upstream may hold rewritten copies of HEAD's own
        // commits, which "behind" shouldn't count twice.
        Some((ahead, behind)) if ahead > 0 && behind > 0 => count(
            path,
            &["rev-list", "--count", "--left-only", "--cherry-pick", "@{upstream}...HEAD"],
        )?,
        Some((_, behind)) => behind,
        None => 0,
    };

    Ok(Status {
        modified: porcelain.modified,
        untracked: porcelain.untracked,
        unpushed,
        behind,
    })
}

/// The commits reachable from `tip` that origin doesn't have in any form.
///
/// A commit counts only if it gets through three filters. Each one removes a
/// kind of false alarm seen on real workspaces:
///
/// 1. **On no origin ref at all.** Measured against the upstream alone,
///    commits that reached origin some other way — merged into main, say —
///    were still counted.
/// 2. **Not a rewritten copy** (`--cherry-pick`: same patch id) of a commit
///    on the upstream or the base branch. A branch rebased and force-pushed
///    from another machine leaves this workspace holding the old copies under
///    hashes origin has never seen.
/// 3. **Its changes aren't already all in the base branch.** Merging `tip`
///    into it would change nothing. This is what catches a squash merge,
///    where many commits became one and no patch id matches.
///
/// Without an upstream, only the base branch is compared against. That is
/// the branch that has never been pushed, which is exactly the one whose
/// commits are most at risk. `upstream` must be a ref on origin.
pub fn unpushed(
    path: &Path,
    tip: &str,
    upstream: Option<&str>,
    base: &str,
) -> Result<HashSet<String>, String> {
    let mut left = lines(&git(path, &["rev-list", tip, "--not", "--remotes=origin"])?);
    if left.is_empty() {
        return Ok(left);
    }

    let base_ref = format!("refs/remotes/origin/{base}");
    let has_base = run("git", &["rev-parse", "--verify", "--quiet", &base_ref], Some(path))
        .map(|o| o.ok())
        .unwrap_or(false);
    let references: Vec<&str> = upstream
        .into_iter()
        .chain(has_base.then_some(base_ref.as_str()))
        .collect();

    for reference in &references {
        let range = format!("{reference}...{tip}");
        let survivors = lines(&git(
            path,
            &["rev-list", "--right-only", "--cherry-pick", &range],
        )?);
        left.retain(|c| survivors.contains(c));
        if left.is_empty() {
            return Ok(left);
        }
    }

    if has_base && already_contains(path, &base_ref, tip) {
        left.clear();
    }
    Ok(left)
}

/// Every change `tip` makes is already in `reference`: merging it in would
/// leave `reference`'s tree as it is.
///
/// `merge-tree --write-tree` touches neither the index nor the working tree.
/// It does write the merged tree into the object store as unreachable objects.
/// Identical inputs produce identical objects, and `gc` collects them. A
/// conflict, or a git too old to have `--write-tree`, comes back as an error,
/// which is read as "not contained": the safe direction.
fn already_contains(path: &Path, reference: &str, tip: &str) -> bool {
    let Ok(merged) = git(path, &["merge-tree", "--write-tree", reference, tip]) else {
        return false;
    };
    let Ok(tree) = git(path, &["rev-parse", &format!("{reference}^{{tree}}")]) else {
        return false;
    };
    merged.lines().next().map(str::trim) == Some(tree.trim())
}

/// Local branches by short name, each with its upstream when that is a ref
/// on origin that still exists — `None` for no upstream, a local one, or one
/// that is gone. One call for all of them; `refname` narrows it to one.
fn branches_with_upstreams(path: &Path, refname: &str) -> Result<Vec<(String, Option<String>)>, String> {
    let out = git(
        path,
        &[
            "for-each-ref",
            "--format=%(refname:short)%09%(upstream)%09%(upstream:track)",
            refname,
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            // Not `split_once`: empty trailing fields leave no tab behind
            // once the output is trimmed.
            let mut fields = line.split('\t');
            let name = fields.next()?.to_string();
            let upstream = fields.next().unwrap_or("");
            let track = fields.next().unwrap_or("");
            let usable = upstream.starts_with("refs/remotes/origin/") && track != "[gone]";
            Some((name, usable.then(|| upstream.to_string())))
        })
        .collect())
}

/// The upstream of one local branch, on the terms of [`branches_with_upstreams`].
pub fn origin_upstream(path: &Path, branch: &str) -> Option<String> {
    branches_with_upstreams(path, &format!("refs/heads/{branch}"))
        .ok()?
        .into_iter()
        .next()?
        .1
}

/// Everything unpushed in a whole repository: every local branch, plus HEAD
/// when it is detached. What removing a clone loses, since every ref it has
/// goes with the directory.
///
/// A clone can carry a hundred branches, nearly all of them long since on
/// origin. One walk over all of them settles the common case. Only when it
/// finds something does each branch get its own filtering, which needs that
/// branch's upstream, and those run on a pool.
pub fn unpushed_anywhere(path: &Path, base: &str) -> Result<HashSet<String>, String> {
    let any = git(
        path,
        &["rev-list", "-1", "--branches", "HEAD", "--not", "--remotes=origin"],
    )?;
    if any.trim().is_empty() {
        return Ok(HashSet::new());
    }

    let mut tips: Vec<(String, Option<String>)> = branches_with_upstreams(path, "refs/heads")?
        .into_iter()
        .map(|(name, upstream)| (format!("refs/heads/{name}"), upstream))
        .collect();
    let detached = git(path, &["rev-parse", "--symbolic-full-name", "HEAD"])?.trim() == "HEAD";
    if detached {
        tips.push(("HEAD".into(), None));
    }

    let mut all = HashSet::new();
    for found in crate::proc::parallel_map(tips, |(tip, upstream)| {
        unpushed(path, tip, upstream.as_deref(), base)
    }) {
        all.extend(found?);
    }
    Ok(all)
}

/// Commits reachable from `tip` and from no local branch other than `except`.
/// What deleting that branch (or abandoning a detached HEAD, with `except`
/// `None`) can actually lose. A commit another branch holds survives.
pub fn held_only_by(path: &Path, tip: &str, except: Option<&str>) -> Result<HashSet<String>, String> {
    let exclude = except.map(|b| format!("--exclude={b}"));
    let mut args = vec!["rev-list", tip, "--not", "--remotes=origin"];
    // `--exclude` applies to the `--branches` that follows it, so the two
    // stay adjacent and in this order.
    if let Some(exclude) = exclude.as_deref() {
        args.push(exclude);
    }
    args.push("--branches");
    Ok(lines(&git(path, &args)?))
}

pub fn stash_count(path: &Path) -> Result<u32, String> {
    Ok(git(path, &["stash", "list"])?.lines().count() as u32)
}

/// The parts of `git status --porcelain=v2 --branch -z` the badges use.
#[derive(Debug, Default, PartialEq, Eq)]
struct Porcelain {
    modified: u32,
    untracked: u32,
    /// `# branch.upstream`, e.g. `origin/feature`.
    upstream: Option<String>,
    /// `# branch.ab`. Absent when the upstream ref no longer exists.
    ahead_behind: Option<(u32, u32)>,
}

/// Parse NUL-separated porcelain v2. Paths come through verbatim, never
/// quoted, so a name with spaces or newlines can't be misread. A rename is
/// the one record followed by a second one, its original path, which is
/// skipped so it can't be taken for an entry of its own.
fn parse_porcelain(out: &str) -> Porcelain {
    let mut p = Porcelain::default();
    let mut records = out.split('\0');
    while let Some(record) = records.next() {
        if let Some(name) = record.strip_prefix("# branch.upstream ") {
            p.upstream = Some(name.to_string());
        } else if let Some(ab) = record.strip_prefix("# branch.ab ") {
            let mut it = ab.split(' ');
            let ahead = it.next().and_then(|a| a.strip_prefix('+')?.parse().ok());
            let behind = it.next().and_then(|b| b.strip_prefix('-')?.parse().ok());
            if let (Some(ahead), Some(behind)) = (ahead, behind) {
                p.ahead_behind = Some((ahead, behind));
            }
        } else if record.starts_with("? ") {
            p.untracked += 1;
        } else if let Some(kind @ ("1" | "2" | "u")) = record.split(' ').next() {
            // `<kind> <XY> <sub> ...`. A submodule whose only change is
            // untracked files inside it (`S..U`) is untracked noise too.
            let sub = record.split(' ').nth(2).unwrap_or("");
            if sub == "S..U" {
                p.untracked += 1;
            } else {
                p.modified += 1;
            }
            if kind == "2" {
                records.next();
            }
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_porcelain_v2() {
        let out = [
            "# branch.oid 0123",
            "# branch.head feature",
            "# branch.upstream origin/feature",
            "# branch.ab +2 -5",
            "1 .M N... 100644 100644 100644 aaa bbb src/a file.ts",
            "1 M. N... 100644 100644 100644 aaa bbb staged.ts",
            // A rename: its original path is a record of its own, and a
            // nasty one at that.
            "2 R. N... 100644 100644 100644 aaa bbb R100 new.ts",
            "? not-untracked",
            "u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.ts",
            "1 .M S..U 160000 160000 160000 aaa aaa vendor/sub",
            "? .DS_Store",
            "? new\ndir/",
            "",
        ]
        .join("\0");
        assert_eq!(
            parse_porcelain(&out),
            Porcelain {
                modified: 4,
                untracked: 3,
                upstream: Some("origin/feature".into()),
                ahead_behind: Some((2, 5)),
            }
        );
    }

    #[test]
    fn a_gone_upstream_has_no_counts() {
        let out = "# branch.oid 0123\0# branch.head f\0# branch.upstream origin/f\0";
        let p = parse_porcelain(out);
        assert_eq!(p.upstream.as_deref(), Some("origin/f"));
        assert_eq!(p.ahead_behind, None);
    }

    /// A scratch area holding a bare `origin.git` and a clone of it, `work`,
    /// with one commit on `main` pushed.
    struct Fixture {
        root: PathBuf,
        work: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("switcheroo-status-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&root).ok();
            std::fs::create_dir_all(root.join("origin.git")).unwrap();
            let f = Fixture { work: root.join("work"), root };
            f.git_in(&f.root, &["init", "--quiet", "--bare", "-b", "main", "origin.git"]);
            f.git_in(&f.root, &["clone", "--quiet", "origin.git", "work"]);
            f.git(&["config", "user.email", "test@example.com"]);
            f.git(&["config", "user.name", "Test"]);
            f.commit("base");
            f.git(&["push", "--quiet", "-u", "origin", "main"]);
            f
        }

        fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
            let out = run("git", args, Some(cwd)).expect("git ran");
            assert!(out.ok(), "git {args:?} failed: {}", out.stderr);
            out.stdout
        }

        fn git(&self, args: &[&str]) -> String {
            self.git_in(&self.work, args)
        }

        /// A commit adding a file named after `msg`.
        fn commit(&self, msg: &str) {
            std::fs::write(self.work.join(msg), msg).unwrap();
            self.git(&["add", "-A"]);
            self.git(&["commit", "--quiet", "-m", msg]);
        }

        fn status(&self) -> Status {
            inspect(&self.work, "main").expect("inspects")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[test]
    fn counts_the_working_tree() {
        let f = Fixture::new("tree");
        assert_eq!(f.status(), Status::default());

        std::fs::write(f.work.join("base"), "changed").unwrap();
        // Not `.DS_Store`: a global gitignore commonly hides that one.
        std::fs::write(f.work.join("stray.txt"), "").unwrap();
        f.git(&["mv", "base", "renamed"]);
        let s = f.status();
        assert_eq!((s.modified, s.untracked), (1, 1), "{s:?}");
    }

    /// The case that hid real work: no upstream, so nothing to compare to,
    /// so zero.
    #[test]
    fn a_never_pushed_branch_is_all_unpushed() {
        let f = Fixture::new("never");
        f.git(&["checkout", "--quiet", "--no-track", "-b", "local"]);
        f.commit("one");
        f.commit("two");
        assert_eq!(f.status().unpushed, 2);
    }

    #[test]
    fn plain_commits_ahead_of_the_upstream_are_unpushed() {
        let f = Fixture::new("ahead");
        f.commit("one");
        let s = f.status();
        assert_eq!((s.unpushed, s.behind), (1, 0));
    }

    /// The reported bug: the branch was rebased and force-pushed elsewhere, so
    /// this workspace holds the old copies under hashes origin never saw.
    #[test]
    fn rewritten_copies_on_the_upstream_are_not_unpushed() {
        let f = Fixture::new("rebased");
        f.git(&["checkout", "--quiet", "-b", "feature"]);
        f.commit("one");
        f.commit("two");
        f.git(&["push", "--quiet", "-u", "origin", "feature"]);

        // Someone else moves main on and rebases the branch onto it.
        f.git(&["checkout", "--quiet", "main"]);
        f.commit("elsewhere");
        f.git(&["push", "--quiet", "origin", "main"]);
        f.git(&["checkout", "--quiet", "-b", "rewritten", "feature"]);
        f.git(&["rebase", "--quiet", "main"]);
        f.commit("three");
        f.git(&["push", "--quiet", "--force", "origin", "rewritten:feature"]);
        f.git(&["checkout", "--quiet", "feature"]);
        f.git(&["fetch", "--quiet"]);

        let s = f.status();
        assert_eq!(s.unpushed, 0, "{s:?}");
        // `elsewhere` and `three`; the rewritten `one` and `two` are not news.
        assert_eq!(s.behind, 2, "{s:?}");
    }

    /// Amended rather than merely rebased: the old content is on no ref.
    #[test]
    fn an_amended_commit_is_still_unpushed() {
        let f = Fixture::new("amended");
        f.git(&["checkout", "--quiet", "-b", "feature"]);
        f.commit("one");
        f.git(&["push", "--quiet", "-u", "origin", "feature"]);
        let original = f.git(&["rev-parse", "HEAD"]);

        std::fs::write(f.work.join("one"), "different").unwrap();
        f.git(&["commit", "--quiet", "-a", "--amend", "--no-edit"]);
        f.git(&["push", "--quiet", "--force", "origin", "feature"]);
        f.git(&["reset", "--quiet", "--hard", original.trim()]);

        assert_eq!(f.status().unpushed, 1);
    }

    /// Ahead of its own upstream, but the commits reached origin by another
    /// branch.
    #[test]
    fn commits_on_another_origin_branch_are_not_unpushed() {
        let f = Fixture::new("elsewhere");
        f.git(&["checkout", "--quiet", "-b", "feature"]);
        f.git(&["push", "--quiet", "-u", "origin", "feature"]);
        f.commit("one");
        f.git(&["push", "--quiet", "origin", "feature:main"]);
        f.git(&["fetch", "--quiet"]);

        let s = f.status();
        assert_eq!(s.unpushed, 0, "{s:?}");
    }

    /// Squash-merged into main and deleted on origin. No ref holds these
    /// commits and no patch id matches, but every change they make is in main.
    #[test]
    fn a_squash_merged_branch_is_not_unpushed() {
        let f = Fixture::new("squash");
        f.git(&["checkout", "--quiet", "-b", "feature"]);
        f.commit("one");
        f.commit("two");
        f.git(&["push", "--quiet", "-u", "origin", "feature"]);

        f.git(&["checkout", "--quiet", "main"]);
        f.commit("unrelated");
        f.git(&["merge", "--quiet", "--squash", "feature"]);
        f.git(&["commit", "--quiet", "-m", "squashed"]);
        f.git(&["push", "--quiet", "origin", "main", ":feature"]);
        f.git(&["fetch", "--quiet", "--prune"]);
        f.git(&["checkout", "--quiet", "feature"]);

        assert_eq!(f.status().unpushed, 0);

        // One more commit after the merge is real work again.
        f.commit("three");
        assert_eq!(f.status().unpushed, 3, "not contained, so every commit counts");
    }

    #[test]
    fn a_missing_directory_is_an_error_not_clean() {
        let f = Fixture::new("missing");
        assert!(inspect(&f.root.join("nope"), "main").is_err());
    }

    #[test]
    fn finds_an_origin_upstream_only_while_it_exists() {
        let f = Fixture::new("upstream");
        assert_eq!(origin_upstream(&f.work, "main").as_deref(), Some("refs/remotes/origin/main"));
        f.git(&["checkout", "--quiet", "--no-track", "-b", "local"]);
        assert_eq!(origin_upstream(&f.work, "local"), None);
        f.git(&["push", "--quiet", "-u", "origin", "local"]);
        f.git(&["push", "--quiet", "origin", ":local"]);
        f.git(&["fetch", "--quiet", "--prune"]);
        assert_eq!(origin_upstream(&f.work, "local"), None, "gone");
    }
}
