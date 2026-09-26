use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, params_from_iter, types::ValueRef};

use crate::sql::{self, Dialect, quote};
use crate::{
    Column, Count, Error, Filter, ForeignKey, Index, Page, PageRequest, QueryResult, Relation, RelationKind,
    RelationRef, Result, ResultColumn, Rows, TableInfo,
};

/// SQLite calls are blocking, so every call hops to the blocking pool.
pub struct Lite {
    conn: Arc<Mutex<Connection>>,
}

impl Lite {
    pub async fn open(path: PathBuf) -> Result<Lite> {
        let conn = tokio::task::spawn_blocking(move || {
            Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        })
        .await??;
        Ok(Lite { conn: Arc::new(Mutex::new(conn)) })
    }

    async fn with<T: Send + 'static>(&self, f: impl FnOnce(&Connection) -> Result<T> + Send + 'static) -> Result<T> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || f(&conn.lock().expect("sqlite connection poisoned"))).await?
    }

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        let sql = sql.to_owned();
        self.with(move |conn| {
            let started = Instant::now();
            let mut statement = conn.prepare(&sql)?;
            let columns: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();
            let rows = collect_rows(statement.raw_query(), columns.len())?;
            Ok(QueryResult { columns, rows, elapsed_ms: ms(started) })
        })
        .await
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
            let mut rows = collect_rows(statement.query(params_from_iter(params))?, info.columns.len())?;
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

fn collect_rows(mut cursor: rusqlite::Rows<'_>, width: usize) -> Result<Rows> {
    let mut rows = Vec::new();
    while let Some(row) = cursor.next()? {
        rows.push((0..width).map(|i| row.get_ref(i).map(text)).collect::<rusqlite::Result<_>>()?);
    }
    Ok(rows)
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
