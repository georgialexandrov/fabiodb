# Fabio

Postgres + SQLite desktop client. Tauri 2 + Rust core + React. Plan and phases: `PLAN.md`.

## Rules

- Any user-visible text follows `VOICE.md` (who Fabio is, how he talks).

- Postgres and SQLite only. Keep the `Engine` trait narrow — autocomplete and
  performance tooling are per-engine modules, not generic abstractions.
- Test first. Core logic is tested against a real Postgres (`dev/pg.sh start`,
  Chinook fixture in both engines), not mocks.
- Tauri commands stay thin — logic lives in `crates/fabio-core` so the MCP server
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

- `dev/pg.sh start|stop|psql|reset` — local Postgres on :54329 with Chinook
- `cargo test -p fabio-core` — core tests (needs dev Postgres running)
- `cd app && pnpm test` — frontend unit tests (Vitest)
- `cargo run -p fabio-core --example explain -- "<sql>"` — print plan findings against dev
- `cd app && pnpm tauri dev` — run the app
- `cd app && pnpm tauri build --bundles app && python3 bench/startup.py` — budgets
- Rust comes from Homebrew rustup: `export PATH=/opt/homebrew/opt/rustup/bin:$PATH`
