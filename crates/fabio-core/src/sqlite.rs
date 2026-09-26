use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, InterruptHandle, OpenFlags, params_from_iter, types::ValueRef};

use crate::sql::{self, Dialect, quote};
use crate::{
    Canceller, Column, CompletionTable, Count, Error, Filter, ForeignKey, Index, Page, PageRequest, QueryResult, Relation, RelationKind,
    RelationRef, Result, ResultColumn, Rows, TableInfo,
};

/// SQLite calls are blocking, so every call hops to the blocking pool.
pub struct Lite {
    path: PathBuf,
    conn: Arc<Mutex<Connection>>,
    interrupt: Mutex<Arc<InterruptHandle>>,
}

fn open_file(path: PathBuf, writable: bool) -> rusqlite::Result<Connection> {
    let mode = if writable { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY };
    Connection::open_with_flags(path, mode | OpenFlags::SQLITE_OPEN_NO_MUTEX)
}

impl Lite {
    pub async fn open(path: PathBuf) -> Result<Lite> {
        let conn = tokio::task::spawn_blocking({
            let path = path.clone();
            move || open_file(path, false)
        })
        .await??;
        let interrupt = Mutex::new(Arc::new(conn.get_interrupt_handle()));
        Ok(Lite { path, conn: Arc::new(Mutex::new(conn)), interrupt })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller::Sqlite(self.interrupt.lock().expect("interrupt handle poisoned").clone())
    }

    /// Read-only is an open flag in SQLite, so switching reopens the file.
    pub async fn set_writable(&self, writable: bool) -> Result<()> {
        let path = self.path.clone();
        let conn = tokio::task::spawn_blocking(move || open_file(path, writable)).await??;
        *self.interrupt.lock().expect("interrupt handle poisoned") = Arc::new(conn.get_interrupt_handle());
        *self.conn.lock().expect("sqlite connection poisoned") = conn;
        Ok(())
    }

    async fn with<T: Send + 'static>(&self, f: impl FnOnce(&Connection) -> Result<T> + Send + 'static) -> Result<T> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || f(&conn.lock().expect("sqlite connection poisoned"))).await?
    }

    pub async fn query(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let sql = sql.to_owned();
        self.with(move |conn| {
            let started = Instant::now();
            let mut statement = conn.prepare(&sql)?;
            let columns: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();
            let (rows, truncated) = collect_rows(statement.raw_query(), columns.len(), max_rows)?;
            Ok(QueryResult { columns, rows, truncated, elapsed_ms: ms(started) })
        })
        .await
    }

    /// Agent mode: no ATTACH (it could reach other files on disk). The file is
    /// already open read-only; `query_only` is set again before each statement.
    pub async fn guard(&self) -> Result<()> {
        self.with(|conn| {
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_ATTACHED, 0)?;
            Ok(())
        })
        .await
    }

    /// One statement, read-only, interrupted after `timeout`. For agents.
    pub async fn query_guarded(&self, sql: &str, max_rows: usize, timeout: Duration) -> Result<QueryResult> {
        let sql = sql.to_owned();
        self.with_timeout(timeout, move |conn| {
            let started = Instant::now();
            conn.execute_batch("PRAGMA query_only = ON")?;
            let mut statement = conn.prepare(&sql).map_err(one_statement)?;
            let columns: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();
            let (rows, truncated) = collect_rows(statement.raw_query(), columns.len(), max_rows)?;
            Ok(QueryResult { columns, rows, truncated, elapsed_ms: ms(started) })
        })
        .await
    }

    async fn with_timeout<T: Send + 'static>(
        &self,
        timeout: Duration,
        f: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let interrupt = self.interrupt.lock().expect("interrupt handle poisoned").clone();
        let running = self.with(f);
        tokio::pin!(running);
        match tokio::time::timeout(timeout, &mut running).await {
            Ok(result) => result,
            Err(_) => {
                interrupt.interrupt();
                running.await
            }
        }
    }

    pub async fn relations(&self) -> Result<Vec<Relation>> {
        self.with(|conn| {
            let schemas: Vec<String> = conn
                .prepare("SELECT name FROM pragma_database_list WHERE name <> 'temp' ORDER BY seq")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut relations = Vec::new();
            for schema in schemas {
                let sql = format!(
                    "SELECT name, type FROM {}.sqlite_schema
                      WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
                      ORDER BY name",
                    quote(&schema)
                );
                let mut statement = conn.prepare(&sql)?;
                let rows = statement.query_map([], |r| {
                    Ok(Relation {
                        schema: schema.clone(),
                        name: r.get(0)?,
                        kind: if r.get::<_, String>(1)? == "view" { RelationKind::View } else { RelationKind::Table },
                        estimated_rows: None,
                    })
                })?;
                relations.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
            }
            Ok(relations)
        })
        .await
    }

    pub async fn explain(&self, sql: &str) -> Result<crate::Plan> {
        let sql = format!("EXPLAIN QUERY PLAN {sql}");
        self.with(move |conn| {
            let rows = conn
                .prepare(&sql)?
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(3)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(crate::plan::from_sqlite_rows(rows))
        })
        .await
    }

    pub async fn completion_schema(&self) -> Result<Vec<CompletionTable>> {
        let relations = self.relations().await?;
        self.with(move |conn| {
            let mut tables = Vec::new();
            for r in relations {
                let columns = conn
                    .prepare("SELECT name FROM pragma_table_info(?1, ?2) ORDER BY cid")?
                    .query_map([&r.name, &r.schema], |row| row.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                tables.push(CompletionTable { schema: r.schema, name: r.name, columns });
            }
            Ok(tables)
        })
        .await
    }

    pub async fn describe(&self, relation: &RelationRef) -> Result<TableInfo> {
        let relation = relation.clone();
        self.with(move |conn| describe(conn, &relation)).await
    }

    pub async fn count(&self, relation: &RelationRef, filters: &[Filter], timeout: Duration) -> Result<Count> {
        let interrupt = self.conn.lock().expect("sqlite connection poisoned").get_interrupt_handle();
        let (relation, filters) = (relation.clone(), filters.to_vec());
        let counting = self.with(move |conn| {
            let info = describe(conn, &relation)?;
            let from = format!("{}.{}", quote(&relation.schema), quote(&relation.name));
            let (sql, params) = sql::count_statement(&DIALECT, &from, &info.columns, &filters)?;
            Ok(conn.query_row(&sql, params_from_iter(params), |r| r.get::<_, i64>(0))?)
        });
        tokio::pin!(counting);
        match tokio::time::timeout(timeout, &mut counting).await {
            Ok(rows) => Ok(Count { rows: Some(rows? as u64), exact: true }),
            Err(_) => {
                interrupt.interrupt();
                // Let the interrupted statement finish unwinding before the
                // connection is used again; SQLite keeps no row estimate.
                let _ = counting.await;
                Ok(Count { rows: None, exact: false })
            }
        }
    }

    pub async fn page(&self, request: &PageRequest) -> Result<Page> {
        let request = request.clone();
        self.with(move |conn| {
            let started = Instant::now();
            let info = describe(conn, &request.relation)?;
            let from = format!("{}.{}", quote(&request.relation.schema), quote(&request.relation.name));
            let (sql, params) = sql::page_statement(&DIALECT, &from, &info.columns, &request)?;

            let mut statement = conn.prepare(&sql)?;
            let (mut rows, _) = collect_rows(statement.query(params_from_iter(params))?, info.columns.len(), usize::MAX)?;
            let has_more = rows.len() > request.limit as usize;
            rows.truncate(request.limit as usize);

            Ok(Page {
                columns: info
                    .columns
                    .iter()
                    .map(|c| ResultColumn { name: c.name.clone(), data_type: c.data_type.clone() })
                    .collect(),
                rows,
                has_more,
                elapsed_ms: ms(started),
                sql,
            })
        })
        .await
    }
}

