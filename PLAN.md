# Fabio — project plan

A fast, calm Postgres & SQLite client. Browse tables, write SQL with real autocomplete,
read query plans like a human, and let an agent query the database while you
watch what it does.

Mascot: Fabio the marmot — sits by the burrow, whistles when something is wrong.

## Why this exists

- Most database clients are either paid per device with a capped free tier, or
  do everything and feel like it — slow, dense, hard to use.
- Nobody does **Postgres + agent + readable performance** well. That is the gap;
  the table browser is table stakes.

## Principles

1. **Postgres first, SQLite second, nothing else.** Postgres sets the design;
   SQLite gets browse, query and agent, not the performance tooling. Every
   further engine is a tax on the whole codebase.
2. **Restraint over coverage.** Whitespace, one accent colour, few features done well.
   When in doubt, leave it out.
3. **Fast is a feature with numbers** (see budgets). Measured in CI, not felt.
4. **Read-only by default.** Writes are an explicit mode, per tab.
5. **The agent is visible.** Every query an agent runs appears in the UI with its
   plan, duration and row count. No hidden database access.
6. **Test first.** Core logic is TDD'd against a real Postgres, not mocks.

## Performance budgets (fail CI if exceeded)

| Metric | Budget |
|---|---|
| Cold start to usable window | < 300 ms |
| Idle memory | < 150 MB |
| Open a table (first page, 100 rows) | < 100 ms on localhost |
| Scroll 100k-row result | 60 fps, no blank rows |
| Bundle size | < 20 MB |

## Stack

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri 2** | System webview, small binary, Rust backend; same frontend cost as Electron |
| Core | Rust — `tokio-postgres` + `deadpool-postgres` | Owns connections; frontend never sees credentials |
| SQLite | `rusqlite` (bundled SQLite) | Open a file, no server; `?mode=ro` makes read-only real |
| SQL parsing | `pg_query` crate (libpg_query) for Postgres; `sqlparser-rs` (SQLite dialect) for SQLite | The real Postgres parser — no regex guessing; pg_query can't parse SQLite |
| Secrets | OS keychain (`keyring` crate) | No passwords in config files |
| Frontend | React + TypeScript + Vite | Already known; lowest ramp |
| Grid | `@tanstack/react-virtual` over DOM rows | Only on-screen rows exist; ~15 KB vs Glide's ~200 KB, which the start budget can't afford. Revisit Glide if 100k-row scroll misses 60 fps |
| Editor | CodeMirror 6 | Light, extensible, good SQL mode |
| Autocomplete | schema-aware completer (v1) → `postgres-language-server` sidecar (v2) | Ship simple first, swap in LSP when the simple one hurts |
| Formatter | `sql-formatter` (v1) | Preserves comments; `pg_query` deparse drops them |
| Plan viewer | PEV2 (Dalibo) | Best EXPLAIN visualiser that exists; embed, don't rebuild |
| Agent | MCP server binary sharing the core crate | Claude Code / pi drive it; no LLM vendor baked in |
| Tests | `cargo test` against `dev/pg.sh` Postgres; Vitest; Playwright for smoke | Real database in every integration test; testcontainers once CI needs it |
| Dev data | **Chinook** (artists/albums/tracks) — Postgres via `dev/pg.sh` (project-local cluster, port 54329) + the SQLite file | Same data in both engines = one fixture, two drivers, directly comparable tests |
| Load data | Pagila (Postgres) | Bigger, more FKs — for feeling slowness in Phase 3 |

## Architecture

```
fabio/
  crates/
    fabio-core/     connections, introspection, paging, explain, audit log
      engine/       `Engine` trait + postgres.rs + sqlite.rs
    fabio-mcp/      MCP server binary (stdio) — uses fabio-core
  app/
    src-tauri/      Tauri commands — thin wrappers over fabio-core
    src/            React UI
  dev/
    pg.sh                project-local Postgres 18 (Homebrew), loads Chinook
    data/                Chinook_PostgreSql.sql + Chinook_Sqlite.sqlite
```

- **A narrow `Engine` trait**, from day 1 but only as wide as what both engines
  genuinely share: connect, list objects, describe table, page rows, run
  statement, cancel, explain (as raw text/JSON). Autocomplete dialect and the
  performance tooling are **not** behind the trait — they're per-engine modules.
  Resist widening it; a leaky "generic database" layer is how clients get slow and dense.
