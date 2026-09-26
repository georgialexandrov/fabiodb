<p align="center"><img src="design/icon-1024.png" width="128" alt="Fabio the marmot"></p>

# Fabio

A fast, calm desktop client for Postgres and SQLite. Fabio keeps watch over
your databases and whistles once when something's wrong.

- **Browse** any table: 5M rows scroll like 50, sorted and filtered on the server.
- **Edit** cells, add and delete rows. Nothing runs until you've read the
  statements, and a row someone else changed meanwhile stops the save.
- **Query** in tabs with their own sessions, autocomplete from your schema, and
  snippets. Everything is read-only until you switch a tab to write mode.
- **Find out why it's slow.** Explain turns a plan into plain sentences:
  "Seq Scan on perf.big reads 5,000,000 rows to keep 5,000. An index on
  (bucket) could help." Insights shows what the server is doing right now.
- **Let an agent in, and watch.** Claude Code (or any MCP client) can query the
  connections you open to it — read-only, enforced at the connection, not in a
  prompt — and every statement shows up in Fabio as it runs.

Postgres and SQLite only. No accounts, no cloud, no telemetry.

## Numbers

Measured on a MacBook, release build (`bench/startup.py`, see `BENCHMARKS.md`):

| | |
|---|---|
| Cold start to a painted window | ~270 ms |
| Opening a table (first page) | 1–5 ms on localhost |
| Scrolling 100k rows | 60 fps, no dropped frames |
| Exporting 5M rows to CSV | 1.3 s, 8 MB of memory |
| App size | 8.3 MB |

## Install

Fabio isn't packaged yet. To build it on macOS:

```sh
brew install rustup pnpm
export PATH="$(brew --prefix rustup)/bin:$PATH" && rustup default stable
cd app && pnpm install && pnpm tauri build --bundles app
open ../target/release/bundle/macos/Fabio.app
```

## Connections

- **Postgres:** fields, or paste a `postgres://` URL. SSL from `disable` to
  `verify-full` (with your own CA file). Through SSH with your usual keys, agent
  and `~/.ssh/config`. Passwords live in the macOS Keychain, never in a file.
- **SQLite:** ⌘O, or drop the file on the window.
- **A project folder:** "Docker folder…" reads `compose.yaml` (and `.env`) and
  offers the Postgres services in it, plus any SQLite files.
- **Many of them:** ⇧⌘K finds any connection; ⌘D switches database on the same
  server, each with its own tabs.

## Agents

Build the MCP server and register it with your agent:

```sh
cargo build -p fabiodb-mcp --release
claude mcp add fabio -- "$PWD/target/release/fabiodb-mcp"
```

Then tick **Agents can query** on the connections you want to share. The agent
can list tables, sample rows, run queries, explain them (with Fabio's findings),
read Insights, find databases in a project folder and add a connection it was
given. What it can't do:

- **write.** Each statement runs alone in a read-only transaction that's rolled
  back, and only reading statements are accepted.
- **run long or fetch everything.** 10 seconds per statement and 500 rows.
- **see connections you didn't open to it.**

For a database that matters, give the agent's connection a role without
superuser: some functions with side effects (`pg_terminate_backend`) are
stopped only by the role's privileges.

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

## Developing

Read `STATUS.md` first. Core logic lives in `crates/fabiodb-core` and is tested
against real databases (`dev/pg.sh start` runs a local Postgres with the
Chinook sample; the same data ships as SQLite). `dev/check.sh` runs what CI
runs; `dev/check.sh --budgets` also checks the performance budgets. Text in the
app follows `VOICE.md`.

## Licence

Either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
