//! Persisted configuration.
//!
//! Two layers: parallelist's own config file (`config.json` in the app config
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
    /// "Primary" in Settings: shown as a button on the workspace card
    /// rather than only in its menu.
    #[serde(default)]
    pub primary: bool,
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
    #[serde(default)]
    pub open_targets: Vec<OpenTarget>,
    /// Whether the first-run choice of "Open in" targets (`targets.rs`) has
    /// been made, or skipped. A new config starts without any targets and
    /// the frontend offers the choice while this is false.
    ///
    /// A config.json that predates the field defaults it to true: it was
    /// written by a build that shipped targets of its own, and its user has
    /// had them, and the Settings to change them, ever since.
    #[serde(default = "yes")]
    pub targets_chosen: bool,
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
    /// The colour theme's id (`src/themes.ts`). Each theme has a light and a
    /// dark palette, picked by the OS appearance. An id the frontend doesn't
    /// know falls back to its default, so this isn't checked here.
    #[serde(default = "default_theme")]
    pub theme: String,
}

pub fn default_theme() -> String {
    "graphite".into()
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
            open_targets: Vec::new(),
            targets_chosen: false,
            fetch_interval_secs: default_fetch_interval(),
            fetch_on_focus: true,
            pr_refresh_secs: default_pr_interval(),
            active_days: default_active_days(),
            selected_project_id: None,
            sidebar_width: default_sidebar_width(),
            list_width: None,
            detail_width: default_detail_width(),
            terminal_font_size: default_terminal_font_size(),
            theme: default_theme(),
        }
    }
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

    /// Take the targets chosen on first run and mark the choice made. Each
    /// replaces a target with its id where there is one, in place, and is
    /// added at the end otherwise, so choosing again never duplicates one.
    /// Skipping the choice adopts none.
    pub fn adopt_targets(&mut self, targets: Vec<OpenTarget>) {
        for target in targets {
            match self.open_targets.iter_mut().find(|t| t.id == target.id) {
                Some(slot) => *slot = target,
                None => self.open_targets.push(target),
            }
        }
        self.targets_chosen = true;
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

    /// The project file committed at the repo root, if there is one: its
    /// name and what it sets, or why it couldn't be read. `.parallelist.json`
    /// wins over a legacy `.switcheroo.json`.
    pub fn committed(&self) -> Option<Committed> {
        let root = Path::new(&self.path);
        let (file, text) = [PROJECT_CONFIG_FILE, LEGACY_PROJECT_CONFIG_FILE]
            .iter()
            .find_map(|name| Some((*name, std::fs::read_to_string(root.join(name)).ok()?)))?;
        let (overrides, error) = match serde_json::from_str::<ProjectOverrides>(&text) {
            Ok(over) => (over, None),
            Err(e) => (ProjectOverrides::default(), Some(e.to_string())),
        };
        Some(Committed { file: file.into(), overrides, error })
    }

    /// Apply `.parallelist.json` (or a legacy `.switcheroo.json`) from the
    /// repo root on top of the stored config. A file that doesn't parse
    /// changes nothing.
    pub fn resolved(&self) -> ProjectConfig {
        let mut out = self.clone();
        let Some(Committed { overrides: over, .. }) = self.committed() else {
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

/// A project file committed in the repo, as Settings shows it: the fields it
/// sets win over the ones stored in the app, so editing those there would
/// change nothing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Committed {
    pub file: String,
    pub overrides: ProjectOverrides,
    /// Set when the file isn't valid; it is then ignored as a whole.
    pub error: Option<String>,
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
    fn only_a_new_config_has_its_targets_still_to_choose() {
        let root = scratch("chosen");
        let fresh = AppConfig::load(&root.join("none"));
        assert!(!fresh.targets_chosen);
        assert!(fresh.open_targets.is_empty());

        // Written before the choice existed, with the targets shipped then.
        std::fs::write(root.join(CONFIG_FILE), r#"{"openTargets": []}"#).unwrap();
        assert!(AppConfig::load(&root).targets_chosen);

        fresh.save(&root).unwrap();
        assert!(!AppConfig::load(&root).targets_chosen, "a fresh config keeps asking until chosen");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn adopting_targets_replaces_by_id_and_appends_the_rest() {
        let target = |id: &str, command: &str| OpenTarget {
            id: id.into(),
            label: id.into(),
            icon: String::new(),
            command: command.into(),
            kind: OpenKind::Command,
            primary: false,
            run_on_create: false,
        };
        let mut config = AppConfig {
            open_targets: vec![target("a", "old"), target("b", "b")],
            ..AppConfig::default()
        };
        config.adopt_targets(vec![target("c", "c"), target("a", "new")]);
        let got: Vec<_> = config.open_targets.iter().map(|t| (t.id.as_str(), t.command.as_str())).collect();
        assert_eq!(got, [("a", "new"), ("b", "b"), ("c", "c")]);
        assert!(config.targets_chosen);

        let mut skipped = AppConfig::default();
        skipped.adopt_targets(Vec::new());
        assert!(skipped.targets_chosen && skipped.open_targets.is_empty());
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

    #[test]
    fn reports_what_a_project_file_sets_and_why_it_is_ignored() {
        let root = scratch("committed");
        let project = ProjectConfig::new(root.clone(), "p".into());
        assert!(project.committed().is_none());

        std::fs::write(root.join(PROJECT_CONFIG_FILE), r#"{"pathTemplate": "{parent}/x"}"#).unwrap();
        let committed = project.committed().unwrap();
        assert_eq!(committed.file, PROJECT_CONFIG_FILE);
        assert_eq!(committed.overrides.path_template.as_deref(), Some("{parent}/x"));
        assert!(committed.overrides.setup_steps.is_none());
        assert!(committed.error.is_none());

        // A file that doesn't parse sets nothing, and says so.
        std::fs::write(root.join(PROJECT_CONFIG_FILE), r#"{"pathTemplate": 3}"#).unwrap();
        let committed = project.committed().unwrap();
        assert!(committed.error.is_some());
        assert_eq!(project.resolved().path_template, project.path_template);
        std::fs::remove_dir_all(&root).ok();
    }
}
