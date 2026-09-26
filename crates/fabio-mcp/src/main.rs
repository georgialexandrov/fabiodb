//! Fabio's MCP server: newline-delimited JSON-RPC over stdio.
//!
//! The agent sees only connections marked "Agents can query" in the app, can
//! only read, and every statement lands in the app's audit log, where the
//! agent panel shows it. All of that is enforced in `fabio_core::Agent`; this
//! file only speaks the protocol.
//!
//! Reads the app's folder (`FABIO_DIR` overrides it) and the app's keychain
//! entries, so it works whether or not the app is running.

mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use fabio_core::{Agent, AuditLog, Limits, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

/// Same service and account names as the app.
const KEYCHAIN_SERVICE: &str = "dev.fabio.app";
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Fabio gives read-only access to the databases its user marked for agents \
(Postgres and SQLite). Start with list_connections, then list_tables and describe_table. \
Each call runs one statement; results are capped at 500 rows and statements stop after 10 s. \
To find out why a query is slow, use explain: it returns Fabio's findings and the plan. \
The user sees every statement you run in Fabio's agent panel.";

#[tokio::main]
async fn main() {
    let (config, data) = dirs();
    for dir in [&config, &data] {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("fabio-mcp: {}: {e}", dir.display());
            std::process::exit(1);
        }
    }
    let audit = match AuditLog::open(data.join("audit.sqlite")) {
        Ok(log) => Arc::new(log),
        Err(e) => {
            eprintln!("fabio-mcp: audit log: {e}");
            std::process::exit(1);
        }
    };
    let agent = Arc::new(Agent::new(
        Store::new(config.join("connections.json")),
        audit,
        Limits::default(),
        Box::new(keychain_password),
    ));

    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    // Requests run concurrently, so a slow query doesn't hold up a quick one.
    let mut running = JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let (agent, stdout) = (agent.clone(), stdout.clone());
        running.spawn(async move {
            if let Some(response) = handle(&agent, &line).await {
                let mut out = stdout.lock().await;
                let _ = out.write_all(format!("{response}\n").as_bytes()).await;
                let _ = out.flush().await;
            }
        });
    }
    while running.join_next().await.is_some() {}
}

/// The response to one message; `None` for notifications.
async fn handle(agent: &Agent, line: &str) -> Option<Value> {
    let message: Value = match serde_json::from_str(line) {
        Ok(m) => m,
        Err(e) => return Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fabio", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools::list()}),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default();
            let (text, is_error) = match tools::call(agent, name, &params["arguments"]).await {
                Ok(text) => (text, false),
                Err(message) => (message, true),
            };
            json!({"content": [{"type": "text", "text": text}], "isError": is_error})
        }
        method => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// No keychain (a Linux box without a secret service) is the same as no saved
/// password: connections that need none still work, and the others fail with
/// the server's own authentication error.
fn keychain_password(id: &str) -> fabio_core::Result<Option<String>> {
    let found = keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|entry| entry.get_password());
    match found {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            eprintln!("fabio-mcp: keychain unavailable, connecting without a password: {e}");
            Ok(None)
        }
    }
}

/// Where the app keeps connections.json (config) and audit.sqlite (data):
/// Tauri's app_config_dir and app_data_dir for the `dev.fabio.app` identifier.
fn dirs() -> (PathBuf, PathBuf) {
    if let Some(dir) = std::env::var_os("FABIO_DIR") {
        return (dir.clone().into(), dir.into());
    }
    let home = PathBuf::from(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default());
    let app = |base: PathBuf| base.join("dev.fabio.app");
    if cfg!(target_os = "macos") {
        let dir = app(home.join("Library/Application Support"));
        (dir.clone(), dir)
    } else if cfg!(windows) {
        let dir = app(std::env::var_os("APPDATA").map_or_else(|| home.join("AppData/Roaming"), PathBuf::from));
        (dir.clone(), dir)
    } else {
        let xdg = |var: &str, fallback: &str| std::env::var_os(var).map_or_else(|| home.join(fallback), PathBuf::from);
        (app(xdg("XDG_CONFIG_HOME", ".config")), app(xdg("XDG_DATA_HOME", ".local/share")))
    }
}
