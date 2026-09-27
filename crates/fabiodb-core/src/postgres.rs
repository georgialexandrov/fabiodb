use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, SimpleQueryMessage, types::ToSql};

use crate::edit::{self, Changes, EditDialect};
use crate::export::{self, ExportFormat, RowWriter};
use crate::sql::{self, Dialect, quote};
use crate::{
    Canceller, Column, CompletionTable, Count, Error, Filter, ForeignKey, Index, Page, PageRequest, QueryResult,
    Relation, RelationKind, RelationRef, Result, ResultColumn, Sort, TableInfo,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    /// Use TLS if the server offers it. Like libpq, the certificate is not verified.
    Prefer,
    /// Require TLS. Like libpq, the certificate is not verified.
    Require,
    /// Require TLS and a certificate signed by a trusted CA (the system's, or `ca_cert`).
    #[serde(rename = "verify-ca")]
    VerifyCa,
    /// As `VerifyCa`, and the certificate must name the host connected to.
    #[serde(rename = "verify-full")]
    #[default]
    VerifyFull,
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
    /// Reach the server through this SSH host; `host`/`port` are then as seen from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<crate::SshTunnel>,
    /// PEM file of the CA that signed the server's certificate, for the verify
    /// modes when it isn't a CA the system already trusts (libpq's sslrootcert).
    #[serde(default)]
    pub ca_cert: Option<PathBuf>,
}

impl SslMode {
    /// The default when none is given: verify the server's certificate, except
    /// on this machine (local servers and Docker rarely have TLS, and the
    /// traffic doesn't leave the host).
    pub fn for_host(host: &str) -> SslMode {
        let local = host == "localhost"
            || host.starts_with('/')
            || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if local { SslMode::Prefer } else { SslMode::VerifyFull }
    }
}

impl PgTarget {
    pub fn from_url(url: &str) -> Result<PgTarget> {
        // tokio-postgres knows neither verify mode nor sslrootcert; take them out first.
        let (url, ssl, ca_cert) = split_tls_params(url);
        let config = tokio_postgres::Config::from_str(&url)?;
        let host = match config.get_hosts().first() {
            Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
            #[cfg(unix)]
            Some(tokio_postgres::config::Host::Unix(p)) => p.display().to_string(),
            None => "localhost".into(),
        };
        let user = config.get_user().unwrap_or_default().to_owned();
        let ssl = ssl.unwrap_or_else(|| SslMode::for_host(&host));
        Ok(PgTarget {
            host,
            port: config.get_ports().first().copied().unwrap_or(5432),
            database: config.get_dbname().map_or_else(|| user.clone(), str::to_owned),
            user,
            password: config.get_password().map(|p| String::from_utf8_lossy(p).into_owned()),
            ssl,
            ca_cert,
            ssh: None,
        })
    }
}

/// The URL without Fabio's TLS parameters, and what they said. With no
/// `sslmode`, Fabio verifies the server name and certificate by default.
fn split_tls_params(url: &str) -> (String, Option<SslMode>, Option<PathBuf>) {
    let Some((base, query)) = url.split_once('?') else { return (url.to_owned(), None, None) };
    let (mut ssl, mut ca_cert, mut kept) = (None, None, Vec::new());
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("sslmode", "disable")) => ssl = Some(SslMode::Disable),
            Some(("sslmode", "prefer")) => ssl = Some(SslMode::Prefer),
            Some(("sslmode", "require")) => ssl = Some(SslMode::Require),
            Some(("sslmode", "verify-ca")) => ssl = Some(SslMode::VerifyCa),
            Some(("sslmode", "verify-full")) => ssl = Some(SslMode::VerifyFull),
            Some(("sslrootcert", path)) => ca_cert = Some(PathBuf::from(percent_decode(path))),
            _ => kept.push(pair),
        }
    }
    let url = if kept.is_empty() { base.to_owned() } else { format!("{base}?{}", kept.join("&")) };
    (url, ssl, ca_cert)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// libpq's semantics: prefer/require encrypt without checking; the verify
