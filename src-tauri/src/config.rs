//! Persisted configuration.
//!
//! Two layers: Parallelist's own config file (`config.json` in the app config
//! dir) and an optional `.parallelist.json` committed at the root of a repo.
//! The repo file wins field by field, so a project can ship its own setup
//! recipe without stopping you from overriding the parts you care about.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "config.json";
pub const PROJECT_CONFIG_FILE: &str = ".parallelist.json";
/// The name the project file had while the app was called Switcheroo. Repos
/// that committed one can't be rewritten by the app, so it is still read
/// when there is no `.parallelist.json`.
pub const LEGACY_PROJECT_CONFIG_FILE: &str = ".switcheroo.json";
/// The app config dir's name while the app was called Switcheroo: a sibling
/// of the current one, whose `config.json` is carried over on first launch.
pub const LEGACY_CONFIG_DIR: &str = "com.jmversteeg.switcheroo";

/// A step offered in the create-workspace dialog. Every step is opt-in at
/// creation time; `default_enabled` only decides how the checkbox starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStep {
    pub id: String,
    pub label: String,
    #[serde(default = "yes")]
    pub default_enabled: bool,
    /// Shell command run inside the new workspace. `{primary}`, `{path}` and
    /// `{branch}` are substituted and shell-quoted.
    ///
    /// Deliberately the only kind of step there is: copying files and
    /// symlinking are one-line shell commands, and having them as separate
    /// step types bought nothing a `cp` or `ln -s` doesn't say more clearly.
    ///
    /// Defaulted so a hand-written `.parallelist.json` can omit it.
    #[serde(default)]
    pub script: String,
}

fn yes() -> bool {
    true
}

/// Settings that a project's own `.parallelist.json` may override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverrides {
    pub base_branch: Option<String>,
    pub path_template: Option<String>,
    pub setup_steps: Option<Vec<SetupStep>>,
    pub open_targets: Option<Vec<OpenTarget>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub id: String,
    /// Display name in the sidebar.
    pub name: String,
    /// The main workspace: the project's own clone. New worktrees hang off
    /// this one and setup steps copy files out of it.
    pub path: String,
    /// Falls back to `origin/HEAD` when unset.
    pub base_branch: Option<String>,
    /// Supports `{parent}`, `{project}`, `{slug}`, `{branch}`, `{pr}`.
    pub path_template: String,
    pub setup_steps: Vec<SetupStep>,
    /// Extra "open in" targets for this project only, appended to the global
    /// ones. A monolith that wants a Docker shell shouldn't force that entry
    /// onto every other project.
    #[serde(default)]
    pub open_targets: Vec<OpenTarget>,
}

/// A user-defined "open in" action. `command` is a shell template; the
/// placeholders are substituted before it is handed to the login shell, or,
/// for `OpenKind::Terminal`, typed into the workspace's own terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTarget {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub command: String,
    #[serde(default)]
    pub kind: OpenKind,
    /// Shown as a button on the workspace row rather than only in the menu.
    #[serde(default)]
    pub primary: bool,
    /// Hidden unless the selected branch has a pull request (for `{prUrl}`).
    #[serde(default)]
    pub requires_pr: bool,
    /// Starts ticked in the create-workspace dialog, which runs the ticked
    /// targets once the new workspace is set up.
    #[serde(default)]
    pub run_on_create: bool,
}

/// Where an open target's command runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenKind {
    /// Detached, through the login shell: it launches something else — an
    /// editor, another terminal app.
    #[default]
    Command,
    /// Typed into the workspace's built-in terminal, which the app then
    /// shows. Unavailable while that terminal already has a shell running,
    /// which the frontend decides: the command would land in whatever
    /// that shell is doing.
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    #[serde(default = "default_open_targets")]
    pub open_targets: Vec<OpenTarget>,
    #[serde(default = "default_fetch_interval")]
    pub fetch_interval_secs: u64,
    #[serde(default = "yes")]
    pub fetch_on_focus: bool,
    #[serde(default = "default_pr_interval")]
    pub pr_refresh_secs: u64,
    /// A branch counts as "active" if it was committed to within this window.
    #[serde(default = "default_active_days")]
    pub active_days: i64,
    #[serde(default)]
    pub selected_project_id: Option<String>,
    /// Pane widths in px. Persisted here rather than in web storage so the
    /// layout survives anything that clears the webview.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f64,
    /// The branch list. The detail pane takes whatever the window has left,
    /// so resizing the window resizes only that. `None` until first set; the
    /// frontend then derives it from `detail_width`.
    #[serde(default)]
    pub list_width: Option<f64>,
    /// What used to be stored instead of `list_width`, when the detail pane
    /// had the fixed width. Kept only to seed it, so an existing layout opens
    /// as it was left.
    #[serde(default = "default_detail_width")]
    pub detail_width: f64,
    /// Shared by every terminal, so zooming one zooms them all.
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: f64,
}

