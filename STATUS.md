# Fabio — status and handoff

Read this first in a new session, then `CLAUDE.md` (rules), `PLAN.md` (phases,
budgets, decisions) and `VOICE.md` (any text users see). Last updated
2026-09-26, after commit `a3ac9ac`.

## What Fabio is

A fast, calm desktop client for **Postgres and SQLite**. It's built for
home projects. The goal is restraint plus two things other tools
don't have: **readable performance** (plans with plain-language findings,
Insights) and, next, an **agent that queries through Fabio while you watch**.
Mascot: Fabio the marmot, who watches the burrow and whistles once.

## Stack and layout

Tauri 2 · Rust core · React 19 + TypeScript + Vite · CodeMirror 6 · `@tanstack/react-virtual`.

```
crates/fabio-core/      all database logic; the MCP server reuses it
  src/lib.rs            Db (enum over engines), shared types, Error, Canceller
  src/postgres.rs       tokio-postgres + native-tls; read-only by default
  src/sqlite.rs         rusqlite (bundled); opened read-only, reopened for writes
  src/sql.rs            page/count statement builder (identifiers only from describe)
  src/plan.rs           EXPLAIN → PlanNode tree (both engines) + findings
  src/insights.rs       pg_stat_activity / pg_stat_statements / index + scan stats
  src/audit.rs          AuditLog: every human/agent statement in SQLite (WAL)
  src/store.rs          saved connections JSON (passwords never written here)
  src/agent.rs          ReadOnlyDb (guardrails) + Agent (allowlist, audit) for MCP
crates/fabio-mcp/       stdio MCP server binary; tools.rs = schemas + result text
  tests/                integration tests against Chinook in BOTH engines
  examples/explain.rs   print findings for a statement against dev Postgres
app/src-tauri/src/lib.rs  Tauri commands: thin wrappers + Keychain + sessions + audit
app/src/                  React UI (App, TableView, Grid, QueryTab, SqlEditor,
                          PlanView, InsightsView, ConnectionForm, Structure, api.ts)
dev/pg.sh               project-local Postgres 18 on :54329 (Chinook + perf.big 5M rows)
dev/data/               Chinook_PostgreSql.sql, Chinook_Sqlite.sqlite
bench/startup.py        cold start / idle memory / bundle size against budgets
design/                 icon source (fabio-1.png), icon-1024.png on the Apple grid, prompt
```

## Running it

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH   # already in ~/.zshrc
dev/pg.sh start                                   # needed by core tests and the app's Chinook PG
cargo test -p fabio-core                          # 83 tests, ~4 s
cargo test -p fabio-mcp                           # 7 tests, drives the binary over stdio
cd app && pnpm test                               # 11 Vitest tests (statement splitter)
cd app && pnpm tauri dev                          # run with hot reload
cd app && pnpm tauri build --bundles app && python3 ../bench/startup.py   # release + budgets
```

Two Chinook connections (Postgres and SQLite) are pre-saved in
`~/Library/Application Support/dev.fabio.app/connections.json`. The audit log is
`audit.sqlite` in the same folder. `chinook-pg` is marked `"agent": true`.

Register the MCP server with Claude Code (it reads that folder and the keychain;
`FABIO_DIR` overrides the folder):

```sh
cargo build -p fabio-mcp --release
claude mcp add fabio -- "$PWD/target/release/fabio-mcp"
```

## Done

### Phase 0 — Spike ✅
Tauri shell, core engine, Chinook in both engines, benchmark harness.

### Phase 1 — Browse ✅
- **Connections:** add/edit/delete, Test, paste a `postgres://` URL to fill the
  fields, SSL disable/prefer/require (no certificate check, like libpq),
  passwords in the macOS Keychain (`keyring` 3.x). SQLite via ⌘O, file dialog or
  drag-and-drop.
- **Sidebar:** connections (a green dot means open) and tables grouped by schema,
  with row estimates.
- **Table view:** virtualized, random-access grid; pages of 100 load for the
  visible range. Pager shows page N of M, total rows, and jump to page.
  Counts are exact, capped at 2 s; past that Postgres cancels and shows its
  estimate. Results over 500k rows scroll inside a window that moves on jumps
  (WebKit layout limit).
- **Grid:** header sort (asc → desc → off), AND-ed filters (=, ≠, <, ≤, >, ≥,
  contains, is null) sent as parameters, cell select + ⌘C, double-click opens a
  value panel (pretty JSON).