/// modes check the chain (and, for verify-full, the host name).
fn tls_connector(target: &PgTarget) -> Result<postgres_native_tls::MakeTlsConnector> {
    let mut builder = native_tls::TlsConnector::builder();
    match target.ssl {
        SslMode::Disable | SslMode::Prefer | SslMode::Require => {
            builder.danger_accept_invalid_certs(true);
        }
        SslMode::VerifyCa | SslMode::VerifyFull => {
            if let Some(path) = &target.ca_cert {
                let pem = std::fs::read(path).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
                let cert = native_tls::Certificate::from_pem(&pem)
                    .map_err(|e| Error::Invalid(format!("{}: not a PEM certificate: {e}", path.display())))?;
                builder.add_root_certificate(cert);
            }
            if target.ssl == SslMode::VerifyCa {
                builder.danger_accept_invalid_hostnames(true);
            }
        }
    }
    Ok(postgres_native_tls::MakeTlsConnector::new(builder.build()?))
}

pub struct Pg {
    client: Client,
    /// Kept open as long as this connection lives.
    _tunnel: Option<std::sync::Arc<crate::tunnel::Tunnel>>,
    /// Kept for cancel requests, which open their own connection.
    tls: postgres_native_tls::MakeTlsConnector,
}

impl Pg {
    pub async fn connect(target: &PgTarget) -> Result<Pg> {
        // Through SSH: connect to the tunnel's local end, but keep `host` as the
        // name TLS checks the certificate against.
        let tunnel = match &target.ssh {
            Some(ssh) => Some(crate::tunnel::open(ssh, &target.host, target.port).await?),
            None => None,
        };
        let mut config = tokio_postgres::Config::new();
        config
            .host(&target.host)
            .port(tunnel.as_ref().map_or(target.port, |t| t.local_port))
            .user(&target.user)
            .dbname(&target.database)
            .application_name("fabio")
            .connect_timeout(Duration::from_secs(5))
            // Notice a dead peer (sleep, network change) in about a minute, not hours.
            .keepalives(true)
            .keepalives_idle(Duration::from_secs(30))
            .ssl_mode(match target.ssl {
                SslMode::Disable => tokio_postgres::config::SslMode::Disable,
                SslMode::Prefer => tokio_postgres::config::SslMode::Prefer,
                SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => tokio_postgres::config::SslMode::Require,
            });
        if tunnel.is_some() {
            config.hostaddr(std::net::IpAddr::from([127, 0, 0, 1]));
        }
        if let Some(password) = &target.password {
            config.password(password);
        }
        let tls = tls_connector(target)?;
        let (client, connection) = config.connect(tls.clone()).await?;
        tokio::spawn(connection);

        // Read-only until the user explicitly asks for writes (PLAN principle 4).
        // Timestamps render in the machine's time zone.
        let tz = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        client
            .batch_execute(&format!(
                "SET /* fabio */ default_transaction_read_only = on; SET /* fabio */ TimeZone = '{}'",
                tz.replace('\'', "''")
            ))
            .await?;
        Ok(Pg { client, tls, _tunnel: tunnel })
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

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// A read-only transaction cannot sandbox superusers, roles that can touch
    /// server files/programs, or ones that can cancel and end other sessions. Agent access is refused for such accounts;
    /// it must use a deliberately least-privilege login.
    pub(crate) async fn ensure_safe_agent_role(&self) -> Result<()> {
        let row = self
            .client
            .query_one(
                "SELECT /* fabio */ r.rolsuper, r.rolcreaterole, r.rolcreatedb,
                        r.rolreplication, r.rolbypassrls,
                        pg_has_role(current_user, 'pg_read_server_files', 'MEMBER'),
                        pg_has_role(current_user, 'pg_write_server_files', 'MEMBER'),
                        pg_has_role(current_user, 'pg_execute_server_program', 'MEMBER'),
                        pg_has_role(current_user, 'pg_signal_backend', 'MEMBER')
                   FROM pg_roles r WHERE r.rolname = current_user",
                &[],
            )
            .await?;
        let privileged = (0..row.len()).any(|i| row.get::<_, bool>(i));
        if privileged {
            return Err(Error::Invalid(
                "Agent access needs a least-privilege Postgres login. Superusers and roles with server-wide or server-file privileges are refused."
                    .into(),
            ));
        }
        Ok(())
    }

    /// Runs one statement inside a read-only transaction that is always rolled
    /// back, so neither the statement nor any `SET` it makes can write or
    /// outlive it. For agents.
    pub async fn query_guarded(&self, sql: &str, max_rows: usize) -> Result<QueryResult> {
        self.client.batch_execute("BEGIN /* fabio */ READ ONLY").await?;
        let result = match self.check_single(sql).await {
            Ok(()) => self.query(sql, max_rows).await,
            Err(e) => Err(e),
        };
        let rolled_back = self.client.batch_execute("ROLLBACK /* fabio */").await;
        let result = result?;
        rolled_back?;
        Ok(result)
    }

    /// The simple-query protocol runs any number of statements; a `COMMIT` in
    /// the middle would end the guarding transaction. Parsing the text as a
    /// prepared statement fails unless it is exactly one statement.
    async fn check_single(&self, sql: &str) -> Result<()> {
        match self.client.prepare(sql).await {
            Ok(_) => Ok(()),
            Err(e) => match Error::from(e) {
                Error::Postgres { message, .. } if message.contains("multiple commands") => {
                    Err(Error::Invalid(ONE_STATEMENT.into()))
                }
                other => Err(other),
            },
        }
    }

    pub async fn set_statement_timeout(&self, timeout: Duration) -> Result<()> {
        Ok(self.client.batch_execute(&format!("SET /* fabio */ statement_timeout = {}", timeout.as_millis())).await?)
    }

    pub async fn explain(&self, sql: &str, analyze: bool, guarded: bool) -> Result<crate::Plan> {
        // VERBOSE is what puts the schema into the JSON.
        let options = if analyze { "ANALYZE, BUFFERS, VERBOSE, FORMAT JSON" } else { "VERBOSE, FORMAT JSON" };
        // ANALYZE executes the statement; the rollback makes that harmless.
        let prefix = format!("EXPLAIN ({options}) ");
        self.client.batch_execute(if guarded { "BEGIN /* fabio */ READ ONLY" } else { "BEGIN /* fabio */" }).await?;
        let explained = match if guarded { self.check_single(sql).await } else { Ok(()) } {
            // Report error positions against the user's statement, not our prefix.
            Ok(()) => self.client.simple_query(&format!("{prefix}{sql}")).await.map_err(|e| match Error::from(e) {
                Error::Postgres { message, position } => Error::Postgres {
                    message,
                    position: position.and_then(|p| p.checked_sub(prefix.chars().count() as u32)),
                },
                other => other,
            }),
            Err(e) => Err(e),
        };
        let rolled_back = self.client.batch_execute("ROLLBACK /* fabio */").await;
        let messages = explained?;
        rolled_back?;
        let raw = messages
            .iter()
            .find_map(|m| match m {
                SimpleQueryMessage::Row(r) => r.get(0).map(str::to_owned),
                _ => None,
            })
            .ok_or_else(|| Error::Invalid("EXPLAIN returned nothing".into()))?;
        crate::plan::from_postgres_json(raw, analyze)
    }

    pub fn canceller(&self) -> Canceller {
        Canceller::Postgres(Box::new((self.client.cancel_token(), self.tls.clone())))
    }

    pub async fn set_writable(&self, writable: bool) -> Result<()> {
        let value = if writable { "off" } else { "on" };
        Ok(self.client.batch_execute(&format!("SET /* fabio */ default_transaction_read_only = {value}")).await?)
    }

    pub async fn relations(&self) -> Result<Vec<Relation>> {
        let rows = self
            .client
            .query(
                "SELECT /* fabio */ n.nspname::text, c.relname::text, c.relkind::text,
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
                "SELECT /* fabio */ n.nspname::text, c.relname::text,
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
        Ok(rows.iter().map(|r| CompletionTable { schema: r.get(0), name: r.get(1), columns: r.get(2) }).collect())
    }

    async fn oid(&self, relation: &RelationRef) -> Result<u32> {
        self.client
            .query_opt(
                "SELECT /* fabio */ c.oid FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
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
                "SELECT /* fabio */ a.attname::text,
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
                "SELECT /* fabio */ ic.relname::text, i.indisunique, i.indisprimary,
                        array(SELECT pg_get_indexdef(i.indexrelid, k, true)
                                FROM generate_series(1, i.indnkeyatts) k ORDER BY k)
                   FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid
                  WHERE i.indrelid = $1
                  ORDER BY i.indisprimary DESC, ic.relname",
                by_oid,
            ),
            self.client.query(
                "SELECT /* fabio */ con.conname::text,
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
                            "SELECT /* fabio */ CASE WHEN reltuples >= 0 THEN reltuples::int8 END FROM pg_class WHERE oid = $1",
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

    pub async fn export_table(
        &self,
        relation: &RelationRef,
        sort: Option<&Sort>,
        filters: &[Filter],
        format: ExportFormat,
        path: &std::path::Path,
    ) -> Result<u64> {
        let info = self.describe(relation).await?;
        let from = format!("{}.{}", quote(&relation.schema), quote(&relation.name));
        let (sql, params) = sql::select_statement(&DIALECT, &from, &info.columns, sort, filters)?;
        let columns = result_columns(&info);
        let out = export::create(path)?;
        let written = async {
            let mut writer = RowWriter::new(format, &columns, Some(relation), out)?;
            // Rows are written as they arrive, so a table of any size fits.
            let stream = self.client.query_raw(&sql, params.iter()).await?;
            futures_util::pin_mut!(stream);
            while let Some(row) = stream.next().await {
                let row = row?;
                writer.row(&(0..row.len()).map(|i| row.get(i)).collect::<Vec<Option<String>>>())?;
            }
            Ok(writer.finish()?.1)
        }
        .await;
        export::remove_on_error(path, written)
    }

    pub async fn apply_changes(&self, relation: &RelationRef, info: &TableInfo, changes: &Changes) -> Result<u64> {
        let from = format!("{}.{}", quote(&relation.schema), quote(&relation.name));
        let steps = edit::steps(&EDIT, &from, info, changes)?;
        self.client.batch_execute("BEGIN /* fabio */").await?;
        let applied = async {
            for step in &steps {
                let params: Vec<&(dyn ToSql + Sync)> = step.params.iter().map(|p| p as _).collect();
                if self.client.execute(&step.sql, &params).await? != 1 {
                    return Err(Error::Invalid(step.if_not_one.to_string()));
                }
            }
            Ok(steps.len() as u64)
        }
        .await;
        match applied {
            Ok(n) => {
                self.client.batch_execute("COMMIT /* fabio */").await?;
                Ok(n)
            }
            Err(e) => {
                self.client.batch_execute("ROLLBACK /* fabio */").await?;
                Err(e)
            }
        }
    }

    pub async fn page(&self, request: &PageRequest) -> Result<Page> {
        let started = Instant::now();
        let info = self.describe(&request.relation).await?;
        let from = format!("{}.{}", quote(&request.relation.schema), quote(&request.relation.name));
        let (sql, params) = sql::page_statement(&DIALECT, &from, &info.columns, request)?;

        let params: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p as _).collect();
        let mut rows: Vec<Vec<Option<String>>> =
            self.client.query(&sql, &params).await?.iter().map(|r| (0..r.len()).map(|i| r.get(i)).collect()).collect();
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

const EDIT: EditDialect = EditDialect {
    param: |n| format!("${n}::text"),
    value: |col, p| format!("CAST({p} AS {})", col.base_type),
    assign: |col, p| format!("{} = CAST({p} AS {})", quote(&col.name), col.base_type),
    key: |col, p| format!("{} = CAST({p} AS {})", quote(&col.name), col.base_type),
    // Compared as text: the value came to the grid as text.
    unchanged: |col, p| format!("{}::text IS NOT DISTINCT FROM {p}", quote(&col.name)),
};

pub(crate) const ONE_STATEMENT: &str = "Agents run one statement at a time. Send the others separately.";

const DIALECT: Dialect = Dialect {
    param: |n| format!("${n}::text"),
    // Cast the text parameter to the column's type so indexes stay usable and
    // a bad value fails with Postgres' own type error.
    compare: |col, op, p| format!("{} {op} CAST({p} AS {})", quote(&col.name), col.base_type),
    contains: |col, p| format!("strpos(lower({}::text), lower({p})) > 0", quote(&col.name)),
    select: |col| format!("{0}::text AS {0}", quote(&col.name)),
};

pub(crate) fn result_columns(info: &TableInfo) -> Vec<ResultColumn> {
    info.columns.iter().map(|c| ResultColumn { name: c.name.clone(), data_type: c.data_type.clone() }).collect()
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
