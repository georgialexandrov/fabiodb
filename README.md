<p align="center"><img src="design/icon-1024.png" width="128" alt="Fabio the marmot"></p>

<h1 align="center">Fabio</h1>

<p align="center">A fast, calm desktop client for Postgres and SQLite.<br>
Fabio keeps watch over your databases and whistles once when something's wrong.</p>

---

Fabio is a desktop app for browsing, editing and querying Postgres and SQLite
databases. It does fewer things than most database clients, and puts three
priorities first:

1. **Quiet.** Typography does the work instead of boxes and icons. One accent
   colour, and it only marks the thing that matters. Keyboard first, with every
   shortcut shown next to its action. No badges, tips or popups. A warning
   appears once, says what happened and what to do, and isn't repeated.
2. **Readable performance.** Explain turns a query plan into plain sentences,
   and Insights shows what the server is doing right now.
3. **Agents you can watch.** Claude Code, or any MCP client, can query the
   connections you open to it. The limits are enforced at the connection,
   not in a prompt, and every statement it runs shows up in Fabio as it runs.

Postgres and SQLite only. No accounts, no cloud, no telemetry.

> **Status: v0.1, macOS first, unsigned.** Fabio isn't packaged yet: you build
> it from source (below). The core and tests also run on Linux in CI; the app
> on Linux and Windows is untested.

## What it does

**Browse.** Open any table and scroll: 5 million rows scroll like 50, because
the grid is virtualized and paged, and sorting and filtering run on the server.
Row counts on huge tables are shown as estimates (`~5.0M rows`) instead of
blocking on `count(*)`. The Structure tab shows columns, indexes and foreign
keys, and you can jump along a foreign key to the table it points to.

**Edit safely.** Edit cells inline, in a row form (⌘I), add rows and delete
them. Nothing is sent until ⌘S shows you the exact statements. The save runs in
one transaction. Each row is matched on its full primary key, and each update is
guarded by the values you saw. If someone else changed or deleted the row in the
meantime, the whole save rolls back and Fabio tells you which row.

**Query.** Each query tab has its own session. The editor is CodeMirror, with
autocomplete from your live schema. You can run one statement (⌘↵) or all
(⇧⌘↵), cancel a running query, and keep history and saved snippets. Tabs are
read-only until you switch one to write mode.

**Find out why it's slow.** ⌘E runs `EXPLAIN ANALYZE` inside a transaction
that is always rolled back, then shows the plan as a tree with findings:

> Seq Scan on perf.big reads 5,000,000 rows to keep 5,000. An index on
> (bucket) could help.

On Postgres, **Insights** shows active sessions, the most expensive statements
from `pg_stat_statements`, and index and scan statistics.

**Export and copy.** Tables stream every matching row to CSV, TSV, JSON,
Markdown or `INSERT` statements (5M rows in 1.3 s using 8 MB of memory). Query
results save what's on screen and never run the query again. Select a range of
cells and copy it as TSV, CSV, Markdown, JSON or `INSERT`.

**Stay oriented.** ⌘K finds tables, connections, snippets and actions. ⇧⌘K
switches connection and ⌘D switches database on the same server, each with its
own tabs. Fabio reconnects after sleep, network changes and server restarts,
and reopens your tabs, query text and window where you left them.

## Numbers

Measured on a MacBook with a release build (`bench/startup.py`; details in
[BENCHMARKS.md](BENCHMARKS.md)). These are budgets, and CI fails if a change
goes over them.

| | |
|---|---|
| Cold start to a painted window | ~270 ms |
| Opening a table (first page) | 1–5 ms on localhost |
| Scrolling 100k rows | 60 fps, no dropped frames |
| Exporting 5M rows to CSV | 1.3 s, 8 MB of memory |
| App size | 8.3 MB |

## Connections

- **Postgres:** fill in the fields, or paste a `postgres://` URL. SSL modes
  from `disable` to `verify-full`, with your own CA file. SSH tunnels go
  through the system `ssh`, so your keys, agent and `~/.ssh/config` just work.
  Passwords are stored in the macOS Keychain, never in a file.
- **SQLite:** ⌘O, or drop the file on the window. It opens read-only and is
  reopened for writing only when you save edits.
