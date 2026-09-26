//! Database access shared by the desktop app and the MCP server.
//!
//! `Db` is the narrow surface both engines share: run a statement, list
//! relations, describe one, page its rows. Values come back as text in the
//! engine's own format; NULL stays `None`.

// Fabio's own statements carry `/* fabio */` right after the first keyword, so
// they can be told apart from the user's in pg_stat_statements and server logs.
// (A leading comment would be stripped by pg_stat_statements.)

mod agent;
mod audit;
mod edit;
mod export;
mod insights;
mod plan;
mod postgres;
mod sql;
mod sqlite;
mod store;

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use agent::{Agent, AgentConnection, Limits, Passwords, ReadOnlyDb};
pub use audit::{AuditEntry, AuditLog, NewAuditEntry, Source};
pub use edit::{CellChange, ColumnValue, RowUpdate};
pub use export::{ExportFormat, RowWriter, format_rows};
pub use insights::{Activity, Insights, SeqScanTable, TopStatement, UnusedIndex};
pub use plan::{Detail, Finding, Plan, PlanNode, Severity};
pub use postgres::{PgTarget, SslMode};
pub use store::{SavedConnection, Store};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "engine", rename_all = "lowercase")]
pub enum Target {
    Postgres(PgTarget),
    Sqlite { path: PathBuf },
}

/// An open connection. Cheap to share behind an `Arc`.
pub enum Db {
    Postgres(postgres::Pg),
    Sqlite(sqlite::Lite),
}

impl Db {
    pub async fn open(target: &Target) -> Result<Db> {
        Ok(match target {
            Target::Postgres(t) => Db::Postgres(postgres::Pg::connect(t).await?),
            Target::Sqlite { path } => Db::Sqlite(sqlite::Lite::open(path.clone()).await?),
        })
    }

    /// Runs `sql`, keeping at most [`DEFAULT_MAX_ROWS`] rows.
    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        self.query_limited(sql, DEFAULT_MAX_ROWS).await
    }

    /// Runs `sql`, keeping at most `max_rows`; the rest is cancelled, not fetched.
    pub async fn query_limited(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        match self {
            Db::Postgres(pg) => pg.query(sql, max_rows).await,
            Db::Sqlite(lite) => lite.query(sql, max_rows).await,
        }
    }

    /// Something that can stop the statement currently running on this connection.
    pub fn canceller(&self) -> Canceller {
        match self {
            Db::Postgres(pg) => pg.canceller(),
            Db::Sqlite(lite) => lite.canceller(),
        }
    }

    /// Connections start read-only; writes need this, explicitly.
    pub async fn set_writable(&self, writable: bool) -> Result<()> {
        match self {
            Db::Postgres(pg) => pg.set_writable(writable).await,
            Db::Sqlite(lite) => lite.set_writable(writable).await,
        }
    }

    pub async fn relations(&self) -> Result<Vec<Relation>> {
        match self {
            Db::Postgres(pg) => pg.relations().await,
            Db::Sqlite(lite) => lite.relations().await,
        }
    }

    pub async fn describe(&self, relation: &RelationRef) -> Result<TableInfo> {
        match self {
            Db::Postgres(pg) => pg.describe(relation).await,
            Db::Sqlite(lite) => lite.describe(relation).await,
        }
    }

    /// The plan for one statement. With `analyze`, the statement really runs —
    /// inside a transaction that is always rolled back.
    pub async fn explain(&self, sql: &str, analyze: bool) -> Result<Plan> {
        let sql = sql.trim().trim_end_matches(';');
        match self {
            Db::Postgres(pg) => pg.explain(sql, analyze, false).await,
            Db::Sqlite(lite) => lite.explain(sql).await,
        }
    }

    /// What the server knows about itself: activity, top statements, index use.
    pub async fn insights(&self) -> Result<Insights> {
        match self {
            Db::Postgres(pg) => insights::postgres(pg.client()).await,
            Db::Sqlite(_) => Err(Error::Invalid("SQLite keeps no statistics to show here.".into())),
        }
    }

    /// Every relation with its column names, in one round trip, for autocomplete.
    pub async fn completion_schema(&self) -> Result<Vec<CompletionTable>> {
        match self {
            Db::Postgres(pg) => pg.completion_schema().await,
            Db::Sqlite(lite) => lite.completion_schema().await,
        }
    }

    /// Rows matching `filters`. Gives up after `timeout` (cancelling the query)
    /// so a huge table can't hold the connection; see [`Count`].
    pub async fn count(&self, relation: &RelationRef, filters: &[Filter], timeout: Duration) -> Result<Count> {
        match self {
            Db::Postgres(pg) => pg.count(relation, filters, timeout).await,
            Db::Sqlite(lite) => lite.count(relation, filters, timeout).await,
        }
    }

    /// Every row matching `filters`, in `sort` then primary-key order, streamed
    /// to `path`. No row cap. Returns how many rows were written.
    pub async fn export_table(
        &self,
        relation: &RelationRef,
        sort: Option<&Sort>,
        filters: &[Filter],
        format: ExportFormat,
        path: &std::path::Path,
    ) -> Result<u64> {
        match self {
            Db::Postgres(pg) => pg.export_table(relation, sort, filters, format, path).await,
            Db::Sqlite(lite) => lite.export_table(relation, sort, filters, format, path).await,
        }
    }

    /// The UPDATEs `apply_updates` would run, as the user should read them.
    pub async fn update_statements(&self, relation: &RelationRef, updates: &[RowUpdate]) -> Result<Vec<String>> {
        let info = self.describe(relation).await?;
        edit::validate(&info, relation, updates)?;
        let from = format!("{}.{}", sql::quote(&relation.schema), sql::quote(&relation.name));
        updates.iter().map(|u| edit::display(&from, &info, u)).collect()
    }

    /// Runs the edits in one transaction. Each must hit exactly its row with
    /// the values the user saw, or nothing is saved. Needs a writable connection.
    pub async fn apply_updates(&self, relation: &RelationRef, updates: &[RowUpdate]) -> Result<u64> {
        let info = self.describe(relation).await?;
        edit::validate(&info, relation, updates)?;
        match self {
            Db::Postgres(pg) => pg.apply_updates(relation, &info, updates).await,
            Db::Sqlite(lite) => lite.apply_updates(relation, info, updates).await,
        }
    }

    pub async fn page(&self, request: &PageRequest) -> Result<Page> {
        match self {
            Db::Postgres(pg) => pg.page(request).await,
            Db::Sqlite(lite) => lite.page(request).await,
        }
    }
}