- **Structure tab:** columns, indexes and foreign keys; click a foreign key to open that table.
- Hidden title bar, app icon, marmot on the empty state.

### Phase 2 — Query ✅ (except snippets)
- Tabs per connection: tables, queries, Insights. ⌘T new query, ⌘W close.
  Every tab stays mounted, so state survives switching.
- **Each query tab has its own connection** (session, transaction, write mode).
- **Editor:** CodeMirror, lazy-loaded so startup is unaffected. Live-schema
  autocomplete via `@codemirror/lang-sql`, ⌘↵ runs the statement under the cursor
  or the selection, ⇧⌘↵ runs all (one statement at a time, stops at the first
  error), ⌥⇧F formats (`sql-formatter`, lazy), line wrapping.
- **Errors** are underlined at the position Postgres reports.
- **Esc cancels** (Postgres cancel request / SQLite interrupt).
- **Write mode** per tab (a red edge while on). Everything is read-only by
  default: Postgres `default_transaction_read_only`, SQLite opened read-only.
- **Row cap** of 10k rows; the rest is cancelled on the server, and the UI says so.
- **History** (⌘Y) from the audit log.

### Phase 4 — Agent 🟡 (built; exit run with Claude Code left)
- **Guardrails in the core** (`agent.rs`, tested against both engines,
  mutation-checked): Postgres statements run one at a time (prepare rejects a
  second) inside `BEGIN READ ONLY … ROLLBACK`, must start with SELECT / WITH /
  VALUES / TABLE / SHOW / EXPLAIN, `statement_timeout` 10 s, 500-row cap,
  `application_name = 'fabio agent'`. SQLite: read-only open, `query_only`
  before each statement, no ATTACH, 10 s interrupt, 500-row cap.
- **Allowlist:** `SavedConnection.agent` ("Agents can query" in the connection
  form). Read fresh on every call, so unticking takes effect immediately.
- **`fabio-mcp`:** stdio server, tools `list_connections`, `list_tables`,
  `describe_table` (bare or `schema.table`), `sample_rows`, `query`,
  `explain` (findings, then the plan one node per line), `insights`.
  Every `query`/`explain` is audited as `Source::Agent`, refusals included.
- **Agent panel:** app polls `AuditLog::agent_since` every 2 s while visible.
  Sidebar line "Agent ran N queries on X · ms" (last 10 min) opens the
  connection's Agent tab; a statement there opens in a query tab and runs or
  explains again.

### Phase 3 — Performance ✅ (except side-by-side plan diff)
- **Explain:** ⌘E = `EXPLAIN (ANALYZE, BUFFERS, VERBOSE, FORMAT JSON)` inside a
  transaction that is **always rolled back**; ⇧⌘E = estimate only. SQLite =
  `EXPLAIN QUERY PLAN` tree, with whole-table `SCAN` steps in red.
- **Plan view:** findings on top (a hot one gets the red bar), node tree with a
  self-time bar, rows (actual vs estimate, flagged at ≥10×), details, own
  buffers, "before → now ×faster" when re-explaining, Copy raw.
- **Findings** (core, one per node, VOICE-style): where the time goes (≥60% of
  ≥50 ms), filtered seq scans (with the columns to index), row misestimates.
- **Insights tab** (Postgres, refreshes every 3 s): running sessions and blockers,
  top statements (click to open in a query tab), tables read by full scans,
  unused indexes.

## Verified vs not

**Seen working on screen:** sidebar, estimates, paged grid, pager, Postgres and
SQLite browsing, query run + highlighting, error underline, autocomplete, plan
view + finding on `perf.big`, SQLite plan, Insights.

**Phase 4 seen on screen:** Agent tab (live entries from the MCP process, a
refused write in red), sidebar summary, opening an agent EXPLAIN in a tab.
Not yet clicked: the "Agents can query" checkbox (the flag was set in the JSON).

**Built and covered by tests, not yet clicked through by a human:** sort, filters,
Structure tab + foreign-key jump, connection form (save/edit/delete, URL paste,
Keychain), drag-and-drop, write-mode toggle, history panel, Esc cancel in the UI,
page jump on the 5M-row table, before → now comparison.

## Left

### Phase 2 leftovers
- Saved snippets.
- Autocomplete noise (obscure Postgres keywords); alias-aware completion via
  `pg_query` or the `postgres-language-server` sidecar if lang-sql isn't enough.