fn default_sidebar_width() -> f64 {
    216.0
}

fn default_detail_width() -> f64 {
    360.0
}

pub fn default_terminal_font_size() -> f64 {
    12.0
}

fn default_fetch_interval() -> u64 {
    180
}
fn default_pr_interval() -> u64 {
    120
}
fn default_active_days() -> i64 {
    30
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            open_targets: default_open_targets(),
            fetch_interval_secs: default_fetch_interval(),
            fetch_on_focus: true,
            pr_refresh_secs: default_pr_interval(),
            active_days: default_active_days(),
            selected_project_id: None,
            sidebar_width: default_sidebar_width(),
            list_width: None,
            detail_width: default_detail_width(),
            terminal_font_size: default_terminal_font_size(),
        }
    }
}

/// Shipped defaults. These are ordinary editable entries, not special cases —
/// the settings pane can rewrite or delete any of them and add new ones.
pub fn default_open_targets() -> Vec<OpenTarget> {
    vec![
        OpenTarget {
            id: "terminal".into(),
            label: "Terminal".into(),
            icon: "\u{25b6}".into(),
            command: "open -na Ghostty --args --working-directory={path}".into(),
            kind: OpenKind::Command,
            primary: true,
            requires_pr: false,
            run_on_create: false,
        },
        OpenTarget {
            id: "editor".into(),
            label: "PhpStorm".into(),
            icon: "\u{2318}".into(),
            command: "phpstorm {path}".into(),
            kind: OpenKind::Command,
            primary: true,
            requires_pr: false,
            run_on_create: false,
        },
        OpenTarget {
            id: "claude".into(),
            label: "Claude Code".into(),
            icon: "\u{2733}".into(),
            command: "claude".into(),
            kind: OpenKind::Terminal,
            primary: true,
            requires_pr: false,
            run_on_create: false,
        },
        OpenTarget {
            id: "finder".into(),
            label: "Finder".into(),
            icon: "\u{1f4c1}".into(),
            command: "open {path}".into(),
            kind: OpenKind::Command,
            primary: false,
            requires_pr: false,
            run_on_create: false,
        },
    ]
}

/// Guess a useful setup recipe by looking at what the repo actually contains.
/// Runs once, when the project is added; everything stays editable afterwards.
pub fn sniff_setup_steps(project: &Path) -> Vec<SetupStep> {
    let mut steps = Vec::new();
    let exists = |p: &str| project.join(p).exists();

    // Only untracked ones. A tracked `.env.cypress` is committed content that
    // can differ between branches, and copying the main workspace's copy over
    // it does not merely dirty the new workspace — it silently reverts
    // whatever that branch committed.
    const ENV_CANDIDATES: [&str; 5] = [
        ".env",
        ".env.local",
        ".env.cypress",
        ".env.phpunit",
        ".env.testing",
    ];
    let tracked = crate::git::tracked_files(project, &ENV_CANDIDATES);
    let env_files: Vec<String> = ENV_CANDIDATES
        .iter()
        .filter(|f| exists(f) && !tracked.contains(**f))
        .map(|f| f.to_string())
        .collect();
    if !env_files.is_empty() {
        steps.push(SetupStep {
            id: "copy-env".into(),
            label: "Copy untracked env files from the main workspace".into(),
            default_enabled: true,
            // Guarded, because the old CopyFiles step skipped absent files and
            // a bare `cp a b c .` fails outright if one of them is missing.
            script: format!(
                "for f in {}; do [ -e {{primary}}/$f ] && cp {{primary}}/$f .; done",
                env_files.join(" ")
            ),
        });
    }

    let linkable: Vec<String> = ["vendor", "node_modules"]
        .iter()
        .filter(|d| exists(d))
        .map(|d| d.to_string())
        .collect();
    if !linkable.is_empty() {
        steps.push(SetupStep {
            id: "symlink-deps".into(),
            label: "Symlink dependencies from the main workspace".into(),
            default_enabled: false,
            // -f so re-running is harmless, -n so an existing link to a
            // directory is replaced rather than linked inside.
            script: format!(
                "for d in {}; do [ -e {{primary}}/$d ] && ln -sfn {{primary}}/$d .; done",
                linkable.join(" ")
            ),
        });
    }

    if exists("composer.json") {
        steps.push(SetupStep {
            id: "composer".into(),
            label: "composer install".into(),
            default_enabled: false,
            script: "composer install".into(),
        });
    }
    if exists("package.json") {
        let script = if exists("pnpm-lock.yaml") {
            "pnpm install"
        } else if exists("yarn.lock") {
            "yarn install"
        } else {
            "npm install"
        };
        steps.push(SetupStep {
            id: "node-deps".into(),
            label: script.into(),
            default_enabled: false,
            script: script.into(),
        });
    }
    steps
}

