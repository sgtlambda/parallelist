//! "Open in" targets the app can suggest, and which of them this machine has.
//!
//! The shipped targets used to be one setup's (Ghostty, PhpStorm, Claude
//! Code), so most users started with buttons that ran nothing. Now a first
//! run starts with none and offers this catalog instead, each entry resolved
//! against what is installed, with the ones found ticked. A suggestion
//! carries a complete `OpenTarget`: once adopted it is an ordinary target,
//! edited and deleted like any other, and nothing remembers where it came
//! from.
//!
//! Finding a program is done the way the command will run it, because a
//! target whose program isn't found when clicked is the button-that-runs-
//! nothing this replaces:
//!
//! - A `command` target runs through `$SHELL -lc`, which reads `.zprofile`
//!   but not `.zshrc`. A program found only on an interactive shell's `PATH`
//!   (nvm, and installers that edit `.zshrc`) is written by its full path.
//! - A `terminal` target is typed into a built-in terminal, an interactive
//!   login shell, so a program on that `PATH` is written by name.
//! - A program in a well-known install directory on neither `PATH` is
//!   written by its full path, for both kinds.
//!
//! Apps are found by bundle name in the usual folders rather than through
//! Launch Services: `open -Ra` reveals the app in Finder, and asking
//! Spotlight fails quietly on a machine with indexing off.

use crate::config::{OpenKind, OpenTarget};
use crate::proc;
use serde::Serialize;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What a suggestion is for, so a list of them can be grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// A program run in the workspace's built-in terminal.
    Agent,
    Editor,
    /// Another terminal app. Only one of these is ticked: a user with three
    /// installed opens workspaces in one of them.
    Terminal,
    Git,
    Other,
}

/// One way of running an entry. An entry's ways are tried in order and the
/// first one installed wins.
enum Way {
    /// A program, with the command template that runs it: `{bin}` is the
    /// program as the command should name it. The template may leave
    /// `{bin}` out when the program is only what proves the entry works
    /// (`open`, for Finder).
    Bin(&'static str, &'static str),
    /// A macOS app by bundle name, without `.app`: `{app}` is that name.
    App(&'static str, &'static str),
}

struct Entry {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    category: Category,
    kind: OpenKind,
    primary: bool,
    ways: Vec<Way>,
}

/// One catalog entry as this machine has it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// Ready to add as it is. When nothing was found, its command names the
    /// entry's first program or app as if it were installed, so it can
    /// still be added by hand.
    pub target: OpenTarget,
    pub category: Category,
    /// The program or app found, as a path. Absent when nothing was.
    pub found: Option<String>,
    /// A target the user already has does this: same id, or the same
    /// command (`phpstorm {path}` is the PhpStorm entry, whatever its id).
    pub configured: bool,
    /// Ticked when offered: found, not configured yet, and for a terminal
    /// app, the first found while none is configured.
    pub recommended: bool,
}

