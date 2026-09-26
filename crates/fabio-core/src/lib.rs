//! Database access shared by the desktop app and (later) the MCP server.
//!
//! `Db` is the narrow surface both engines share: run a statement, list
//! relations, describe one, page its rows. Values come back as text in the
//! engine's own format; NULL stays `None`.

mod postgres;
mod sql;
mod sqlite;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use postgres::{PgTarget, SslMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        match self {
            Db::Postgres(pg) => pg.query(sql).await,
            Db::Sqlite(lite) => lite.query(sql).await,
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

    pub async fn page(&self, request: &PageRequest) -> Result<Page> {
        match self {
            Db::Postgres(pg) => pg.page(request).await,
            Db::Sqlite(lite) => lite.page(request).await,
        }
    }
}

pub type Rows = Vec<Vec<Option<String>>>;

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Rows,
    pub elapsed_ms: f64,
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

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Postgres(String),
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
            Some(db) => Error::Postgres(db.message().to_owned()),
            None => Error::Postgres(err.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
