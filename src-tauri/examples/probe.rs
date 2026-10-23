use std::path::PathBuf;
use std::time::Instant;
use parallelist_lib::{build_overview, peek_overview, workspace, config::ProjectConfig, gh, git};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("usage: probe <repo path> [overview.json]"));
    let base = git::detect_base_branch(&path);
    println!("base branch: {base}");

    let url = git::origin_url(&path).unwrap();
    let slug = git::origin_slug(&url).unwrap();
    println!("origin: {url} -> {slug}");

    let t = Instant::now();
    let branches = git::branches(&path, &base).unwrap();
    println!("branches took {:?}", t.elapsed());
    println!("\n{} branches, 6 most recent:", branches.len());
    for b in branches.iter().take(6) {
        println!(
            "  {:<44} remote={} local={} merged={} {} — {}",
            b.name, b.has_remote, b.has_local, b.merged, b.author, b.subject
        );
    }

    let repo = ProjectConfig::new(path.clone(), "probe".into());
    println!("\nsniffed setup steps:");
    for s in &repo.setup_steps {
        println!("  [{}] {}", if s.default_enabled { "x" } else { " " }, s.label);
    }

    let t = Instant::now();
    let workspaces = workspace::scan(&repo, &base, &|_, _| {}).unwrap();
    println!("\n{} workspaces (scan took {:?}):", workspaces.len(), t.elapsed());
    for c in &workspaces {
        let status = match (&c.status, &c.status_error) {
            (Some(s), _) => format!(
                "modified={} untracked={} unpushed={} behind={}",
                s.modified, s.untracked, s.unpushed, s.behind
            ),
            (None, Some(e)) => format!("status unknown: {e}"),
            (None, None) => "no status".to_string(),
        };
        println!("  {:?} {:<58} branch={:?} {status}", c.kind, c.path, c.branch);
    }

    // What Cleanup would say about removing each one. Read-only.
    let t = Instant::now();
    println!("\nremoval impacts:");
    for c in workspaces.iter().filter(|c| c.kind != git::WorkspaceKind::Primary) {
        let one = Instant::now();
        let i = workspace::removal_impact(&PathBuf::from(&c.path), c.kind, c.branch.clone(), &base);
        println!(
            "  {:<58} {:>6.0?} commits={}{} stashes={} dependents={} error={:?}",
            c.path,
            one.elapsed(),
            i.commits,
            if i.commits_need_branch_delete { " (if branch deleted)" } else { "" },
            i.stashes,
            i.dependents.len(),
            i.error
        );
    }
    println!("  (took {:?})", t.elapsed());

    if let Some(b) = branches.iter().find(|b| b.has_remote && b.name != base) {
        println!("\nplan for {}:", b.name);
        println!("  suggested path: {}", workspace::suggest_path(&repo, &b.name, Some(9999)));
        println!("  conflict: {:?}", workspace::conflicting_worktree(&repo, &b.name));
        println!("  ahead/behind: {:?}", git::ahead_behind(&path, &base, &b.name));
    }

    if gh::is_available() {
        let t = Instant::now();
        match gh::pull_requests(&path, &slug) {
            Ok(prs) => {
                let open: Vec<_> = prs.values().filter(|p| p.state == "open").collect();
                println!(
                    "\n{} PRs known, {} open (gh took {:?}). First 5 open:",
                    prs.len(),
                    open.len(),
                    t.elapsed()
                );
                for pr in open.iter().take(5) {
                    println!(
                        "  #{} {:<34} checks={:?} review={} — {}",
                        pr.number, pr.branch, pr.checks, pr.review_decision, pr.title
                    );
                }
            }
            Err(e) => println!("\ngh error: {e}"),
        }
    }

    if gh::is_available() {
        let t = Instant::now();
        match gh::open_issues(&path, &slug) {
            Ok(issues) => {
                println!("\n{} open issues (gh took {:?}). First 5:", issues.len(), t.elapsed());
                for i in issues.iter().take(5) {
                    println!(
                        "  #{} {:<8} linked={:?} — {}",
                        i.number,
                        i.issue_type.as_deref().unwrap_or("-"),
                        i.linked_branches,
                        i.title
                    );
                }
            }
            Err(e) => println!("\ngh issues error: {e}"),
        }
    }

    // End-to-end: does every workspace have a branch row to attach to?
    let t = Instant::now();
    let overview = build_overview(ProjectConfig::new(path.clone(), "probe".into()), false, &|_, _, _| {}).unwrap();
    // `probe <repo> <file>` also writes the overview exactly as the frontend
    // receives it, for checking UI logic against real data headlessly.
    if let Some(out) = std::env::args().nth(2) {
        std::fs::write(&out, serde_json::to_string_pretty(&overview).unwrap()).unwrap();
        println!("\noverview written to {out}");
    }
    println!("\n--- overview: {} branch rows, built in {:?} ---", overview.branches.len(), t.elapsed());
    for c in &overview.workspaces {
        if let Some(b) = c.branch.as_deref() {
            let listed = overview.branches.iter().find(|x| x.name == b);
            println!(
                "  {:<9?} {:<40} row={} {}",
                c.kind,
                b,
                if listed.is_some() { "yes" } else { "MISSING" },
                listed
                    .map(|x| format!("(onOrigin={} wasPushed={} merged={})", x.has_remote, x.was_pushed, x.merged))
                    .unwrap_or_default()
            );
        }
    }

    // The quick pass shown while that runs: it should have the same rows.
    let t = Instant::now();
    let peek = peek_overview(ProjectConfig::new(path.clone(), "probe".into())).unwrap();
    let names = |o: &parallelist_lib::Overview| {
        let mut n: Vec<String> = o.branches.iter().map(|b| b.name.clone()).collect();
        n.sort();
        n
    };
    println!(
        "\n--- peek: {} branch rows, {} workspaces, built in {:?}; rows {} the overview's ---",
        peek.branches.len(),
        peek.workspaces.len(),
        t.elapsed(),
        if names(&peek) == names(&overview) { "match" } else { "DIFFER from" }
    );
}
