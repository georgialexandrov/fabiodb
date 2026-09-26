# Fabio

Postgres + SQLite desktop client. Tauri 2 + Rust core + React.

**Start here:** `STATUS.md` (what's done, what's left, gotchas), then `PLAN.md`
(phases, budgets) and `VOICE.md` (any user-visible text).

## Rules

- Any user-visible text follows `VOICE.md` (who Fabio is, how he talks).

- Postgres and SQLite only. Keep the `Engine` trait narrow — autocomplete and
  performance tooling are per-engine modules, not generic abstractions.
- Test first. Core logic is tested against a real Postgres (`dev/pg.sh start`,
  Chinook fixture in both engines), not mocks.
- Tauri commands stay thin — logic lives in `crates/fabiodb-core` so the MCP server
  shares it.
- Agent access is read-only, enforced at the connection (`default_transaction_read_only`,
  `statement_timeout`, row cap). Never rely on the prompt for safety.
- Internal Postgres SQL carries `/* fabio */` right after the first keyword
  (`SELECT /* fabio */ …`) — pg_stat_statements strips a leading comment, and
  Insights filters on the tag.
- Every statement a human types or an agent runs goes through the audit log
  (`AuditLog`). Browsing (page/count/describe) doesn't — it would drown history.
- Performance budgets in `PLAN.md` are hard limits; record measurements in `BENCHMARKS.md`.
- Finish the current phase's exit criteria before starting the next.

## Commands

- `dev/check.sh [--budgets]` — everything CI runs (fmt, tsc, Vitest, clippy -D warnings, tests); `--budgets` builds the release app and fails over budget
- `dev/pg.sh start|stop|psql|reset|load` — local Postgres on :54329 with Chinook (`load`: into any running server, as CI does)
- `cargo test -p fabiodb-core` — core tests (needs dev Postgres running)
- `cd app && pnpm test` — frontend unit tests (Vitest)
- `cargo run -p fabiodb-core --example explain -- "<sql>"` — print plan findings against dev
- `cd app && pnpm tauri dev` — run the app
- `cd app && pnpm tauri build --bundles app && python3 bench/startup.py` — budgets
- Rust comes from Homebrew rustup: `export PATH=/opt/homebrew/opt/rustup/bin:$PATH`
