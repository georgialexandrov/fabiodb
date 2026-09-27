# Fabio — status and handoff

Read this first in a new session, then `CLAUDE.md` (rules), `PLAN.md` (phases,
budgets, the v0.1 list) and `VOICE.md` (any text users see). Last updated
2026-09-27: licence, rename and bundle id settled.

## What Fabio is

A fast, calm desktop client for **Postgres and SQLite**. It's built for
home projects. The goal is restraint plus two things other tools
don't have: **readable performance** (plans with plain-language findings,
Insights) and an **agent that queries through Fabio while you watch**.
Mascot: Fabio the marmot, who watches the burrow and whistles once.

## Stack and layout

Tauri 2 · Rust core · React 19 + TypeScript + Vite · CodeMirror 6 · `@tanstack/react-virtual`.

```
crates/fabiodb-core/      all database logic; the app and the MCP server share it
  src/lib.rs            Db (enum over engines), shared types, Error, Canceller
  src/postgres.rs       tokio-postgres + native-tls (SSL modes, CA file); read-only by default
  src/sqlite.rs         rusqlite (bundled); opened read-only, reopened for writes
  src/tunnel.rs         SSH tunnels via the system ssh, shared per host
  src/sql.rs            page/count/select builder (identifiers only from describe)
  src/edit.rs           Changes (updates/deletes/inserts) → guarded statements + preview
  src/export.rs         RowWriter: CSV/TSV/JSON/Markdown/INSERT, streaming
  src/discover.rs       databases in a folder: Compose services (+ .env), SQLite files
  src/plan.rs           EXPLAIN → PlanNode tree (both engines) + findings
  src/insights.rs       pg_stat_activity / pg_stat_statements / index + scan stats
  src/audit.rs          AuditLog: every human/agent statement in SQLite (WAL)
  src/store.rs          connections.json and snippets.json (no passwords)
  src/agent.rs          ReadOnlyDb (guardrails) + Agent (allowlist, audit, create) for MCP
  tests/                integration tests against Chinook in BOTH engines
  tests/fixtures/       fake-ssh.py (tunnel tests)
crates/fabiodb-mcp/       stdio MCP server; tools.rs = schemas + result text
app/src-tauri/src/      lib.rs: thin commands, Keychain, sessions, reconnect; menu.rs
app/src/                React UI — App (workspaces, tabs, shortcuts), TableView, Grid,
                        QueryTab, SqlEditor, PlanView, InsightsView, AgentView,
                        ConnectionSwitcher, ConnectionForm, CommandPalette, ColumnFilter,
                        NewRow, DiscoverDialog, ExportMenu, ScrollBench, theme, api.ts
dev/pg.sh               project-local Postgres 18 on :54329 (Chinook, perf.big 5M rows, TLS)
dev/check.sh            everything CI runs; --budgets adds the release build + budgets
bench/startup.py        cold start, scroll fps, idle memory, bundle size against budgets
.github/workflows/      CI (not run yet: no remote)
```

## Running it

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH   # already in ~/.zshrc
dev/pg.sh start                                   # core tests and the app's Chinook PG
dev/check.sh                                      # fmt, tsc, 16 Vitest, clippy, 127 core + 8 MCP tests
dev/check.sh --budgets                            # + release build: start, scroll, bundle
dev/build.sh                                      # local release build, signed with "Fabio Local" (Keychain "Always Allow" sticks)
cd app && pnpm tauri dev                          # run with hot reload
```

Two Chinook connections are pre-saved in
`~/Library/Application Support/dev.alexandrov.fabio/connections.json`; `chinook-pg` is
open to agents. The audit log is `audit.sqlite`, snippets `snippets.json`, in
the same folder. `perf.fabio_demo` is a scratch table used to try cell editing.

MCP server for Claude Code (reads that folder and the keychain; `FABIO_DIR` overrides):

```sh
cargo build -p fabiodb-mcp --release
claude mcp add fabio -- "$PWD/target/release/fabiodb-mcp"
```

## Done

### Phases 0–3 ✅ (Spike, Browse, Query, Performance)
Connections (URL paste, Keychain, SSL), sidebar with estimates, virtualized
paged grid with sort, Structure tab with FK jumps; query tabs with their own
session, CodeMirror with live-schema completion, run statement/all, cancel,
write mode, 10k row cap, error underline, history; EXPLAIN ANALYZE (always
rolled back) with findings and plan tree, SQLite query plan, Insights.

### Phase 4 — Agent 🟡 (built; the Claude Code exit run is left)
- **Guardrails in the core:** Postgres statements run alone (prepare rejects a
  second) inside `BEGIN READ ONLY … ROLLBACK`, must start with SELECT / WITH /
  VALUES / TABLE / SHOW / EXPLAIN, 10 s `statement_timeout`, 500-row cap.
  SQLite: read-only, `query_only`, no ATTACH, 10 s interrupt, 500 rows.
- **Allowlist** (`agent` flag, "Agents can query"), read fresh on every call.
- **`fabiodb-mcp` tools:** list_connections, list_tables, describe_table,
  sample_rows, query, explain (findings + plan text), insights. Connections are
  created and explicitly opened to agents in the desktop app; MCP cannot grant
  itself new filesystem or network access.
- **Agent panel** + sidebar line; every agent statement is audited.

### Phase 5 / v0.1 (see PLAN.md "v0.1")
- **CI:** `dev/check.sh`; workflow written; budgets fail the check.
- **Export:** tables stream all matching rows (CSV/JSON/INSERT, 5M rows in
  1.3 s at 8 MB); results save/copy what's on screen, never re-run.
- **Editing:** cells inline (Enter/double-click), row details pane (⌘I), new
  rows (+ Row), delete rows (⌘⌫); all pending until ⌘S shows the statements;
  one transaction, each keyed on the whole primary key, updates guarded by the
  values seen — a changed or missing row rolls everything back and is named.
- **Grid:** ranges (shift), ⌘C as TSV, right-click copy as CSV/Markdown/JSON/INSERT,
  column filters (mark on every header, popover, chips), arrows, Space = value.
- **Navigation:** ⌘K palette (tables, connections, snippets, actions),
  connection switcher (⇧⌘K; open, recent, groups) that is also the start page,
  **⌘D databases** on the same server — each `connection#database` is its own
  workspace with its own tabs.
