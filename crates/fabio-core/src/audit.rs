//! Every statement Fabio runs, from a human or an agent, in a small SQLite file.
//! Query history reads from it, and the agent panel tails it.

use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Human,
    Agent,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Human => "human",
            Source::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewAuditEntry {
    pub connection_id: String,
    pub source: Source,
    pub sql: String,
    pub elapsed_ms: f64,
    pub rows: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub id: i64,
    pub at_ms: i64,
    pub connection_id: String,
    pub source: Source,
    pub sql: String,
    pub elapsed_ms: f64,
    pub rows: Option<u64>,
    pub error: Option<String>,
}

pub struct AuditLog {
    conn: Mutex<Connection>,
}

impl AuditLog {
    /// Opens or creates the log. `":memory:"` gives a throwaway log for tests.
    pub fn open(path: impl AsRef<Path>) -> Result<AuditLog> {
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        // WAL so the MCP server and the app can both write.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(())).optional()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS statements (
                 id INTEGER PRIMARY KEY,
                 at_ms INTEGER NOT NULL,
                 connection_id TEXT NOT NULL,
                 source TEXT NOT NULL,
                 sql TEXT NOT NULL,
                 elapsed_ms REAL NOT NULL,
                 rows INTEGER,
                 error TEXT
             );
             CREATE INDEX IF NOT EXISTS statements_by_connection ON statements (connection_id, id DESC);",
        )?;
        Ok(AuditLog { conn: Mutex::new(conn) })
    }

    pub fn record(&self, entry: &NewAuditEntry) -> Result<i64> {
        let at_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let conn = self.conn.lock().expect("audit log poisoned");
        conn.execute(
            "INSERT INTO statements (at_ms, connection_id, source, sql, elapsed_ms, rows, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                at_ms,
                entry.connection_id,
                entry.source.as_str(),
                entry.sql,
                entry.elapsed_ms,
                entry.rows.map(|n| n as i64),
                entry.error
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// Newest first, optionally for one connection.
    pub fn recent(&self, connection_id: Option<&str>, limit: u32) -> Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().expect("audit log poisoned");
        let mut statement = conn.prepare(
            "SELECT id, at_ms, connection_id, source, sql, elapsed_ms, rows, error FROM statements
              WHERE ?1 IS NULL OR connection_id = ?1
              ORDER BY id DESC LIMIT ?2",
        )?;
        let entries = statement
            .query_map(params![connection_id, limit], |r| {
                Ok(AuditEntry {
                    id: r.get(0)?,
                    at_ms: r.get(1)?,
                    connection_id: r.get(2)?,
                    source: if r.get::<_, String>(3)? == "agent" { Source::Agent } else { Source::Human },
                    sql: r.get(4)?,
                    elapsed_ms: r.get(5)?,
                    rows: r.get::<_, Option<i64>>(6)?.map(|n| n as u64),
                    error: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(entries)
    }
}
