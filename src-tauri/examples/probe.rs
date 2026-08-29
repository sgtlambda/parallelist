use std::path::PathBuf;
use std::time::Instant;
use switcheroo_lib::{build_overview, checkout, config::RepoConfig, gh, git};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("usage: probe <repo path>"));
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

    let repo = RepoConfig::new(path.clone(), "probe".into());
    println!("\nsniffed setup steps:");
    for s in &repo.setup_steps {
        println!("  [{}] {}", if s.default_enabled { "x" } else { " " }, s.label);
    }

    let remote_branches: std::collections::HashSet<String> = branches
        .iter()
        .filter(|b| b.has_remote)
        .map(|b| b.name.clone())
        .collect();
    let t = Instant::now();
    let checkouts = checkout::scan(&repo, &base, &remote_branches, &|_, _| {}).unwrap();
    println!("\n{} checkouts (scan took {:?}):", checkouts.len(), t.elapsed());
    for c in &checkouts {
        println!(
            "  {:?} {:<58} branch={:?} modified={} untracked={} unpushed={} gone={}",
            c.kind, c.path, c.branch, c.modified, c.untracked, c.unpushed, c.branch_gone
        );
    }

    if let Some(b) = branches.iter().find(|b| b.has_remote && b.name != base) {
        println!("\nplan for {}:", b.name);
        println!("  suggested path: {}", checkout::suggest_path(&repo, &b.name, Some(9999)));
        println!("  conflict: {:?}", checkout::conflicting_worktree(&repo, &b.name));
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

    // End-to-end: does every checkout have a branch row to attach to?
    let t = Instant::now();
    let overview = build_overview(RepoConfig::new(path.clone(), "probe".into()), false, &|_, _, _| {}).unwrap();
    println!("\n--- overview: {} branch rows, built in {:?} ---", overview.branches.len(), t.elapsed());
    for c in &overview.checkouts {
        if let Some(b) = c.branch.as_deref() {
            let listed = overview.branches.iter().find(|x| x.name == b);
            println!(
                "  {:<9?} {:<40} row={} {}",
                c.kind,
                b,
                if listed.is_some() { "yes" } else { "MISSING" },
                listed
                    .map(|x| format!("(onOrigin={})", x.has_remote))
                    .unwrap_or_default()
            );
        }
    }
}
