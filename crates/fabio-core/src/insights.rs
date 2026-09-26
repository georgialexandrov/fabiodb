//! Postgres' own statistics views, read-only.

use serde::Serialize;
use tokio_postgres::Client;

use crate::Result;

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub pid: i32,
    pub user: Option<String>,
    pub application: Option<String>,
    pub state: Option<String>,
    /// `Lock:transactionid`, `IO:DataFileRead`, …
    pub waiting_on: Option<String>,
    pub running_ms: Option<f64>,
    pub query: String,
    /// Sessions holding locks this one waits for.
    pub blocked_by: Vec<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopStatement {
    pub query: String,
    pub calls: i64,
    pub total_ms: f64,
    pub mean_ms: f64,
    pub rows: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnusedIndex {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub scans: i64,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeqScanTable {
    pub schema: String,
    pub table: String,
    pub seq_scans: i64,
    pub seq_rows_read: i64,
    pub index_scans: Option<i64>,
    pub live_rows: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Insights {
    pub activity: Vec<Activity>,
    /// `None` when pg_stat_statements isn't installed.
    pub top_statements: Option<Vec<TopStatement>>,
    pub unused_indexes: Vec<UnusedIndex>,
    pub seq_scan_tables: Vec<SeqScanTable>,
}

pub async fn postgres(client: &Client) -> Result<Insights> {
    let activity = client
        .query(
            "SELECT pid, usename::text, application_name, state,
                    wait_event_type || ':' || wait_event,
                    (extract(epoch FROM now() - query_start) * 1000)::float8,
                    query, pg_blocking_pids(pid)
               FROM pg_stat_activity
              WHERE datname = current_database() AND pid <> pg_backend_pid()
                AND backend_type = 'client backend'
              ORDER BY state = 'active' DESC, query_start",
            &[],
        )
        .await?
        .iter()
        .map(|r| Activity {
            pid: r.get(0),
            user: r.get(1),
            application: r.get(2),
            state: r.get(3),
            waiting_on: r.get(4),
            running_ms: r.get(5),
            query: r.get(6),
            blocked_by: r.get(7),
        })
        .collect();

    let installed = client
        .query_opt("SELECT 1 FROM pg_extension WHERE extname = 'pg_stat_statements'", &[])
        .await?
        .is_some();
    let top_statements = if installed {
        // The view errors if the library isn't preloaded; treat that as "not available".
        client
            .query(
                "SELECT s.query, s.calls, s.total_exec_time, s.mean_exec_time, s.rows
                   FROM pg_stat_statements s JOIN pg_database d ON d.oid = s.dbid
                  WHERE d.datname = current_database()
                  ORDER BY s.total_exec_time DESC LIMIT 50",
                &[],
            )
            .await
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|r| TopStatement {
                        query: r.get(0),
                        calls: r.get(1),
                        total_ms: r.get(2),
                        mean_ms: r.get(3),
                        rows: r.get(4),
                    })
                    .collect()
            })
    } else {
        None
    };

    let unused_indexes = client
        .query(
            "SELECT s.schemaname::text, s.relname::text, s.indexrelname::text, s.idx_scan,
                    pg_relation_size(s.indexrelid)
               FROM pg_stat_user_indexes s JOIN pg_index i ON i.indexrelid = s.indexrelid
              WHERE s.idx_scan = 0 AND NOT i.indisunique AND NOT i.indisprimary
              ORDER BY pg_relation_size(s.indexrelid) DESC LIMIT 50",
            &[],
        )
        .await?
        .iter()
        .map(|r| UnusedIndex { schema: r.get(0), table: r.get(1), name: r.get(2), scans: r.get(3), size_bytes: r.get(4) })
        .collect();

    let seq_scan_tables = client
        .query(
            "SELECT schemaname::text, relname::text, seq_scan, seq_tup_read, idx_scan, n_live_tup
               FROM pg_stat_user_tables WHERE seq_tup_read > 0
              ORDER BY seq_tup_read DESC LIMIT 20",
            &[],
        )
        .await?
        .iter()
        .map(|r| SeqScanTable {
            schema: r.get(0),
            table: r.get(1),
            seq_scans: r.get(2),
            seq_rows_read: r.get(3),
            index_scans: r.get(4),
            live_rows: r.get(5),
        })
        .collect();

    Ok(Insights { activity, top_statements, unused_indexes, seq_scan_tables })
}
