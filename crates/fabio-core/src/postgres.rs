use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use futures_util::StreamExt;
use tokio_postgres::{Client, SimpleQueryMessage, types::ToSql};

use crate::sql::{self, Dialect, quote};
use crate::{
    Canceller, Column, CompletionTable, Count, Error, Filter, ForeignKey, Index, Page, PageRequest, QueryResult, Relation, RelationKind,
    RelationRef, Result, ResultColumn, TableInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    /// Use TLS if the server offers it. Like libpq, the certificate is not verified.
    #[default]
    Prefer,
    /// Require TLS. Like libpq, the certificate is not verified.
    Require,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PgTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
    #[serde(default)]
    pub ssl: SslMode,
}

impl PgTarget {
    pub fn from_url(url: &str) -> Result<PgTarget> {
        let config = tokio_postgres::Config::from_str(url)?;
        let host = match config.get_hosts().first() {
            Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
            #[cfg(unix)]
            Some(tokio_postgres::config::Host::Unix(p)) => p.display().to_string(),
            None => "localhost".into(),
        };
        let user = config.get_user().unwrap_or_default().to_owned();
        Ok(PgTarget {
            host,
            port: config.get_ports().first().copied().unwrap_or(5432),
            database: config.get_dbname().map_or_else(|| user.clone(), str::to_owned),
            user,
            password: config.get_password().map(|p| String::from_utf8_lossy(p).into_owned()),
            ssl: match config.get_ssl_mode() {
                tokio_postgres::config::SslMode::Disable => SslMode::Disable,
                tokio_postgres::config::SslMode::Require => SslMode::Require,
                _ => SslMode::Prefer,
            },
        })
    }
}

pub struct Pg {
    client: Client,
    /// Kept for cancel requests, which open their own connection.
    tls: postgres_native_tls::MakeTlsConnector,
}

impl Pg {
    pub async fn connect(target: &PgTarget) -> Result<Pg> {
        let mut config = tokio_postgres::Config::new();
        config
            .host(&target.host)
            .port(target.port)
            .user(&target.user)
            .dbname(&target.database)
            .application_name("fabio")
            .connect_timeout(Duration::from_secs(5))
            .ssl_mode(match target.ssl {
                SslMode::Disable => tokio_postgres::config::SslMode::Disable,
                SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                SslMode::Require => tokio_postgres::config::SslMode::Require,
            });
        if let Some(password) = &target.password {
            config.password(password);
        }
        let tls = native_tls::TlsConnector::builder()
            .danger_accept_invalid_certs(true)
            .build()?;
        let tls = postgres_native_tls::MakeTlsConnector::new(tls);
        let (client, connection) = config.connect(tls.clone()).await?;
        tokio::spawn(connection);

        // Read-only until the user explicitly asks for writes (PLAN principle 4).
        // Timestamps render in the machine's time zone.
        let tz = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        client
            .batch_execute(&format!(
                "SET default_transaction_read_only = on; SET TimeZone = '{}'",
                tz.replace('\'', "''")
            ))
            .await?;
        Ok(Pg { client, tls })
    }

    pub async fn query(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        let started = Instant::now();
        // Simple-query protocol returns every value in Postgres' own text format.
        // With several statements, the last result set wins.
        let stream = self.client.simple_query_raw(sql).await?;
        futures_util::pin_mut!(stream);
        let (mut columns, mut rows, mut truncated) = (Vec::new(), Vec::new(), false);
        while let Some(message) = stream.next().await {
            match message? {
                SimpleQueryMessage::RowDescription(description) => {
                    columns = description.iter().map(|c| c.name().to_owned()).collect();
                    rows.clear();
                }
                SimpleQueryMessage::Row(row) => {
                    if rows.len() == max_rows {
                        truncated = true;
                        break;
                    }
                    rows.push((0..row.len()).map(|i| row.get(i).map(str::to_owned)).collect());
                }
                _ => {}
            }
        }
        if truncated {
            // Stop the server producing rows nobody will read, then drain what's
            // in flight so the session is idle before the next statement.
            self.canceller().cancel().await?;
            while stream.next().await.is_some() {}
        }
        Ok(QueryResult { columns, rows, truncated, elapsed_ms: ms(started) })
    }