fn describe(conn: &Connection, relation: &RelationRef) -> Result<TableInfo> {
    let args = [&relation.name, &relation.schema];

    let columns: Vec<Column> = conn
        .prepare(r#"SELECT name, type, "notnull", dflt_value, pk FROM pragma_table_xinfo(?1, ?2) ORDER BY cid"#)?
        .query_map(args, |r| {
            let data_type: String = r.get(1)?;
            Ok(Column {
                name: r.get(0)?,
                base_type: data_type.clone(),
                data_type,
                nullable: !r.get::<_, bool>(2)?,
                default: r.get(3)?,
                primary_key: r.get::<_, i64>(4)? > 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if columns.is_empty() {
        return Err(Error::NotFound(format!("{}.{}", relation.schema, relation.name)));
    }

    let index_list: Vec<(String, bool, String)> = conn
        .prepare(r#"SELECT name, "unique", origin FROM pragma_index_list(?1, ?2) ORDER BY origin = 'pk' DESC, name"#)?
        .query_map(args, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut indexes = Vec::new();
    for (name, unique, origin) in index_list {
        let columns = conn
            .prepare("SELECT coalesce(name, '<expression>') FROM pragma_index_info(?1, ?2) ORDER BY seqno")?
            .query_map([&name, &relation.schema], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        indexes.push(Index { name, columns, unique, primary: origin == "pk" });
    }

    let mut foreign_keys: Vec<ForeignKey> = Vec::new();
    let mut last_id = None;
    let mut statement = conn
        .prepare(r#"SELECT id, "table", "from", "to" FROM pragma_foreign_key_list(?1, ?2) ORDER BY id, seq"#)?;
    let mut rows = statement.query(args)?;
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        if last_id != Some(id) {
            foreign_keys.push(ForeignKey {
                name: None,
                columns: vec![],
                ref_schema: relation.schema.clone(),
                ref_table: r.get(1)?,
                ref_columns: vec![],
            });
            last_id = Some(id);
        }
        let fk = foreign_keys.last_mut().expect("pushed above");
        fk.columns.push(r.get(2)?);
        // `to` is NULL when the key references the parent's primary key implicitly.
        fk.ref_columns.push(r.get::<_, Option<String>>(3)?.unwrap_or_default());
    }

    Ok(TableInfo { columns, indexes, foreign_keys })
}

const DIALECT: Dialect = Dialect {
    param: |n| format!("?{n}"),
    compare: |col, op, p| format!("{} {op} {p}", quote(&col.name)),
    contains: |col, p| format!("instr(lower({}), lower({p})) > 0", quote(&col.name)),
    select: |col| quote(&col.name),
};

fn one_statement(e: rusqlite::Error) -> Error {
    match e {
        rusqlite::Error::MultipleStatement => Error::Invalid(crate::postgres::ONE_STATEMENT.into()),
        other => other.into(),
    }
}

/// Up to `max` rows, and whether more were left.
fn collect_rows(mut cursor: rusqlite::Rows<'_>, width: usize, max: usize) -> Result<(Rows, bool)> {
    let mut rows = Vec::new();
    while let Some(row) = cursor.next()? {
        if rows.len() == max {
            return Ok((rows, true));
        }
        rows.push((0..width).map(|i| row.get_ref(i).map(text)).collect::<rusqlite::Result<_>>()?);
    }
    Ok((rows, false))
}

fn text(value: ValueRef) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        // Same shape as Postgres' bytea hex output.
        ValueRef::Blob(b) => Some(format!("\\x{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>())),
    }
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