fn catalog() -> Vec<Entry> {
    use Category::*;
    let entry = |id, label, icon, category, kind, primary, ways| Entry {
        id,
        label,
        icon,
        category,
        kind,
        primary,
        ways,
    };
    let agent = |id, label, icon, bin| {
        entry(id, label, icon, Agent, OpenKind::Terminal, true, vec![Way::Bin(bin, "{bin}")])
    };
    // An editor opens the folder through its app when it can: no command
    // line tool needs installing, and `open` returns at once where some
    // launchers stay alive for the whole session.
    let editor = |id, label, apps: &[&'static str], bin| {
        let mut ways: Vec<Way> = apps.iter().map(|a| Way::App(a, "open -a {app} {path}")).collect();
        ways.push(Way::Bin(bin, "{bin} {path}"));
        entry(id, label, "\u{2318}", Editor, OpenKind::Command, true, ways)
    };
    let terminal = |id, label, app, template| {
        entry(id, label, "\u{25b6}", Terminal, OpenKind::Command, true, vec![Way::App(app, template)])
    };
    let git = |id, label, app, bin: Option<&'static str>| {
        let mut ways = vec![Way::App(app, "open -a {app} {path}")];
        ways.extend(bin.map(|b| Way::Bin(b, "{bin} {path}")));
        entry(id, label, "\u{2442}", Git, OpenKind::Command, false, ways)
    };

    vec![
        agent("claude", "Claude Code", "\u{2733}", "claude"),
        agent("codex", "Codex", "\u{25ce}", "codex"),
        agent("gemini", "Gemini CLI", "\u{2726}", "gemini"),
        agent("copilot", "Copilot CLI", "\u{29bf}", "copilot"),
        agent("cursor-agent", "Cursor Agent", "\u{2b21}", "cursor-agent"),
        agent("opencode", "opencode", "\u{25c7}", "opencode"),
        agent("amp", "Amp", "\u{25b2}", "amp"),
        agent("aider", "aider", "\u{25c6}", "aider"),
        editor("vscode", "VS Code", &["Visual Studio Code"], "code"),
        editor("cursor", "Cursor", &["Cursor"], "cursor"),
        editor("zed", "Zed", &["Zed", "Zed Preview"], "zed"),
        editor("windsurf", "Windsurf", &["Windsurf"], "windsurf"),
        editor("sublime-text", "Sublime Text", &["Sublime Text"], "subl"),
        editor("phpstorm", "PhpStorm", &["PhpStorm"], "phpstorm"),
        editor(
            "intellij",
            "IntelliJ IDEA",
            &["IntelliJ IDEA", "IntelliJ IDEA Ultimate", "IntelliJ IDEA CE", "IntelliJ IDEA Community Edition"],
            "idea",
        ),
        editor("webstorm", "WebStorm", &["WebStorm"], "webstorm"),
        editor(
            "pycharm",
            "PyCharm",
            &["PyCharm", "PyCharm Professional Edition", "PyCharm CE", "PyCharm Community Edition"],
            "pycharm",
        ),
        editor("goland", "GoLand", &["GoLand"], "goland"),
        editor("rubymine", "RubyMine", &["RubyMine"], "rubymine"),
        editor("rustrover", "RustRover", &["RustRover"], "rustrover"),
        editor("clion", "CLion", &["CLion"], "clion"),
        editor("rider", "Rider", &["Rider"], "rider"),
        // In order of preference: only the first one found is ticked, and
        // Terminal is always there.
        terminal("ghostty", "Ghostty", "Ghostty", "open -na {app} --args --working-directory={path}"),
        terminal("iterm", "iTerm", "iTerm", "open -a {app} {path}"),
        terminal("warp", "Warp", "Warp", "open -a {app} {path}"),
        terminal("wezterm", "WezTerm", "WezTerm", "open -na {app} --args start --cwd {path}"),
        terminal("kitty", "kitty", "kitty", "open -na {app} --args --directory {path}"),
        terminal("alacritty", "Alacritty", "Alacritty", "open -na {app} --args --working-directory {path}"),
        // Not "terminal": that is the id the old shipped Ghostty target had,
        // and an id match counts as configured.
        terminal("terminal-app", "Terminal", "Terminal", "open -a {app} {path}"),
        git("github-desktop", "GitHub Desktop", "GitHub Desktop", Some("github")),
        git("fork", "Fork", "Fork", None),
        git("tower", "Tower", "Tower", Some("gittower")),
        git("sublime-merge", "Sublime Merge", "Sublime Merge", Some("smerge")),
        entry(
            "finder",
            "Finder",
            "\u{1f4c1}",
            Other,
            OpenKind::Command,
            false,
            vec![Way::Bin("open", "open {path}")],
        ),
    ]
}

/// Where programs and apps are looked for.
pub struct Probe {
    /// The login shell's `PATH`, which `command` targets run with.
    pub login: Vec<PathBuf>,
    /// An interactive login shell's `PATH`, which the built-in terminals
    /// have: `.zshrc` is where nvm and many installers add theirs.
    pub interactive: Vec<PathBuf>,
    /// Where installers put programs without always getting them onto
    /// either `PATH`.
    pub fallback: Vec<PathBuf>,
    /// Folders holding `.app` bundles.
    pub apps: Vec<PathBuf>,
}

