#!/usr/bin/env bash
# Run parallelist. No arguments: a dev build with hot reload.
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

# A build from anything but main says which branch it is, in the title bar, so
# it can't be mistaken for the real thing. Vite hands VITE_* to the frontend.
branch=""
if [ -e .git ]; then
  branch=$(git symbolic-ref --short -q HEAD || git rev-parse --short HEAD || true)
  [ "$branch" = main ] && branch=""
fi
export VITE_PARALLELIST_BRANCH="$branch"

if [ "${1:-}" = "--build" ]; then
  pnpm tauri build
  app="src-tauri/target/release/bundle/macos/Parallelist.app"
  rm -rf "/Applications/Parallelist.app"
  cp -R "$app" /Applications/
  # `open` hands the app its caller's environment, so launching it from a
  # terminal gives it a PATH it will never see from the Dock — the exact reason
  # missing-`gh` bugs hide from terminal testing. `env -i` reproduces the
  # launchd environment instead; HOME is kept so it finds its config.
  env -i HOME="$HOME" PATH=/usr/bin:/bin open -na /Applications/Parallelist.app
  echo "installed and launched /Applications/Parallelist.app (launchd-like environment)"
else
  # A random free port rather than the fixed 1420, so a dev build can run from
  # several worktrees at once. Vite reads it from the environment (strictPort,
  # so a port taken in the meantime fails loudly); tauri gets it as devUrl.
  for _ in $(seq 50); do
    port=$((20000 + RANDOM % 10000))
    lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1 || break
  done
  echo "dev server on port $port"
  PARALLELIST_DEV_PORT=$port pnpm tauri dev \
    --config "{\"build\":{\"devUrl\":\"http://localhost:$port\"}}"
fi
