#!/usr/bin/env bash
# Everything CI runs, locally. Needs dev Postgres (dev/pg.sh start).
#
#   dev/check.sh            format, lint, types, all tests
#   dev/check.sh --budgets  also build the release app and check PLAN.md budgets
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

step() { printf '\n== %s\n' "$*"; }

step "rustfmt";  cargo fmt --check
step "frontend"; (cd app && pnpm exec tsc --noEmit && pnpm test)
# The app crate embeds app/dist; build it so clippy can compile the app.
[ -d app/dist ] || (cd app && pnpm build >/dev/null)
step "clippy";   cargo clippy --workspace --all-targets -- -D warnings
step "tests";    cargo test -p fabio-core -p fabio-mcp

if [ "${1:-}" = "--budgets" ]; then
  step "budgets"
  (cd app && pnpm tauri build --bundles app >/dev/null)
  python3 bench/startup.py
fi
printf '\nAll checks passed.\n'
