#!/usr/bin/env bash
# Run Switcheroo. No arguments: a dev build with hot reload.
#   ./run.sh            dev build, reloads as you edit
#   ./run.sh --build    release .app, installed to /Applications and launched
set -euo pipefail

cd "$(dirname "$0")"

# rustup was installed with --no-modify-path on this machine, so cargo is not
# necessarily on PATH even when Rust is present.
if [ -d "$HOME/.cargo/bin" ]; then PATH="$HOME/.cargo/bin:$PATH"; fi

need() {
  command -v "$1" >/dev/null 2>&1 && return 0
  echo "missing: $1 — install it with: $2" >&2
  return 1
}

ok=0
need pnpm  'brew install pnpm'                            || ok=1
need cargo 'curl https://sh.rustup.rs -sSf | sh'          || ok=1
need git   'xcode-select --install'                       || ok=1
[ "$ok" -eq 0 ] || exit 1

command -v gh >/dev/null 2>&1 ||
  echo "note: gh not found — the app runs, minus pull request data (brew install gh)" >&2

[ -d node_modules ] || pnpm install

if [ "${1:-}" = "--build" ]; then
  pnpm tauri build
  app="src-tauri/target/release/bundle/macos/Switcheroo.app"
  rm -rf "/Applications/Switcheroo.app"
  cp -R "$app" /Applications/
  # `open` hands the app its caller's environment, so launching it from a
  # terminal gives it a PATH it will never see from the Dock — the exact reason
  # missing-`gh` bugs hide from terminal testing. `env -i` reproduces the
  # launchd environment instead; HOME is kept so it finds its config.
  env -i HOME="$HOME" PATH=/usr/bin:/bin open -na /Applications/Switcheroo.app
  echo "installed and launched /Applications/Switcheroo.app (launchd-like environment)"
else
  pnpm tauri dev
fi