impl Probe {
    /// This machine's. Starts an interactive shell, so it can take as long
    /// as the user's `.zshrc` does: call it off the main thread.
    pub fn system() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let under_home = |dirs: &[&str]| -> Vec<PathBuf> {
            home.iter().flat_map(|h| dirs.iter().map(move |d| h.join(d))).collect()
        };
        let login = proc::login_path()
            .map(str::to_string)
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let mut fallback = under_home(&[".local/bin", ".claude/local", ".bun/bin", ".npm-global/bin", ".cargo/bin"]);
        fallback.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
        let mut apps: Vec<PathBuf> = [
            "/Applications",
            "/Applications/Utilities",
            "/Applications/Setapp",
            "/System/Applications",
            "/System/Applications/Utilities",
        ]
        .map(PathBuf::from)
        .into();
        apps.extend(under_home(&["Applications", "Applications/JetBrains Toolbox"]));
        Self {
            login: split_path(&login),
            interactive: split_path(&interactive_path(home.as_deref()).unwrap_or_default()),
            fallback,
            apps,
        }
    }

    /// Where `name` is, and whether a command of `kind` can name it bare.
    fn program(&self, name: &str, kind: OpenKind) -> Option<(PathBuf, bool)> {
        let find = |dirs: &[PathBuf]| dirs.iter().map(|d| d.join(name)).find(|p| executable(p));
        find(&self.login)
            .map(|p| (p, true))
            .or_else(|| find(&self.interactive).map(|p| (p, kind == OpenKind::Terminal)))
            .or_else(|| find(&self.fallback).map(|p| (p, false)))
    }

    fn app(&self, name: &str) -> Option<PathBuf> {
        self.apps.iter().map(|d| d.join(format!("{name}.app"))).find(|p| p.is_dir())
    }

    /// The first of an entry's ways that is installed: its command and
    /// what was found.
    fn resolve(&self, entry: &Entry) -> Option<(String, PathBuf)> {
        entry.ways.iter().find_map(|way| match *way {
            Way::Bin(name, template) => self.program(name, entry.kind).map(|(path, bare)| {
                let word = if bare { proc::sh_word(name) } else { proc::sh_word(&path.to_string_lossy()) };
                (template.replace("{bin}", &word), path)
            }),
            Way::App(name, template) => {
                self.app(name).map(|path| (template.replace("{app}", &proc::sh_word(name)), path))
            }
        })
    }
}

/// `PATH` as an interactive login shell has it, as in a built-in terminal.
/// Marked, since an interactive shell's startup files may print anything.
fn interactive_path(home: Option<&Path>) -> Option<String> {
    const MARK: &str = "__parallelist_path__";
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let script = format!("printf '\\n{MARK}%s\\n' \"$PATH\"");
    let out = proc::run_timeout(&shell, &["-ilc", &script], home, Duration::from_secs(10)).ok()?;
    out.stdout.lines().find_map(|l| l.strip_prefix(MARK)).map(str::to_string)
}

fn split_path(path: &str) -> Vec<PathBuf> {
    path.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).collect()
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Every way an entry's command could read, written by a person or by
/// [`Probe::resolve`] on another machine: each program and app named bare.
fn bare_commands(entry: &Entry) -> impl Iterator<Item = String> + '_ {
    entry.ways.iter().map(|way| match *way {
        Way::Bin(name, template) => template.replace("{bin}", &proc::sh_word(name)),
        Way::App(name, template) => template.replace("{app}", &proc::sh_word(name)),
    })
}

