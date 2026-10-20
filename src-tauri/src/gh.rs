//! Pull request state, via the `gh` CLI.
//!
//! Two calls per refresh: open PRs with their check rollup (the expensive
//! field, but only for the handful that are open), and a lightweight pass over
//! everything so merged and closed branches can be flagged for cleanup.

use crate::proc::{run, run_timeout, NETWORK_TIMEOUT};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckState {
    Pending,
    Passing,
    Failing,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub branch: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub is_draft: bool,
    pub url: String,
    pub author: String,
    /// `APPROVED`, `CHANGES_REQUESTED`, `REVIEW_REQUIRED` or empty.
    pub review_decision: String,
    pub checks: CheckState,
    /// GitHub logins, for the "assigned to me" filter.
    pub assignees: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPr {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    url: String,
    #[serde(default)]
    head_ref_name: String,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    author: Option<RawAuthor>,
    #[serde(default)]
    assignees: Vec<RawAuthor>,
    #[serde(default)]
    status_check_rollup: Option<Vec<RawCheck>>,
}

#[derive(Deserialize)]
struct RawAuthor {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct RawCheck {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

fn rollup(checks: &Option<Vec<RawCheck>>) -> CheckState {
    let Some(checks) = checks else {
        return CheckState::None;
    };
    if checks.is_empty() {
        return CheckState::None;
    }
    let mut pending = false;
    let mut failing = false;
    for c in checks {
        // Check runs report status/conclusion; legacy commit statuses report
        // `state`. Normalise both onto one axis.
        let status = c.status.as_deref().unwrap_or("");
        let verdict = c
            .conclusion
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(c.state.as_deref())
            .unwrap_or("");

        if status.eq_ignore_ascii_case("QUEUED")
            || status.eq_ignore_ascii_case("IN_PROGRESS")
            || status.eq_ignore_ascii_case("PENDING")
            || verdict.eq_ignore_ascii_case("PENDING")
        {
            pending = true;
        } else if verdict.eq_ignore_ascii_case("FAILURE")
            || verdict.eq_ignore_ascii_case("TIMED_OUT")
            || verdict.eq_ignore_ascii_case("ERROR")
            || verdict.eq_ignore_ascii_case("CANCELLED")
        {
            failing = true;
        }
    }
    if failing {
        CheckState::Failing
    } else if pending {
        CheckState::Pending
    } else {
        CheckState::Passing
    }
}

fn to_pr(raw: RawPr, fallback_state: &str) -> PullRequest {
    PullRequest {
        number: raw.number,
        title: raw.title,
        branch: raw.head_ref_name,
        state: if raw.state.is_empty() {
            fallback_state.to_string()
        } else {
            raw.state.to_lowercase()
        },
        is_draft: raw.is_draft,
        url: raw.url,
        author: raw.author.map(|a| a.login).unwrap_or_default(),
        assignees: raw.assignees.into_iter().map(|a| a.login).collect(),
        review_decision: raw.review_decision.unwrap_or_default(),
        checks: rollup(&raw.status_check_rollup),
    }
}

fn list(repo: &Path, slug: &str, args: &[&str]) -> Result<Vec<RawPr>, String> {
    let mut argv = vec!["pr", "list", "--repo", slug];
    argv.extend_from_slice(args);
    let out = run_timeout("gh", &argv, Some(repo), NETWORK_TIMEOUT)?;
    if !out.ok() {
        return Err(out.stderr.trim().to_string());
    }
    serde_json::from_str(&out.stdout).map_err(|e| format!("could not parse gh output: {e}"))
}

/// Pull requests keyed by head branch name. Open PRs win over closed ones when
/// a branch has been reused.
///
/// The two queries are independent round trips to GitHub, so they run side by
/// side rather than one after the other.
pub fn pull_requests(repo: &Path, slug: &str) -> Result<HashMap<String, PullRequest>, String> {
    let (all, open) = std::thread::scope(|scope| {
        // Historic PRs: cheap fields only, so 300 of them stay fast.
        let all = scope.spawn(|| {
            list(
                repo,
                slug,
                &[
                    "--state",
                    "all",
                    "--limit",
                    "300",
                    "--json",
                    "number,title,state,url,headRefName,author,isDraft",
                ],
            )
        });
        // Open PRs, with the review and CI state that only they need.
        let open = scope.spawn(|| {
            list(
                repo,
                slug,
                &[
                    "--state",
                    "open",
                    "--limit",
                    "150",
                    "--json",
                    "number,title,state,url,headRefName,author,isDraft,reviewDecision,statusCheckRollup,assignees",
                ],
            )
        });
        (
            all.join().unwrap_or_else(|_| Err("gh query panicked".into())),
            open.join().unwrap_or_else(|_| Err("gh query panicked".into())),
        )
    });

    let mut map = HashMap::new();
    // A failure to list historic PRs only costs us merged/closed badges, so it
    // is not worth failing the whole refresh over.
    if let Ok(all) = all {
        for raw in all {
            let pr = to_pr(raw, "closed");
            map.insert(pr.branch.clone(), pr);
        }
    }
    for raw in open? {
        let pr = to_pr(raw, "open");
        map.insert(pr.branch.clone(), pr);
    }
    Ok(map)
}

/// An open issue, offered by the New workspace dialog as something to start a
/// branch for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// RFC 3339, as GitHub sends it.
    pub updated_at: String,
    /// `Bug`, `Feature`, `Task`, … — the repository's issue types. None when
    /// the issue has none, or the repository doesn't use them.
    pub issue_type: Option<String>,
    /// Branches linked through the issue's "Development" panel, which win over
    /// a name derived from the title.
    pub linked_branches: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssue {
    number: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    issue_type: Option<RawName>,
    #[serde(default)]
    linked_branches: Option<RawLinkedBranches>,
}

#[derive(Deserialize)]
struct RawName {
    name: String,
}

#[derive(Deserialize)]
struct RawLinkedBranches {
    #[serde(default)]
    nodes: Vec<RawLinkedBranch>,
}

#[derive(Deserialize)]
struct RawLinkedBranch {
    /// Null once the linked branch has been deleted.
    #[serde(default)]
    r#ref: Option<RawName>,
}

/// One issue per line, as `--jq '...nodes[]'` prints them across pages.
fn parse_issues(stdout: &str) -> Result<Vec<Issue>, String> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let raw: RawIssue = serde_json::from_str(line)
                .map_err(|e| format!("could not parse gh output: {e}"))?;
            Ok(Issue {
                number: raw.number,
                title: raw.title,
                url: raw.url,
                updated_at: raw.updated_at,
                issue_type: raw.issue_type.map(|t| t.name),
                linked_branches: raw
                    .linked_branches
                    .map(|l| l.nodes.into_iter().filter_map(|b| b.r#ref).map(|r| r.name).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Every open issue, most recently updated first.
///
/// GraphQL rather than `gh issue list`, which can't ask for issue types or
/// linked branches. Pages of 100: ~4s for 300 issues, about what
/// `gh issue list` takes for the same without those two fields.
pub fn open_issues(repo: &Path, slug: &str) -> Result<Vec<Issue>, String> {
    let (owner, name) = slug.split_once('/').ok_or_else(|| format!("not owner/name: {slug}"))?;
    let query = "query($owner: String!, $name: String!, $endCursor: String) {
        repository(owner: $owner, name: $name) {
          issues(states: OPEN, first: 100, after: $endCursor,
                 orderBy: { field: UPDATED_AT, direction: DESC }) {
            pageInfo { hasNextPage endCursor }
            nodes {
              number title url updatedAt
              issueType { name }
              linkedBranches(first: 5) { nodes { ref { name } } }
            }
          }
        }
      }";
    let query = format!("query={query}");
    let owner = format!("owner={owner}");
    let name = format!("name={name}");
    let out = run_timeout(
        "gh",
        &[
            "api",
            "graphql",
            "--paginate",
            "-f",
            &query,
            // `-f`, not `-F`: a repository called `1` or `true` stays a string.
            "-f",
            &owner,
            "-f",
            &name,
            "--jq",
            ".data.repository.issues.nodes[]",
        ],
        Some(repo),
        NETWORK_TIMEOUT,
    )?;
    if !out.ok() {
        return Err(out.stderr.trim().to_string());
    }
    parse_issues(&out.stdout)
}

static VIEWER: OnceLock<Option<String>> = OnceLock::new();

/// The logged-in GitHub user. Fixed for the life of the process.
pub fn viewer_login() -> Option<&'static str> {
    VIEWER
        .get_or_init(|| {
            run("gh", &["api", "user", "--jq", ".login"], None)
                .ok()
                .filter(|out| out.ok())
                .map(|out| out.stdout.trim().to_string())
                .filter(|login| !login.is_empty())
        })
        .as_deref()
}

pub fn is_available() -> bool {
    run("gh", &["--version"], None).map(|o| o.ok()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_issues_one_per_line() {
        let out = concat!(
            r#"{"number":101,"title":"Dark mode","url":"u","updatedAt":"2026-10-20T15:40:32Z","issueType":null,"linkedBranches":{"nodes":[{"ref":{"name":"feature/dark-mode"}},{"ref":null}]}}"#,
            "\n",
            r#"{"number":102,"title":"Login fails","url":"v","updatedAt":"2026-10-14T09:34:47Z","issueType":{"name":"Bug"},"linkedBranches":{"nodes":[]}}"#,
            "\n",
        );
        let issues = parse_issues(out).unwrap();
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].issue_type, None);
        // A deleted linked branch comes back as a null ref and is dropped.
        assert_eq!(issues[0].linked_branches, vec!["feature/dark-mode"]);
        assert_eq!(issues[1].issue_type.as_deref(), Some("Bug"));
        assert!(issues[1].linked_branches.is_empty());
    }
}