- **SQLite differences to design for, not discover:** dynamic typing (a column
  declared INTEGER can hold text — render the value's actual type), no schemas
  (attached databases instead), `rowid` for paging when there's no PK, no
  `EXPLAIN ANALYZE` (only `EXPLAIN QUERY PLAN`), and a busy file may be locked
  by another writer (WAL mode helps; show a clear "database is locked" state).

- **One core, two front doors.** The desktop app and the MCP server both call
  `fabio-core`. The agent works even when the app is closed.
- **Shared audit log** — SQLite in the app data dir. Every statement (human or
  agent) is logged with source, SQL, duration, rows, error. The UI tails it; that
  is how agent activity becomes visible.
- **Paging:** `LIMIT/OFFSET`, ordered by the sort column then the PK so pages are
  stable. Keyset pagination is deferred until a real table makes deep offsets
  slow — Chinook can't show the difference. Results stream to the UI over Tauri channels in chunks.
- **Cancellation:** every running query has a cancel token (`pg_cancel_backend`
  via the client's cancel handle). Esc cancels.

## Phases

Each phase ends with something usable daily. Don't start the next until the
current one's exit criteria hold.

### Phase 0 — Spike (1 evening) — ✅ done 2026-09-26, see BENCHMARKS.md
- Tauri 2 scaffold, connect to dev Postgres, run `SELECT 1`, show result.
- Measure cold start and bundle size on this Mac. Record them in `BENCHMARKS.md`.
- **Exit:** numbers exist. If Tauri misses the start budget here, stop and rethink.

### Phase 1 — Browse (weekend 1) — ✅ built 2026-09-26; seen working on screen: sidebar, estimates, paged grid. Not yet eyeballed: sort, filters, Structure, connection form, drag-drop
- Connection manager: add/edit/delete, test connection, secrets in keychain, SSL modes.
- Sidebar: schemas → tables/views/matviews, with row estimates (`pg_class.reltuples`).
- Table view: paged grid, sort by column, simple filter row (`col op value`).
- Correct rendering: NULL vs empty, jsonb (collapsed, expandable), arrays,
  timestamptz in local tz, numeric without float loss, bytea as hex preview.
- Structure tab: columns, types, defaults, indexes, FKs, constraints.
- **SQLite:** open a file (drag-drop or ⌘O), recent files list, same sidebar and
  grid. Opened read-only unless you switch the tab to write mode.
- Pager under the grid: page N of M, total rows, jump to page — infinite scroll
  stays. Totals are exact `count(*)` capped at 2 s; past that Postgres shows the
  planner estimate. Results over 500k rows scroll within a window that moves on
  jumps (WebKit can't lay out a 130M px tall element).
- **Known limit:** counts share the connection with page loads, so scrolling
  waits behind a slow count (≤ 2 s). Give counts their own connection if it bites.
- **Exit:** you'd reach for Fabio instead of `psql` / `sqlite3` to look at a table.

### Phase 2 — Query (weekend 2)
- Tabs (unlimited — the whole point). Each tab has its own connection/session.
- Editor: CodeMirror 6, run statement under cursor (⌘↵), run all (⇧⌘↵).
- Autocomplete v1: keywords + schemas/tables/columns from an introspection cache,
  alias-aware via `pg_query` parse of the current statement.
- Format (⌥⇧F), comment toggle, query history (from the audit log), saved snippets.
- Errors shown with position highlighted in the editor.
- **Exit:** you'd write a real query here instead of in your old client.

### Phase 3 — Performance (weekend 3) — the differentiator
- "Explain" button → `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` in a rolled-back
  transaction → PEV2 view. Plain `EXPLAIN` for writes.
- Plan diff: run twice (e.g. before/after an index) and compare.
- Insights page per database:
  - top queries from `pg_stat_statements` (total time, mean, calls)
  - unused indexes, tables with high seq scans, bloat estimate
  - live activity + blocking locks from `pg_stat_activity` / `pg_locks`
- **SQLite gets only** `EXPLAIN QUERY PLAN` as an indented tree, with full scans
  (`SCAN` without an index) highlighted. No stats views exist to build more on.
- **Exit:** you can answer "why is this slow?" without leaving Fabio.

### Phase 4 — Agent (weekend 4)
- `fabio-mcp` tools: `list_schemas`, `describe_table`, `query` (read-only),
  `explain`, `top_queries`, `sample_rows`.
- Guardrails enforced in the core, not in the prompt:
  - dedicated connection with `default_transaction_read_only = on`
  - `statement_timeout` (default 10 s), hard row cap (default 500)
  - SQLite: opened with `mode=ro` + `PRAGMA query_only = ON`; row cap and a
    progress-handler timeout replace `statement_timeout`
  - connection allowlist — the agent only sees connections marked "agent OK"
- Agent panel in the UI: live feed of agent statements from the audit log,
  click one to open its result and plan in a tab.
- **Exit:** Claude Code debugs a slow query on the dev DB and you watch every step.

### Phase 5 — Polish & open source (ongoing)
- Cell editing (PK required; generates `UPDATE`, shows it, runs in a transaction).
- Export CSV/JSON, copy as INSERT/markdown.
- Light/dark themes, keyboard-first everything, command palette (⌘K).
- SSH tunnel.
- CI: GitHub Actions `tauri-action` builds for macOS (signed + notarized),
  Windows, Linux (AppImage). Linux uses WebKitGTK — test it, expect rough edges.
- Licence: MIT or Apache-2.0.

## Non-goals

Databases other than Postgres and SQLite. Performance tooling for SQLite beyond
the query plan. ER diagrams. Schema migration tooling. Team/cloud sync.
A built-in chat UI with its own LLM keys (MCP covers it; revisit after Phase 4).

## Risks

| Risk | Mitigation |
|---|---|
| Grid polish eats months | Glide Data Grid; read-only until Phase 5 |
| Autocomplete never feels right | LSP sidecar in v2 instead of growing the homemade one |
| Linux webview quirks | Mac first; Linux is best-effort until someone asks |
| Scope creep into "every feature, but pretty" | Non-goals list; a feature has to beat `psql` for you |
| Agent runs something harmful | Read-only enforced at the connection, not trusted to the model |

## Name

Working name **Fabio**; repo/binary **`fabiodb`** before going public —
`fabio` alone collides with the existing fabiolb load balancer.