fn same_command(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

/// The catalog as `probe` finds this machine, next to the targets the user
/// already has.
pub fn suggest(probe: &Probe, configured: &[OpenTarget]) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = catalog()
        .iter()
        .map(|entry| {
            let resolved = probe.resolve(entry);
            let command = resolved
                .as_ref()
                .map(|(command, _)| command.clone())
                .or_else(|| bare_commands(entry).next())
                .unwrap_or_default();
            let is_configured = configured.iter().any(|t| {
                t.id == entry.id
                    || same_command(&t.command, &command)
                    || bare_commands(entry).any(|c| same_command(&t.command, &c))
            });
            Suggestion {
                target: OpenTarget {
                    id: entry.id.into(),
                    label: entry.label.into(),
                    icon: entry.icon.into(),
                    command,
                    kind: entry.kind,
                    primary: entry.primary,
                    run_on_create: false,
                },
                category: entry.category,
                found: resolved.map(|(_, path)| path.to_string_lossy().into_owned()),
                configured: is_configured,
                recommended: false,
            }
        })
        .collect();

    let mut terminal_taken = out.iter().any(|s| s.category == Category::Terminal && s.configured);
    for s in &mut out {
        s.recommended = s.found.is_some() && !s.configured;
        if s.category == Category::Terminal {
            s.recommended &= !terminal_taken;
            terminal_taken |= s.recommended;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("parallelist-targets-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn dir(&self, name: &str) -> PathBuf {
            let dir = self.0.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn program(&self, dir: &str, name: &str) -> PathBuf {
            let path = self.dir(dir).join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        }

        fn app(&self, name: &str) -> PathBuf {
            let path = self.dir("Applications").join(format!("{name}.app"));
            std::fs::create_dir_all(&path).unwrap();
            path
        }

        fn probe(&self) -> Probe {
            Probe {
                login: vec![self.dir("login")],
                interactive: vec![self.dir("interactive")],
                fallback: vec![self.dir("fallback")],
                apps: vec![self.dir("Applications")],
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn find<'a>(all: &'a [Suggestion], id: &str) -> &'a Suggestion {
        all.iter().find(|s| s.target.id == id).unwrap_or_else(|| panic!("no {id}"))
    }

    #[test]
    fn the_catalog_is_well_formed() {
        let entries = catalog();
        let mut ids: Vec<_> = entries.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "duplicate ids");
        for entry in &entries {
            assert!(!entry.ways.is_empty(), "{} has no way to run it", entry.id);
            for command in bare_commands(entry) {
                assert!(!command.contains("{bin}") && !command.contains("{app}"), "{command}");
                assert_eq!(entry.kind == OpenKind::Command, command.contains("{path}"), "{command}");
            }
        }
    }

    #[test]
    fn a_program_is_named_as_the_shell_that_runs_it_finds_it() {
        let s = Scratch::new("programs");
        s.program("login", "claude");
        let gemini = s.program("interactive", "gemini");
        let codex = s.program("fallback", "codex");
        let sublime = s.program("interactive", "subl");
        let all = suggest(&s.probe(), &[]);

        assert_eq!(find(&all, "claude").target.command, "claude");
        // Typed into a built-in terminal, which has the interactive PATH.
        assert_eq!(find(&all, "gemini").target.command, "gemini");
        assert_eq!(find(&all, "gemini").found.as_deref(), Some(&*gemini.to_string_lossy()));
        // On no PATH at all.
        assert_eq!(find(&all, "codex").target.command, proc::sh_word(&codex.to_string_lossy()));
        // Run through `-lc`, which hasn't read `.zshrc`.
        assert_eq!(
            find(&all, "sublime-text").target.command,
            format!("{} {{path}}", proc::sh_word(&sublime.to_string_lossy()))
        );
        assert!(find(&all, "claude").recommended);
    }

    #[test]
    fn a_file_that_cannot_run_is_not_a_program() {
        let s = Scratch::new("not-executable");
        let path = s.dir("login").join("claude");
        std::fs::write(&path, "").unwrap();
        s.dir("login/codex");
        let all = suggest(&s.probe(), &[]);
        for id in ["claude", "codex"] {
            assert_eq!(find(&all, id).found, None);
            assert!(!find(&all, id).recommended);
        }
        // Still offered, to add by hand.
        assert_eq!(find(&all, "claude").target.command, "claude");
    }

    #[test]
    fn an_editor_opens_through_its_app_before_its_launcher() {
        let s = Scratch::new("editors");
        s.app("Visual Studio Code");
        s.program("login", "code");
        s.app("IntelliJ IDEA Community Edition");
        s.program("login", "zed");
        let all = suggest(&s.probe(), &[]);
        assert_eq!(find(&all, "vscode").target.command, "open -a 'Visual Studio Code' {path}");
        assert_eq!(find(&all, "intellij").target.command, "open -a 'IntelliJ IDEA Community Edition' {path}");
        assert_eq!(find(&all, "zed").target.command, "zed {path}");
        // Not installed: offered as its first way, unticked.
        let phpstorm = find(&all, "phpstorm");
        assert_eq!((phpstorm.target.command.as_str(), phpstorm.recommended), ("open -a PhpStorm {path}", false));
    }

    #[test]
    fn only_the_first_terminal_app_found_is_ticked() {
        let s = Scratch::new("terminals");
        for app in ["Warp", "iTerm", "Terminal"] {
            s.app(app);
        }
        let all = suggest(&s.probe(), &[]);
        let ticked: Vec<_> = all
            .iter()
            .filter(|s| s.category == Category::Terminal && s.recommended)
            .map(|s| s.target.id.as_str())
            .collect();
        assert_eq!(ticked, ["iterm"]);
        assert!(find(&all, "terminal-app").found.is_some());
    }

    /// The targets parallelist shipped before this catalog, as they sit in
    /// existing configs under ids of their own.
    #[test]
    fn the_old_shipped_targets_count_as_configured() {
        let s = Scratch::new("configured");
        for app in ["Ghostty", "PhpStorm", "Terminal"] {
            s.app(app);
        }
        s.program("login", "claude");
        s.program("login", "open");
        let old = |id: &str, command: &str| OpenTarget {
            id: id.into(),
            label: id.into(),
            icon: String::new(),
            command: command.into(),
            kind: OpenKind::Command,
            primary: true,
            run_on_create: false,
        };
        let configured = [
            old("terminal", "open -na Ghostty --args --working-directory={path}"),
            old("editor", "phpstorm  {path}"),
            old("claude", "claude"),
            old("finder", "open {path}"),
        ];
        let all = suggest(&s.probe(), &configured);
        for id in ["ghostty", "phpstorm", "claude", "finder"] {
            assert!(find(&all, id).configured, "{id}");
            assert!(!find(&all, id).recommended, "{id}");
        }
        // A terminal app is configured, so none is ticked.
        assert!(!find(&all, "terminal-app").configured);
        assert!(!find(&all, "terminal-app").recommended);
    }
}
