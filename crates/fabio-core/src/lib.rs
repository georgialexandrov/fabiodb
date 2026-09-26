//! Database access shared by the desktop app and (later) the MCP server.
//!
//! Phase 0: run one statement and return every value as text. Typed values,
//! paging and the `Engine` trait arrive in Phase 1.

use std::path::PathBuf;
use std::time::Instant;

use rusqlite::{OpenFlags, types::ValueRef};
use serde::{Deserialize, Serialize};
use tokio_postgres::{NoTls, SimpleQueryMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "engine", rename_all = "lowercase")]
pub enum Target {
    Postgres { url: String },
    Sqlite { path: PathBuf },
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// `None` is SQL NULL, kept distinct from the empty string.
    pub rows: Vec<Vec<Option<String>>>,
    pub elapsed_ms: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Postgres(String),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Task(#[from] tokio::task::JoinError),
}

impl From<tokio_postgres::Error> for Error {
    fn from(err: tokio_postgres::Error) -> Self {
        // The Display impl of a server error is just "db error"; the message is inside.
        match err.as_db_error() {
            Some(db) => Error::Postgres(db.message().to_owned()),
            None => Error::Postgres(err.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub async fn run(target: &Target, sql: &str) -> Result<QueryResult> {
    let started = Instant::now();
    let (columns, rows) = match target {
        Target::Postgres { url } => run_postgres(url, sql).await?,
        Target::Sqlite { path } => {
            let (path, sql) = (path.clone(), sql.to_owned());
            tokio::task::spawn_blocking(move || run_sqlite(&path, &sql)).await??
        }
    };
    Ok(QueryResult { columns, rows, elapsed_ms: started.elapsed().as_secs_f64() * 1000.0 })
}

type Table = (Vec<String>, Vec<Vec<Option<String>>>);

async fn run_postgres(url: &str, sql: &str) -> Result<Table> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(connection);

    // Simple-query protocol returns every value in Postgres' own text format.
    // With several statements, the last result set wins.
    let (mut columns, mut rows) = (Vec::new(), Vec::new());
    for message in client.simple_query(sql).await? {
        match message {
            SimpleQueryMessage::RowDescription(description) => {
                columns = description.iter().map(|c| c.name().to_owned()).collect();
                rows.clear();
            }
            SimpleQueryMessage::Row(row) => {
                rows.push((0..row.len()).map(|i| row.get(i).map(str::to_owned)).collect());
            }
            _ => {}
        }
    }
    Ok((columns, rows))
}

fn run_sqlite(path: &std::path::Path, sql: &str) -> Result<Table> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut statement = conn.prepare(sql)?;
    let columns: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();

    let mut rows = Vec::new();
    let mut cursor = statement.raw_query();
    while let Some(row) = cursor.next()? {
        rows.push(
            (0..columns.len())
                .map(|i| row.get_ref(i).map(sqlite_text))
                .collect::<rusqlite::Result<_>>()?,
        );
    }
    Ok((columns, rows))
}

fn sqlite_text(value: ValueRef) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        // Same shape as Postgres' bytea hex output.
        ValueRef::Blob(b) => Some(format!("\\x{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())),
    }
}