pub type Rows = Vec<Vec<Option<String>>>;

/// Enough for any grid; stops `select *` on a huge table from filling memory.
pub const DEFAULT_MAX_ROWS: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Rows,
    /// More rows existed than were kept.
    pub truncated: bool,
    pub elapsed_ms: f64,
}

pub enum Canceller {
    /// Boxed: a cancel token plus TLS connector is ~240 bytes, the SQLite handle 8.
    Postgres(Box<(tokio_postgres::CancelToken, postgres_native_tls::MakeTlsConnector)>),
    Sqlite(std::sync::Arc<rusqlite::InterruptHandle>),
}

impl Canceller {
    pub async fn cancel(&self) -> Result<()> {
        match self {
            Canceller::Postgres(pg) => Ok(pg.0.cancel_query(pg.1.clone()).await?),
            Canceller::Sqlite(handle) => {
                handle.interrupt();
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Table,
    View,
    MaterializedView,
}

#[derive(Debug, Clone, Serialize)]
pub struct Relation {
    pub schema: String,
    pub name: String,
    pub kind: RelationKind,
    /// Planner estimate; `None` when the engine has none (never analyzed, SQLite).
    pub estimated_rows: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompletionTable {
    pub schema: String,
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelationRef {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub name: String,
    /// As the engine displays it, e.g. `numeric(10,2)`.
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
    /// Type without modifiers, used to cast filter values. Postgres only.
    #[serde(skip)]
    pub(crate) base_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Index {
    pub name: String,
    /// Column names, or the expression for expression indexes.
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForeignKey {
    pub name: Option<String>,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableInfo {
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    /// Case-insensitive substring match on the value's text form.
    Contains,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequest {
    pub relation: RelationRef,
    pub sort: Option<Sort>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    pub offset: u64,
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub columns: Vec<ResultColumn>,
    pub rows: Rows,
    pub has_more: bool,
    pub elapsed_ms: f64,
    /// The statement that produced this page, with placeholders.
    pub sql: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Count {
    /// `None` when counting timed out and there's no estimate to fall back on.
    pub rows: Option<u64>,
    /// `false` when `rows` is the planner's estimate.
    pub exact: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Postgres {
        message: String,
        /// 1-based character offset into the statement, when the server gives one.
        position: Option<u32>,
    },
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Tls(#[from] native_tls::Error),
    #[error("{0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("{0}")]
    Invalid(String),
    #[error("{0} not found")]
    NotFound(String),
}

impl From<tokio_postgres::Error> for Error {
    fn from(err: tokio_postgres::Error) -> Self {
        // The Display impl of a server error is just "db error"; the message is inside.
        match err.as_db_error() {
            Some(db) => Error::Postgres {
                message: db.message().to_owned(),
                position: match db.position() {
                    Some(tokio_postgres::error::ErrorPosition::Original(p)) => Some(*p),
                    _ => None,
                },
            },
            None => Error::Postgres { message: err.to_string(), position: None },
        }
    }
}

impl Error {
    /// Where in the statement the error is, if known (1-based characters).
    pub fn position(&self) -> Option<u32> {
        match self {
            Error::Postgres { position, .. } => *position,
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
