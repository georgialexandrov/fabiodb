//! Talks to the real binary over stdio, against Chinook in both engines.
//! Postgres needs `dev/pg.sh start`.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use fabiodb_core::{AuditLog, PgTarget, SavedConnection, Source, Store, Target};
use serde_json::{Value, json};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    dir: PathBuf,
    next_id: u64,
}

impl Server {
    /// A server over a fresh Fabio folder with Chinook in Postgres (open to
    /// agents), Chinook in SQLite (open) and a private copy (not open).
    fn start() -> Server {
        // Tests run in parallel and clean up after themselves: one folder each.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fabiodb-mcp-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::new(dir.join("connections.json"));
        let sqlite: PathBuf = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/data/Chinook_Sqlite.sqlite").into();
        let url =
            std::env::var("FABIO_TEST_PG_URL").unwrap_or_else(|_| "postgres://fabio@localhost:54329/chinook".into());
        for (id, agent, target) in [
            ("pg", true, Target::Postgres(PgTarget::from_url(&url).unwrap())),
            ("lite", true, Target::Sqlite { path: sqlite.clone() }),
            ("private", false, Target::Sqlite { path: sqlite }),
        ] {
            store
                .save(SavedConnection { id: id.into(), name: format!("{id} chinook"), target, agent, group: None })
                .unwrap();
        }

        let mut child = Command::new(env!("CARGO_BIN_EXE_fabiodb-mcp"))
            .env("FABIO_DIR", &dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut server = Server { child, stdin, stdout, dir, next_id: 1 };
        let init = server.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "fabio");
        server.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        server
    }

    fn send(&mut self, message: Value) {
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert_eq!(response["id"], id);
        response
    }

    /// The tool's text, and whether it's an error.
    fn call(&mut self, tool: &str, arguments: Value) -> (String, bool) {
        let response = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &response["result"];
        (result["content"][0]["text"].as_str().unwrap().to_owned(), result["isError"] == true)
    }

    fn audit(&self) -> AuditLog {
        AuditLog::open(self.dir.join("audit.sqlite")).unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn lists_its_tools() {
    let mut server = Server::start();
    let tools = server.request("tools/list", json!({}));
    let names: Vec<_> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "list_connections",
            "list_tables",
            "describe_table",
            "sample_rows",
            "query",
            "explain",
            "insights",
            "find_databases",
            "create_connection"
        ]
    );
}

#[test]
fn answers_ping_and_rejects_unknown_methods() {
    let mut server = Server::start();
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));
    assert_eq!(server.request("nope", json!({}))["error"]["code"], -32601);
}

#[test]
fn lists_only_connections_open_to_agents() {
    let (text, error) = Server::start().call("list_connections", json!({}));
    assert!(!error);
    assert!(text.contains("pg chinook") && text.contains("lite chinook"), "{text}");
    assert!(!text.contains("private"), "{text}");
}

#[test]
fn browses_tables_in_both_engines() {
    let mut server = Server::start();
    let (text, _) = server.call("list_tables", json!({"connection": "pg chinook"}));
    assert!(text.contains("public.track"), "{text}");

    let (text, _) = server.call("describe_table", json!({"connection": "lite", "table": "Track"}));
    assert!(text.contains("TrackId") && text.contains("AlbumId"), "{text}");

    let (text, _) = server.call("describe_table", json!({"connection": "pg", "table": "public.track"}));
    assert!(text.contains("track_id"), "{text}");

    let (text, _) = server.call("sample_rows", json!({"connection": "pg", "table": "artist", "rows": 3}));
    assert!(text.contains("AC/DC"), "{text}");
}

#[test]
fn queries_are_read_only_and_audited_as_agent() {
    let mut server = Server::start();
    let (text, error) =
        server.call("query", json!({"connection": "pg", "sql": "select name from artist where artist_id = 1"}));
    assert!(!error, "{text}");
    assert!(text.contains("AC/DC"), "{text}");

    let (text, error) = server.call("query", json!({"connection": "pg", "sql": "delete from artist"}));
    assert!(error);
    assert!(text.contains("only read"), "{text}");

    let (text, error) = server.call("query", json!({"connection": "private", "sql": "select 1"}));
    assert!(error);
    assert!(text.contains("not open to agents"), "{text}");

    let log = server.audit().recent(Some("pg"), 10).unwrap();
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|e| e.source == Source::Agent));
}

#[test]
fn explain_returns_findings_and_the_plan() {
    let mut server = Server::start();
    let (text, error) = server.call(
        "explain",
        json!({"connection": "pg", "sql": "select * from perf.big where label = 'x'", "analyze": false}),
    );
    assert!(!error, "{text}");
    assert!(text.contains("Seq Scan") && text.contains("perf.big"), "{text}");
}

#[test]
fn insights_on_postgres() {
    let (text, error) = Server::start().call("insights", json!({"connection": "pg"}));
    assert!(!error, "{text}");
    assert!(text.contains("top_statements"), "{text}");
}

#[test]
fn creates_a_connection_and_finds_databases_in_a_folder() {
    let mut server = Server::start();
    let (text, error) =
        server.call("create_connection", json!({"name": "again", "url": "postgres://fabio@localhost:54329/chinook"}));
    assert!(!error, "{text}");
    let (text, _) = server.call("list_connections", json!({}));
    assert!(text.contains("again"), "{text}");
    let (text, error) = server.call("query", json!({"connection": "again", "sql": "select 1"}));
    assert!(!error, "{text}");

    let folder = server.dir.join("project");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("compose.yaml"), "services:\n  db:\n    image: postgres:17\n    ports: ['5999:5432']\n")
        .unwrap();
    let (text, error) = server.call("find_databases", json!({"folder": folder}));
    assert!(!error, "{text}");
    assert!(text.contains("postgres://postgres@localhost:5999/postgres"), "{text}");
}