- **A project folder:** "Docker folder…" reads `compose.yaml` (with `.env` and
  `env_file`) and offers the Postgres services in it, plus any SQLite files it
  finds.

## Agents (MCP)

Fabio ships a small stdio MCP server, `fabiodb-mcp`, that uses the same core
as the app. Build it and register it with your agent:

```sh
cargo build -p fabiodb-mcp --release
claude mcp add fabio -- "$PWD/target/release/fabiodb-mcp"
```

Then tick **Agents can query** on the connections you want to share. The agent
can list tables, describe them, sample rows, run queries, explain them (with
Fabio's findings), read Insights, find databases in a project folder, and add a
connection you gave it. The connection is tested before it's saved.

What the agent **can't** do:

- **Write.** Each Postgres statement runs alone inside `BEGIN READ ONLY … ROLLBACK`,
  and only reading statements (`SELECT`, `WITH`, `VALUES`, `TABLE`, `SHOW`,
  `EXPLAIN`) are accepted. SQLite connections open read-only with `query_only`
  on and `ATTACH` refused.
- **Run long or fetch everything.** 10 seconds per statement, 500 rows.
- **See connections you didn't open to it.** The allowlist is read on every call.
- **Act unseen.** Every agent statement, and every connection it adds, is
  written to the audit log and shown in Fabio's Agent panel.

A read-only transaction is not a full sandbox. Some functions with side
effects, such as `pg_terminate_backend`, are stopped only by the role's
privileges. For a database that matters, give the agent's connection a role
without superuser.

## Keys

| | |
|---|---|
| ⌘K | Command palette: tables, connections, snippets, actions |
| ⇧⌘K / ⌘D | Switch connection / database |
| ⌘T / ⌘W | New query / close tab |
| ⌘↵ / ⇧⌘↵ | Run statement / run all |
| ⌘E | Explain (runs it, rolled back); ⇧⌘E estimates only |
| ⌘S | Review and save edits (tables), save a snippet (queries) |
| ⌘I | Row details |
| ⌘F | Filter the selected column |
| ⇧⌘L | Theme: system, light, dark |

## Build from source

Requires macOS, Rust (stable) and pnpm.

```sh
brew install rustup pnpm
export PATH="$(brew --prefix rustup)/bin:$PATH" && rustup default stable
cd app && pnpm install && pnpm tauri build --bundles app
open ../target/release/bundle/macos/Fabio.app
```

For a disk image to install from or share, build `--bundles dmg` instead:
it's written to `target/release/bundle/dmg/Fabio_<version>_<arch>.dmg`
(about 6 MB). Open it and drag Fabio to Applications.

The app is not signed or notarized yet. The first time you open it, macOS
blocks it: go to System Settings → Privacy & Security and choose Open Anyway.

## How it's built

[Tauri 2](https://tauri.app) with a Rust core and a React 19 + TypeScript UI.

```
crates/fabiodb-core/   all database logic: Postgres (tokio-postgres) and SQLite (rusqlite),
                       paging, edits, export, plans and findings, Insights, audit log,
                       agent guardrails, SSH tunnels, folder discovery
crates/fabiodb-mcp/    the stdio MCP server
app/src-tauri/         thin Tauri commands, Keychain, sessions, native menu
app/src/               the React UI (CodeMirror 6, @tanstack/react-virtual)
```

The app and the MCP server share `fabiodb-core`, so the agent follows exactly
the rules the app does.

## Developing

Start with [STATUS.md](STATUS.md) (what's done, what's left, lessons learned),
then [PLAN.md](PLAN.md) (phases and budgets) and [VOICE.md](VOICE.md) (how any
text in the app is written).

```sh
dev/pg.sh start            # local Postgres 18 on :54329 with the Chinook sample
dev/check.sh               # what CI runs: fmt, tsc, Vitest, clippy, core + MCP tests
dev/check.sh --budgets     # also builds the release app and checks the budgets
cd app && pnpm tauri dev   # run with hot reload
```

Core logic is test-first against real databases, with Chinook loaded in both
engines. There are no mocks.

## Licence

Either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
The [Chinook](https://github.com/lerocha/chinook-database) sample database in
`dev/data/` is under its own MIT licence.
