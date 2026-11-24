import type { AppConfig, OpenTarget, TargetCategory, TargetSuggestion } from '../api'
import { setHomeDirectory } from '../format'
import { backend } from './backend'
import { boot } from './seed'
import { config } from './fixtures'

/**
 * `suggest_targets` and `adopt_targets` as the backend answers them, on a
 * Mac with a typical set of programs (`TYPICAL`) or with nothing but what
 * macOS ships (`BARE`), for a config that has not had the first-run choice.
 */

const HOME = '/Users/sandbox'

type Row = [TargetCategory, string, string, string, string, OpenTarget['kind'], boolean]

/** `targets.rs`'s catalog, with each command as it reads when found bare. */
const CATALOG: Row[] = [
  ['agent', 'claude', 'Claude Code', '✳', 'claude', 'terminal', true],
  ['agent', 'codex', 'Codex', '◎', 'codex', 'terminal', true],
  ['agent', 'gemini', 'Gemini CLI', '✦', 'gemini', 'terminal', true],
  ['agent', 'copilot', 'Copilot CLI', '⦿', 'copilot', 'terminal', true],
  ['agent', 'cursor-agent', 'Cursor Agent', '⬡', 'cursor-agent', 'terminal', true],
  ['agent', 'opencode', 'opencode', '◇', 'opencode', 'terminal', true],
  ['agent', 'amp', 'Amp', '▲', 'amp', 'terminal', true],
  ['agent', 'aider', 'aider', '◆', 'aider', 'terminal', true],
  ['editor', 'vscode', 'VS Code', '⌘', "open -a 'Visual Studio Code' {path}", 'command', true],
  ['editor', 'cursor', 'Cursor', '⌘', 'open -a Cursor {path}', 'command', true],
  ['editor', 'zed', 'Zed', '⌘', 'open -a Zed {path}', 'command', true],
  ['editor', 'windsurf', 'Windsurf', '⌘', 'open -a Windsurf {path}', 'command', true],
  ['editor', 'sublime-text', 'Sublime Text', '⌘', "open -a 'Sublime Text' {path}", 'command', true],
  ['editor', 'phpstorm', 'PhpStorm', '⌘', 'open -a PhpStorm {path}', 'command', true],
  ['editor', 'intellij', 'IntelliJ IDEA', '⌘', "open -a 'IntelliJ IDEA' {path}", 'command', true],
  ['editor', 'webstorm', 'WebStorm', '⌘', 'open -a WebStorm {path}', 'command', true],
  ['editor', 'pycharm', 'PyCharm', '⌘', 'open -a PyCharm {path}', 'command', true],
  ['editor', 'goland', 'GoLand', '⌘', 'open -a GoLand {path}', 'command', true],
  ['editor', 'rubymine', 'RubyMine', '⌘', 'open -a RubyMine {path}', 'command', true],
  ['editor', 'rustrover', 'RustRover', '⌘', 'open -a RustRover {path}', 'command', true],
  ['editor', 'clion', 'CLion', '⌘', 'open -a CLion {path}', 'command', true],
  ['editor', 'rider', 'Rider', '⌘', 'open -a Rider {path}', 'command', true],
  ['terminal', 'ghostty', 'Ghostty', '▶', 'open -na Ghostty --args --working-directory={path}', 'command', true],
  ['terminal', 'iterm', 'iTerm', '▶', 'open -a iTerm {path}', 'command', true],
  ['terminal', 'warp', 'Warp', '▶', 'open -a Warp {path}', 'command', true],
  ['terminal', 'wezterm', 'WezTerm', '▶', 'open -na WezTerm --args start --cwd {path}', 'command', true],
  ['terminal', 'kitty', 'kitty', '▶', 'open -na kitty --args --directory {path}', 'command', true],
  ['terminal', 'alacritty', 'Alacritty', '▶', 'open -na Alacritty --args --working-directory {path}', 'command', true],
  ['terminal', 'terminal-app', 'Terminal', '▶', 'open -a Terminal {path}', 'command', true],
  ['git', 'github-desktop', 'GitHub Desktop', '⑂', "open -a 'GitHub Desktop' {path}", 'command', false],
  ['git', 'fork', 'Fork', '⑂', 'open -a Fork {path}', 'command', false],
  ['git', 'tower', 'Tower', '⑂', 'open -a Tower {path}', 'command', false],
  ['git', 'sublime-merge', 'Sublime Merge', '⑂', "open -a 'Sublime Merge' {path}", 'command', false],
  ['other', 'finder', 'Finder', '📁', 'open {path}', 'command', false],
]

/** What each machine has, by id: where it was found. */
export const TYPICAL: Record<string, string> = {
  claude: `${HOME}/.local/bin/claude`,
  codex: '/opt/homebrew/bin/codex',
  vscode: '/Applications/Visual Studio Code.app',
  phpstorm: `${HOME}/Applications/PhpStorm.app`,
  ghostty: '/Applications/Ghostty.app',
  iterm: '/Applications/iTerm.app',
  'terminal-app': '/System/Applications/Utilities/Terminal.app',
  fork: '/Applications/Fork.app',
  finder: '/usr/bin/open',
}

export const BARE: Record<string, string> = {
  'terminal-app': '/System/Applications/Utilities/Terminal.app',
  finder: '/usr/bin/open',
}

/** As `targets::suggest` answers: only the agents and Finder found are ticked. */
function suggestions(machine: Record<string, string>): TargetSuggestion[] {
  return CATALOG.map(([category, id, label, icon, command, kind, primary]) => {
    const found = machine[id] ?? null
    return {
      target: { id, label, icon, command, kind, primary, runOnCreate: false },
      category,
      found,
      configured: false,
      recommended: !!found && (category === 'agent' || id === 'finder'),
    }
  })
}

/** A config that has not had the choice yet; `projects` empty for a first
 *  launch, as the onboarding shows it. */
function fresh(withProjects: boolean): AppConfig {
  return config({
    openTargets: [],
    targetsChosen: false,
    ...(withProjects ? {} : { projects: [], selectedProjectId: null }),
  })
}

export async function firstRun(
  machine: Record<string, string>,
  options: { projects?: boolean; loading?: boolean } = {},
) {
  setHomeDirectory(HOME)
  const cfg = fresh(!!options.projects)
  await boot({ config: cfg })
  backend.handle({
    // Starting an interactive shell takes about this long.
    suggest_targets: () =>
      options.loading
        ? new Promise(() => {})
        : new Promise((done) => setTimeout(() => done(suggestions(machine)), 700)),
    adopt_targets: ({ targets }: { targets: OpenTarget[] }) => {
      for (const t of targets) {
        const i = cfg.openTargets.findIndex((o) => o.id === t.id)
        if (i >= 0) cfg.openTargets[i] = t
        else cfg.openTargets.push(t)
      }
      cfg.targetsChosen = true
      return structuredClone(cfg)
    },
  })
}