### Phase 3 leftovers
- Side-by-side plan diff (today: a before → now time line only).

### Phase 4 — Agent: exit run left
- **Exit criterion not yet met:** register the server (above), ask Claude Code
  to find out why `select count(*) from perf.big where bucket = 7` is slow, and
  watch the Agent tab. The agent can't create the index; that's for the human.
- Nice to have: the single whistle (VOICE "Sound") when an agent tries to write.
- The keychain may ask once to let `fabio-mcp` read a password the app saved
  (different binary). Chinook has no password, so the dev setup doesn't show it.

### Phase 5 — Polish / open source
Cell editing (PK required, show the UPDATE, run in a transaction), CSV/JSON
export, ⌘K command palette, SSH tunnel, SSL `verify-full`, signed builds
(macOS notarized, Windows, Linux AppImage via `tauri-action`), licence, README.

### Cross-cutting
- **CI:** `dev/check.sh` runs everything locally (`--budgets` adds the release
  build and fails over budget). `.github/workflows/ci.yml` does the same on
  Ubuntu with Postgres 18 in Docker, plus the bundle size on macOS — **never
  run yet: the repo has no remote.** Expect to fix a thing or two on its first run.
- **Counts share the browse connection** with page loads, so scrolling waits
  behind a slow count (≤ 2 s). Give counts their own connection if it bites.
- **Keyset paging** instead of OFFSET for deep pages on huge tables.
- **Idle memory:** 190 MB RSS against a 150 MB budget, but RSS overcounts
  shared WebKit pages. Physical footprint: app 27 MB + WebKit GPU 59 MB; the
  WebContent process needs sudo to measure. Budget undecided.
- **Startup:** 283 ms median against 300 ms. Keep heavy UI lazy (the editor and
  formatter already are).
- **Cargo crates** have no 7-day release quarantine (npm does, via `app/.npmrc`).

## Decisions that differ from the first plan (all recorded in PLAN.md)
- Grid: `@tanstack/react-virtual` instead of Glide Data Grid (bundle size vs start budget).
- Plan viewer: own React tree instead of PEV2 (Vue + Bootstrap, can't follow VOICE).
- Paging: OFFSET for now; keyset deferred.
- Dev Postgres: `dev/pg.sh` (Homebrew Postgres 18, no Docker on this machine).
- Autocomplete v1: `@codemirror/lang-sql` with the live schema.
- Browse calls (page/count/describe) are **not** audited; only statements a
  human types or an agent runs.

## Gotchas learned the hard way
- **A read-only transaction is not a sandbox.** As superuser, `COPY … TO
  PROGRAM` runs a shell inside `BEGIN READ ONLY`. Hence the statement-kind check.
- UI automation: another app's window can sit over Fabio. Capture only Fabio's
  window (`screencapture -l <CGWindowID>`), and click only after checking Fabio
  is frontmost.
- `tokio_postgres::Error`'s Display is just "db error". Use `as_db_error()` (done in `From`).
- Postgres allows **temp-table writes in read-only transactions**, so test
  read-only with real tables.
- `EXPLAIN` JSON includes `Schema` only with `VERBOSE`.
- Under `Gather`, workers overlap: time per loop is wall time, don't multiply by
  loops. Rows removed are per loop, so multiply those.
- **pg_stat_statements strips a leading comment.** Internal SQL is tagged
  `SELECT /* fabio */ …` (after the first keyword) and Insights filters on it.
  Keep the tag on any new internal query.
- ORDER BY a bare column name binds to the `::text` select alias and sorts as
  text. Page statements qualify ORDER BY with the table.
- `keyring` 4.x changed its API. Stay on 3.x.
- The npm quarantine (`minimum-release-age=10080`) blocked a Vite dependency
  that was 42 h old. Vite is pinned at 8.3.0; fresh-install if the lockfile pulls
  something young.
- UI automation from the shell: screenshots are 2× (divide pixel coordinates by
  2, window at +331,+128), `click at` sometimes fails with -25204 on the first
  try (retry), and a `keystroke … using command down` can leave ⌘ held for the
  next typing. Use separate calls and cap each `osascript` with
  `perl -e 'alarm 8; exec @ARGV'`.

## Working conventions
- TDD against real databases (Chinook in both engines); core first, then
  command, then UI; then look at it on screen.
- Commit per coherent step, messages via file, with the `Co-Authored-By` line.
- Update `PLAN.md` phase status and this file when a phase moves.
