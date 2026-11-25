//! Calibrates the screen search models (`screen_search::MODELS`): how
//! similar a query is to the screen it is about, and to the screens it
//! isn't. Each model's `floor` belongs above what unrelated screens score,
//! its `ceiling` around what the right one does.
//!
//! cargo run --release --example screen_search -- <model cache dir> [model id…]
//!
//! Downloads each model into the cache directory the first time.
//!
//! With `live` first, it searches real terminals the way the app does
//! instead: shells in ptys print some of the screens below, and the search
//! runs through a worker process started from this binary.
//!
//! cargo run --release --example screen_search -- live <model cache dir> [model id]

use parallelist_lib::git::WorkspaceKind;
use parallelist_lib::screen_search::{self, similarities, Screen, ScreenSearch, MODELS};
use parallelist_lib::terminal::{Home, Terminals};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const SCREENS: &[(&str, &str)] = &[
    ("agent-stripe", "\
╭──────────────────────────────────────────────╮
│ ✻ Welcome to the agent                       │
╰──────────────────────────────────────────────╯
> the stripe webhook handler drops events when the signature header is missing
⏺ I'll look at how the webhook endpoint verifies signatures.
⏺ Read(app/Http/Controllers/StripeWebhookController.php)
  ⎿  Read 142 lines
⏺ The controller calls Webhook::constructEvent before checking the header, so a missing
  Stripe-Signature throws and the event is logged as unprocessable. Should I return a 400
  early instead, or queue the payload for a retry?
──────────────────────────────────────────────
>
──────────────────────────────────────────────
  ? for shortcuts"),
    ("vite", "\
  VITE v6.2.0  ready in 412 ms
  ➜  Local:   http://localhost:5173/
  ➜  Network: use --host to expose
5:12:01 PM [vite] hmr update /src/components/Header.vue
5:12:04 PM [vite] hmr update /src/styles.css
5:13:22 PM [vite] page reload src/main.ts
5:14:10 PM [vite] hmr update /src/components/Header.vue"),
    ("cargo", "\
$ cargo test
   Compiling parallelist v0.1.0
    Finished test profile in 8.2s
     Running unittests src/lib.rs
running 42 tests
test status::tests::unpushed_ignores_squash_merges ... ok
test workspace::tests::removes_a_worktree ... FAILED
failures:
---- workspace::tests::removes_a_worktree stdout ----
thread panicked at src/workspace.rs:812: is not a working tree
test result: FAILED. 41 passed; 1 failed
$"),
    ("agent-css", "\
> the header overflows on narrow windows, the logo gets pushed under the nav
⏺ Update(src/components/Header.vue)
  ⎿  Updated with 4 additions and 2 removals
     .header { display: flex; flex-wrap: nowrap; min-width: 0 }
⏺ The nav now shrinks instead of the logo, and the menu collapses below 640px.
  Want me to add a screenshot test for the breakpoint?
>"),
    ("migrate", "\
$ php artisan migrate
   INFO  Running migrations.
  2026_11_20_101500_add_invoice_number_to_orders ......... 12ms DONE
  2026_11_24_093000_create_refunds_table ................. 30ms DONE
$ git status
On branch feature/refunds
Changes not staged for commit:
	modified:   app/Models/Refund.php
$"),
    ("shell", "\
$ ls
README.md  package.json  src  vite.config.ts
$ git log --oneline -5
d900652 ci(pages): upload and deploy in separate jobs
b19fe28 Merge redundant-ui-texts
81be979 feat(ui): drop text the controls already say
edddd72 Merge first-run: choose targets from what is installed
2d99e88 feat(onboarding): choose the targets as the first step
$"),
    ("agent-auth", "\
> users get logged out after five minutes even with remember me ticked
⏺ Search(pattern: \"SESSION_LIFETIME|remember\", path: \"config\")
  ⎿  Found 3 files
⏺ config/session.php sets 'lifetime' => env('SESSION_LIFETIME', 5), and the remember-me
  cookie is never queued because Auth::attempt is called without its second argument.
⏺ Bash(php artisan test --filter=LoginTest)
  ⎿  Running…
✶ Checking the login flow… (esc to interrupt)"),
    ("docker", "\
$ docker compose up
[+] Running 3/3
 ✔ Container app-redis-1     Started
 ✔ Container app-postgres-1  Started
 ✔ Container app-mailpit-1   Started
postgres-1  | LOG:  database system is ready to accept connections
redis-1     | * Ready to accept connections tcp
postgres-1  | LOG:  checkpoint complete: wrote 44 buffers"),
    ("pnpm", "\
$ pnpm install
Lockfile is up to date, resolution step is skipped
Packages: +812
++++++++++++++++++++++++++++++++++++++++++++++++++++++
Progress: resolved 812, reused 790, downloaded 22, added 812, done
dependencies:
+ vue 3.5.13
+ @tauri-apps/api 2.2.0
Done in 6.1s"),
];

/// A query, and the screen it is about (`None`: about none of them).
const QUERIES: &[(&str, Option<&str>)] = &[
    ("stripe webhook", Some("agent-stripe")),
    ("payment signature verification", Some("agent-stripe")),
    ("dev server", Some("vite")),
    ("hot reload", Some("vite")),
    ("failing test", Some("cargo")),
    ("worktree removal bug", Some("cargo")),
    ("header layout on mobile", Some("agent-css")),
    ("responsive navigation", Some("agent-css")),
    ("database migration", Some("migrate")),
    ("refunds table", Some("migrate")),
    ("recent commits", Some("shell")),
    ("login session expiry", Some("agent-auth")),
    ("users logged out", Some("agent-auth")),
    ("postgres container", Some("docker")),
    ("installing dependencies", Some("pnpm")),
    ("kubernetes deploy", None),
    ("email newsletter template", None),
    ("translation strings", None),
    ("image upload resizing", None),
    ("search ranking", None),
];

fn percentile(values: &mut [f32], p: f32) -> f32 {
    values.sort_by(f32::total_cmp);
    values[((values.len() - 1) as f32 * p).round() as usize]
}

/// Shells printing a few of `SCREENS`, searched as the palette would.
fn live(cache: PathBuf, model: &str) {
    let terminals = Terminals::default();
    let root = std::env::temp_dir().join(format!("parallelist-live-{}", std::process::id()));
    let mut paths = Vec::new();
    for (name, text) in SCREENS.iter().take(6) {
        let dir = root.join(name);
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let file = dir.join("screen.txt");
        std::fs::write(&file, text).unwrap();
        let path = dir.to_string_lossy().to_string();
        let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
        cmd.cwd(&dir);
        cmd.env("PS1", "$ ");
        let home = Home { project_id: "p".into(), branch: name.to_string(), kind: WorkspaceKind::Worktree };
        let size = portable_pty::PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 };
        terminals.open(home, &path, size, cmd, Box::new(|_| true)).unwrap();
        terminals.write(&path, b"clear; cat screen.txt\n").unwrap();
        paths.push((path, *name));
    }
    std::thread::sleep(Duration::from_millis(500));
    let name_of = |path: &str| paths.iter().find(|(p, _)| p == path).map_or("?", |(_, n)| *n);

    let search = ScreenSearch::new(cache, Box::new(|status| println!("  status: {status:?}")));
    search.configure(Some(model), false);
    // The first search starts the worker and is literal only.
    let started = Instant::now();
    let found = search.search("payment signature", &terminals.screens());
    println!("first search, worker starting: {:?}, {} literal hits", started.elapsed(), found.hits.len());
    while !matches!(search.status(), screen_search::ModelStatus::Ready { .. } | screen_search::ModelStatus::Failed { .. }) {
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("worker ready after {:?}", started.elapsed());
    for query in ["payment signature", "payment signature verification", "failing test", "header layout on mobile", "refunds", "kubernetes deploy"] {
        let started = Instant::now();
        let found = search.search(query, &terminals.screens());
        println!("\n{query:?} in {:?}", started.elapsed());
        for hit in &found.hits {
            println!("  {:.2} {:14} {}", hit.relevance, name_of(&hit.path), hit.snippet);
        }
    }
    for (path, _) in &paths {
        terminals.close(path);
    }
    std::fs::remove_dir_all(&root).ok();
}

fn main() {
    // This binary is the worker too, in `live`, as the app's is.
    if let Some(code) = screen_search::serve_from_args() {
        std::process::exit(code);
    }
    if std::env::args().nth(1).as_deref() == Some("live") {
        let cache = PathBuf::from(std::env::args().nth(2).expect("usage: screen_search live <model cache dir> [model id]"));
        let model = std::env::args().nth(3).unwrap_or_else(|| screen_search::DEFAULT_MODEL.into());
        return live(cache, &model);
    }
    let mut args = std::env::args().skip(1);
    let cache = PathBuf::from(args.next().expect("usage: screen_search <model cache dir> [model id…]"));
    let wanted: Vec<String> = args.collect();
    let screens: Vec<Screen> = SCREENS
        .iter()
        .map(|(name, text)| Screen { path: name.to_string(), text: text.to_string() })
        .collect();
    let queries: Vec<&str> = QUERIES.iter().map(|(q, _)| *q).collect();

    for model in MODELS.iter().filter(|m| wanted.is_empty() || wanted.iter().any(|w| w == m.id)) {
        let started = Instant::now();
        let sims = match similarities(model.id, cache.clone(), &queries, &screens) {
            Ok(sims) => sims,
            Err(e) => {
                println!("{}: {e}", model.id);
                continue;
            }
        };
        println!("\n=== {} ({}) · {:.1}s", model.id, model.name, started.elapsed().as_secs_f32());
        let mut right = Vec::new();
        let mut wrong = Vec::new();
        let mut misses = 0;
        for ((query, target), row) in QUERIES.iter().zip(&sims) {
            let mut ranked: Vec<(&str, f32)> = SCREENS.iter().map(|(n, _)| *n).zip(row.iter().copied()).collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (name, sim) in &ranked {
                if Some(*name) == *target { right.push(*sim) } else { wrong.push(*sim) }
            }
            if target.is_some() && ranked[0].0 != target.unwrap() {
                misses += 1;
            }
            let shown: Vec<String> = ranked
                .iter()
                .take(3)
                .map(|(n, s)| format!("{}{n}={s:.2}", if Some(*n) == *target { "*" } else { "" }))
                .collect();
            println!("  {query:32} {}", shown.join("  "));
        }
        println!(
            "  first place wrong: {misses}/{}\n  right screen: min {:.2} median {:.2}\n  other screens: median {:.2} p90 {:.2} p97 {:.2} max {:.2}\n  set: floor {:.2}, ceiling {:.2}",
            QUERIES.iter().filter(|(_, t)| t.is_some()).count(),
            percentile(&mut right.clone(), 0.0),
            percentile(&mut right.clone(), 0.5),
            percentile(&mut wrong.clone(), 0.5),
            percentile(&mut wrong.clone(), 0.9),
            percentile(&mut wrong.clone(), 0.97),
            percentile(&mut wrong.clone(), 1.0),
            model.floor_and_ceiling().0,
            model.floor_and_ceiling().1,
        );
    }
}