- **Docker/folders:** "Docker folder…" finds Compose Postgres services (with
  .env/env_file interpolation, ports, image defaults) and SQLite files.
- **Snippets:** ⌘S in a query tab, listed next to History, found with ⌘K.
- **Connections:** SSL verify-ca / verify-full + CA file; SSH tunnel via system ssh.
- **Resilience:** reconnect after sleep/network loss (browse retries silently,
  query tabs say so); tabs, query text, workspace and window restored on launch.
- **Look and feel:** dark mode (System/Light/Dark, ⇧⌘L), native menu bar with
  About box, window shown only once painted.
- **DBML and diagram (v0.2):** `Db::schema()` (5 pipelined catalog queries),
  `dbml()` validated against the reference parser, MCP tool `schema`; Diagram
  tab (⇧⌘D) with drag, zoom, Arrange, Export DBML…; `links.json` + macOS
  bookmarks (`app/src-tauri/src/bookmark.rs`, tested across a folder rename);
  layouts next to the DBML or in `layouts/`. Not yet seen on screen.
- **Speed:** views and dialogs lazy (main JS 262 KB), cold start 267–284 ms,
  scroll 100k rows at 60 fps with 0% dropped (DOM grid is enough; no canvas).

## Verified vs not

**Seen on screen:** browsing, queries, plans, Insights, the Agent tab and
summary, dark mode, cell editing end to end (edit, review, conflict refused,
reload, save, audited).

**Tests only, not yet clicked through:** column filters, row details pane, add
and delete rows, grid ranges and copy formats, ⌘K, snippets, connection
switcher and start page, ⌘D, Docker folder dialog, SSL/SSH form sections,
menu bar (check each shortcut fires once), layout restore, reconnect in the
app, export dialogs. The user was using the app during this work, so UI
automation stopped; these want a manual pass.

## Left

- **Phase 4 exit run** with Claude Code on `perf.big`.
- **Release (v0.1 item 13):** decided: MIT OR Apache-2.0, crates/binaries
  `fabiodb`, bundle id `dev.alexandrov.fabio`. Left: GitHub remote (then CI's
  first run), Apple Developer ID for signing and notarization.
- Agent role hardening: side-effect functions (`pg_terminate_backend`,
  `dblink`) are limited only by the role; recommend a non-superuser role.
- Smaller: plan diff side by side, autocomplete noise, keyset paging, counts on
  their own connection, idle memory budget (RSS 200 MB, overcounts WebKit),
  the agent-write whistle.

## Decisions that differ from the first plan (recorded in PLAN.md)
- Grid: `@tanstack/react-virtual` DOM grid, not Glide/canvas — measured 60 fps.
- Plan viewer: own React tree instead of PEV2.
- Paging: OFFSET; keyset deferred.
- MCP protocol hand-written (~100 lines), no SDK.
- SSH via the system `ssh`, not an embedded SSH library.
- Browse calls aren't audited; typed statements, saves and agent actions are.

## Gotchas learned the hard way
- **A read-only transaction is not a sandbox.** Agent connections reject
  superusers and server-file/program roles, in addition to the statement-kind
  check and `BEGIN READ ONLY` guard.
- **A hidden window gets no animation frames.** Showing it "after first paint"
  waited for a 1.5 s fallback (1717 ms start). Show after render, then measure paint.
- tokio-postgres `Config::port()` appends; set it once. Its non-server errors
  hide the reason in `source()` (now chained into the message).
- tokio-postgres knows no `verify-*` sslmode or `sslrootcert`; Fabio strips and
  handles them itself.
- Rust async closures can't promise `Send` futures yet; Tauri commands need
  `Fn(Arc<Db>) -> impl Future + Send` instead.
- The page sees ⌘ keys before the native menu; handlers must `preventDefault`,
  and app shortcuts skip keys the editor already handled (CodeMirror's ⌘D).
- No shared JS formatter config: don't run prettier on whole files.
- `tokio_postgres::Error`'s Display is "db error"; use `as_db_error()`.
- Postgres allows temp-table writes in read-only transactions: test with real tables.
- `EXPLAIN` JSON includes `Schema` only with `VERBOSE`. Under `Gather`, time per
  loop is wall time; rows removed are per loop.
- **pg_stat_statements strips a leading comment:** tag internal SQL after the
  first keyword (`SELECT /* fabio */ …`, `BEGIN /* fabio */`).
- ORDER BY a bare name binds to the `::text` alias: qualify with the table.
- `keyring` 4.x changed its API; stay on 3.x. npm has a 7-day quarantine
  (`app/.npmrc`); cargo doesn't.
- UI automation: capture only Fabio's window (largest on-screen window of the
  process, via `CGWindowListCopyWindowInfo`), click/type only when Fabio is
  frontmost, and stop if the user is using the app.

## Working conventions
- TDD against real databases (Chinook in both engines); core first, then
  command, then UI; then look at it on screen.
- Commit per coherent step, with the `Co-Authored-By` line; `dev/check.sh` green first.
- Update `PLAN.md` and this file when something moves.