    pub fn canceller(&self) -> Canceller {
        Canceller::Postgres(self.client.cancel_token(), self.tls.clone())
    }

    pub async fn set_writable(&self, writable: bool) -> Result<()> {
        let value = if writable { "off" } else { "on" };
        Ok(self.client.batch_execute(&format!("SET default_transaction_read_only = {value}")).await?)
    }

    pub async fn relations(&self) -> Result<Vec<Relation>> {
        let rows = self
            .client
            .query(
                "SELECT n.nspname::text, c.relname::text, c.relkind::text,
                        CASE WHEN c.relkind IN ('r', 'm', 'p') AND c.reltuples >= 0
                             THEN c.reltuples::int8 END
                   FROM pg_class c
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
                    AND n.nspname NOT IN ('pg_catalog', 'information_schema')
                    AND n.nspname NOT LIKE 'pg\\_toast%'
                    AND n.nspname NOT LIKE 'pg\\_temp%'
                  ORDER BY 1, 2",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Relation {
                schema: r.get(0),
                name: r.get(1),
                kind: match r.get::<_, String>(2).as_str() {
                    "v" => RelationKind::View,
                    "m" => RelationKind::MaterializedView,
                    _ => RelationKind::Table,
                },
                estimated_rows: r.get::<_, Option<i64>>(3).map(|n| n as u64),
            })
            .collect())
    }

    pub async fn completion_schema(&self) -> Result<Vec<CompletionTable>> {
        let rows = self
            .client
            .query(
                "SELECT n.nspname::text, c.relname::text,
                        array(SELECT a.attname::text FROM pg_attribute a
                               WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
                               ORDER BY a.attnum)
                   FROM pg_class c
                   JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
                    AND n.nspname NOT IN ('pg_catalog', 'information_schema')
                    AND n.nspname NOT LIKE 'pg\\_toast%'
                    AND n.nspname NOT LIKE 'pg\\_temp%'
                  ORDER BY 1, 2",
                &[],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| CompletionTable { schema: r.get(0), name: r.get(1), columns: r.get(2) })
            .collect())
    }

    async fn oid(&self, relation: &RelationRef) -> Result<u32> {
        self.client
            .query_opt(
                "SELECT c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                  WHERE n.nspname = $1 AND c.relname = $2",
                &[&relation.schema, &relation.name],
            )
            .await?
            .map(|r| r.get(0))
            .ok_or_else(|| Error::NotFound(format!("{}.{}", relation.schema, relation.name)))
    }

    pub async fn describe(&self, relation: &RelationRef) -> Result<TableInfo> {
        let oid = self.oid(relation).await?;
        let by_oid: &[&(dyn ToSql + Sync)] = &[&oid];
        let (columns, indexes, foreign_keys) = tokio::try_join!(
            self.client.query(
                "SELECT a.attname::text,
                        format_type(a.atttypid, a.atttypmod),
                        format_type(a.atttypid, NULL),
                        NOT a.attnotnull,
                        pg_get_expr(d.adbin, d.adrelid),
                        coalesce(a.attnum = ANY (i.indkey), false)
                   FROM pg_attribute a
                   LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
                   LEFT JOIN pg_index i ON i.indrelid = a.attrelid AND i.indisprimary
                  WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
                  ORDER BY a.attnum",
                by_oid,
            ),
            self.client.query(
                "SELECT ic.relname::text, i.indisunique, i.indisprimary,
                        array(SELECT pg_get_indexdef(i.indexrelid, k, true)
                                FROM generate_series(1, i.indnkeyatts) k ORDER BY k)
                   FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid
                  WHERE i.indrelid = $1
                  ORDER BY i.indisprimary DESC, ic.relname",
                by_oid,
            ),
            self.client.query(
                "SELECT con.conname::text,
                        array(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY k(n, o)
                                JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.n
                               ORDER BY k.o),
                        rn.nspname::text, rc.relname::text,
                        array(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY k(n, o)
                                JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.n
                               ORDER BY k.o)
                   FROM pg_constraint con
                   JOIN pg_class rc ON rc.oid = con.confrelid
                   JOIN pg_namespace rn ON rn.oid = rc.relnamespace
                  WHERE con.conrelid = $1 AND con.contype = 'f'
                  ORDER BY con.conname",
                by_oid,
            ),
        )?;
        Ok(TableInfo {
            columns: columns
                .iter()
                .map(|r| Column {
                    name: r.get(0),
                    data_type: r.get(1),
                    base_type: r.get(2),
                    nullable: r.get(3),
                    default: r.get(4),
                    primary_key: r.get(5),
                })
                .collect(),
            indexes: indexes
                .iter()
                .map(|r| Index { name: r.get(0), unique: r.get(1), primary: r.get(2), columns: r.get(3) })
                .collect(),
            foreign_keys: foreign_keys
                .iter()
                .map(|r| ForeignKey {
                    name: Some(r.get(0)),
                    columns: r.get(1),
                    ref_schema: r.get(2),
                    ref_table: r.get(3),
                    ref_columns: r.get(4),
                })
                .collect(),
        })
    }

    pub async fn count(&self, relation: &RelationRef, filters: &[Filter], timeout: Duration) -> Result<Count> {
        let info = self.describe(relation).await?;
        let from = format!("{}.{}", quote(&relation.schema), quote(&relation.name));
        let (sql, params) = sql::count_statement(&DIALECT, &from, &info.columns, filters)?;
        let params: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as _).collect();

        match tokio::time::timeout(timeout, self.client.query_one(&sql, &params)).await {
            Ok(row) => Ok(Count { rows: Some(row?.get::<_, i64>(0) as u64), exact: true }),
            Err(_) => {
                self.client.cancel_token().cancel_query(self.tls.clone()).await?;
                // Without filters the planner's estimate is a fair stand-in.
                let estimate = if filters.is_empty() {
                    self.client
                        .query_one(
                            "SELECT CASE WHEN reltuples >= 0 THEN reltuples::int8 END FROM pg_class WHERE oid = $1",
                            &[&self.oid(relation).await?],
                        )
                        .await?
                        .get::<_, Option<i64>>(0)
                        .map(|n| n as u64)
                } else {
                    None
                };
                Ok(Count { rows: estimate, exact: false })
            }
        }
    }

    pub async fn page(&self, request: &PageRequest) -> Result<Page> {
        let started = Instant::now();
        let info = self.describe(&request.relation).await?;
        let from = format!("{}.{}", quote(&request.relation.schema), quote(&request.relation.name));
        let (sql, params) = sql::page_statement(&DIALECT, &from, &info.columns, request)?;

        let params: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as _).collect();
        let mut rows: Vec<Vec<Option<String>>> = self
            .client
            .query(&sql, &params)
            .await?
            .iter()
            .map(|r| (0..r.len()).map(|i| r.get(i)).collect())
            .collect();
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
    }
}

const DIALECT: Dialect = Dialect {
    param: |n| format!("${n}::text"),
    // Cast the text parameter to the column's type so indexes stay usable and
    // a bad value fails with Postgres' own type error.
    compare: |col, op, p| format!("{} {op} CAST({p} AS {})", quote(&col.name), col.base_type),
    contains: |col, p| format!("strpos(lower({}::text), lower({p})) > 0", quote(&col.name)),
    select: |col| format!("{0}::text AS {0}", quote(&col.name)),
};

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