impl AppConfig {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(CONFIG_FILE);
        if !path.exists() {
            Self::adopt_legacy(dir);
        }
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Copy `config.json` over from the Switcheroo-era config dir, so the
    /// rename doesn't start the user from an empty project list. Copied, not
    /// moved, so an old build still launches with its settings.
    fn adopt_legacy(dir: &Path) {
        let Some(legacy) = dir.parent().map(|p| p.join(LEGACY_CONFIG_DIR).join(CONFIG_FILE)) else {
            return;
        };
        if legacy.is_file() && std::fs::create_dir_all(dir).is_ok() {
            let _ = std::fs::copy(legacy, dir.join(CONFIG_FILE));
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(CONFIG_FILE), json).map_err(|e| e.to_string())
    }

    pub fn project(&self, id: &str) -> Option<&ProjectConfig> {
        self.projects.iter().find(|p| p.id == id)
    }
}

impl ProjectConfig {
    pub fn new(path: PathBuf, name: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            setup_steps: sniff_setup_steps(&path),
            path: path.to_string_lossy().to_string(),
            base_branch: None,
            path_template: "{parent}/{project}-{slug}".into(),
            open_targets: Vec::new(),
        }
    }

    /// Apply `.parallelist.json` (or a legacy `.switcheroo.json`) from the
    /// repo root on top of the stored config.
    pub fn resolved(&self) -> ProjectConfig {
        let mut out = self.clone();
        let root = Path::new(&self.path);
        let Some(text) = [PROJECT_CONFIG_FILE, LEGACY_PROJECT_CONFIG_FILE]
            .iter()
            .find_map(|name| std::fs::read_to_string(root.join(name)).ok())
        else {
            return out;
        };
        let Ok(over) = serde_json::from_str::<ProjectOverrides>(&text) else {
            return out;
        };
        if let Some(v) = over.base_branch {
            out.base_branch = Some(v);
        }
        if let Some(v) = over.path_template {
            out.path_template = v;
        }
        if let Some(v) = over.setup_steps {
            out.setup_steps = v;
        }
        if let Some(v) = over.open_targets {
            out.open_targets = v;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("parallelist-config-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn carries_the_config_over_from_switcheroo() {
        let root = scratch("legacy");
        let legacy = root.join(LEGACY_CONFIG_DIR);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join(CONFIG_FILE), r#"{"fetchIntervalSecs": 77}"#).unwrap();

        let dir = root.join("com.jmversteeg.parallelist");
        assert_eq!(AppConfig::load(&dir).fetch_interval_secs, 77);
        // Copied, so the old directory is left as it was.
        assert!(legacy.join(CONFIG_FILE).is_file());

        // Once there is a config of its own, the old one is no longer read.
        std::fs::write(dir.join(CONFIG_FILE), r#"{"fetchIntervalSecs": 5}"#).unwrap();
        assert_eq!(AppConfig::load(&dir).fetch_interval_secs, 5);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn reads_a_switcheroo_project_file_only_without_a_parallelist_one() {
        let root = scratch("project");
        let project = ProjectConfig {
            base_branch: None,
            ..ProjectConfig::new(root.clone(), "p".into())
        };
        std::fs::write(root.join(LEGACY_PROJECT_CONFIG_FILE), r#"{"baseBranch": "old"}"#).unwrap();
        assert_eq!(project.resolved().base_branch.as_deref(), Some("old"));

        std::fs::write(root.join(PROJECT_CONFIG_FILE), r#"{"baseBranch": "new"}"#).unwrap();
        assert_eq!(project.resolved().base_branch.as_deref(), Some("new"));
        std::fs::remove_dir_all(&root).ok();
    }
}
