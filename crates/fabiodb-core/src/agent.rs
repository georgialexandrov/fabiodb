//! Agent access, as the MCP server offers it. The guardrails live here, at the
//! connection, not in any prompt:
//!
//! - Postgres: every statement runs alone (no `;`-chained second statement)
//!   inside `BEGIN READ ONLY … ROLLBACK`, so it can't write and nothing it
//!   `SET`s survives it; it must be a reading statement (no COPY, DO, CALL);
//!   `statement_timeout` bounds it.
//! - SQLite: the file is opened read-only, `query_only` is on, ATTACH is off,
//!   and a timer interrupts long statements.
//! - Both: a row cap, and only connections marked for agents.
//!
//! Every statement goes to the audit log as `Source::Agent`, which is how the
//! app shows what the agent did.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::{
    AuditLog, Db, Error, Insights, NewAuditEntry, Page, PageRequest, Plan, QueryResult, Relation, RelationRef, Result,
    SavedConnection, Source, Store, TableInfo, Target,
};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_rows: usize,
    pub timeout: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits { max_rows: 500, timeout: Duration::from_secs(10) }
    }
}

/// A connection that can only read. There is no way to make it writable.
pub struct ReadOnlyDb {
    db: Db,
    limits: Limits,
}

impl ReadOnlyDb {
    pub async fn open(target: &Target, limits: Limits) -> Result<ReadOnlyDb> {
        let db = Db::open(target).await?;
        match &db {
            Db::Postgres(pg) => {
                pg.set_statement_timeout(limits.timeout).await?;
                // So Insights and pg_stat_activity show whose session it is.
                pg.client().batch_execute("SET /* fabio */ application_name = 'fabio agent'").await?;
            }
            Db::Sqlite(lite) => lite.guard().await?,
        }
        Ok(ReadOnlyDb { db, limits })
    }

    pub async fn query(&self, sql: &str) -> Result<QueryResult> {
        match &self.db {
            Db::Postgres(pg) => {
                check_reads(sql)?;
                pg.query_guarded(sql, self.limits.max_rows).await
            }
            Db::Sqlite(lite) => lite.query_guarded(sql, self.limits.max_rows, self.limits.timeout).await,
        }
    }

    /// With `analyze` on Postgres, the statement runs inside a read-only
    /// transaction that is rolled back. SQLite only ever plans.
    pub async fn explain(&self, sql: &str, analyze: bool) -> Result<Plan> {
        let sql = sql.trim().trim_end_matches(';');
        match &self.db {
            Db::Postgres(pg) => pg.explain(sql, analyze, true).await,
            Db::Sqlite(lite) => lite.explain(sql).await,
        }
    }

    pub async fn relations(&self) -> Result<Vec<Relation>> {
        self.db.relations().await
    }

    pub async fn describe(&self, relation: &RelationRef) -> Result<TableInfo> {
        self.db.describe(relation).await
    }

    /// The first `rows` rows of a table, in primary-key order.
    pub async fn sample(&self, relation: &RelationRef, rows: u32) -> Result<Page> {
        let limit = rows.min(self.limits.max_rows as u32);
        self.db.page(&PageRequest { relation: relation.clone(), sort: None, filters: vec![], offset: 0, limit }).await
    }

    pub async fn insights(&self) -> Result<Insights> {
        self.db.insights().await
    }
}

/// Statements that can only read. A read-only transaction still lets a
/// superuser run `COPY … TO PROGRAM` or a `DO` block, so on Postgres the
/// agent's statement must also be one of these. The statement is already
/// known to be a single one, so its first keyword says what it is.
const READING: &[&str] = &["select", "with", "values", "table", "show", "explain"];

fn check_reads(sql: &str) -> Result<()> {
    let keyword = first_keyword(sql).to_ascii_lowercase();
    if READING.contains(&keyword.as_str()) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "Agents can only read: SELECT, WITH, VALUES, TABLE, SHOW or EXPLAIN. This statement starts with {}.",
            if keyword.is_empty() { "nothing".to_string() } else { keyword.to_ascii_uppercase() }
        )))
    }
}

/// The first word, skipping whitespace, comments and opening parentheses.
fn first_keyword(sql: &str) -> &str {
    let mut rest = sql;
    loop {
        let trimmed = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
        if let Some(after) = trimmed.strip_prefix("--") {
            rest = after.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            // Postgres block comments nest.
            let mut depth = 1;
            let mut i = 0;
            let bytes = after.as_bytes();
            while depth > 0 && i < bytes.len() {
                if bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            rest = &after[i..];
        } else {
            let end = trimmed.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(trimmed.len());
            return &trimmed[..end];
        }
    }
}

/// A connection as the agent sees it: no host, user or password.
#[derive(Debug, Clone, Serialize)]
pub struct AgentConnection {
    pub id: String,
    pub name: String,
    pub engine: &'static str,
}

/// Where saved connections' passwords live (the OS keychain, in practice),
/// keyed by connection id.
pub trait Keychain: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<String>>;
    fn set(&self, id: &str, password: &str) -> Result<()>;
}

/// Group for connections an agent added, so they're easy to find and review.
pub const AGENT_GROUP: &str = "Added by agents";

/// Everything the MCP server's tools do, over the saved connections.
pub struct Agent {
    store: Store,
    audit: Arc<AuditLog>,
    limits: Limits,
    keychain: Box<dyn Keychain>,
    open: Mutex<HashMap<String, Arc<ReadOnlyDb>>>,
}

impl Agent {
    pub fn new(store: Store, audit: Arc<AuditLog>, limits: Limits, keychain: Box<dyn Keychain>) -> Agent {
        Agent { store, audit, limits, keychain, open: Mutex::default() }
    }

    /// Saves a new connection from a `postgres://` URL or a SQLite file path,
    /// after checking it connects. It's open to agents (the agent supplied the
    /// credentials, so it could reach the database anyway), grouped under
    /// [`AGENT_GROUP`], and the addition shows in the agent panel.
    pub async fn create_connection(&self, name: &str, url_or_path: &str) -> Result<AgentConnection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("A connection needs a name.".into()));
        }
        let target = if url_or_path.starts_with("postgres://") || url_or_path.starts_with("postgresql://") {
            Target::Postgres(crate::PgTarget::from_url(url_or_path)?)
        } else {
            Target::Sqlite { path: url_or_path.into() }
        };
        // Nothing half-working gets saved.
        let db = ReadOnlyDb::open(&target, self.limits).await?;
        db.query("SELECT 1").await?;

        let password = match &target {
            Target::Postgres(pg) => pg.password.clone(),
            Target::Sqlite { .. } => None,
        };
        let described = match &target {
            Target::Postgres(pg) => format!("{}@{}:{}/{} (Postgres)", pg.user, pg.host, pg.port, pg.database),
            Target::Sqlite { path } => format!("{} (SQLite)", path.display()),
        };
        let saved = self.store.save(SavedConnection {
            id: String::new(),
            name: name.to_owned(),
            target,
            agent: true,
            group: Some(AGENT_GROUP.into()),
        })?;
        if let Some(password) = password {
            self.keychain.set(&saved.id, &password)?;
        }
        self.open.lock().expect("agent connections poisoned").insert(saved.id.clone(), Arc::new(db));
        let entry = NewAuditEntry {
            connection_id: saved.id.clone(),
            source: Source::Agent,
            sql: format!("-- Agent added connection “{name}”: {described}"),
            elapsed_ms: 0.0,
            rows: None,
            error: None,
        };
        if let Err(e) = self.audit.record(&entry) {
            eprintln!("audit log: {e}");
        }
        Ok(AgentConnection {
            id: saved.id,
            name: saved.name,
            engine: if matches!(saved.target, Target::Postgres(_)) { "postgres" } else { "sqlite" },
        })
    }

    /// Connections marked for agents. Read fresh each time, so unticking one
    /// in the app takes effect on the next call.
    pub fn connections(&self) -> Result<Vec<AgentConnection>> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .filter(|c| c.agent)
            .map(|c| AgentConnection {
                engine: match c.target {
                    Target::Postgres(_) => "postgres",
                    Target::Sqlite { .. } => "sqlite",
                },
                id: c.id,
                name: c.name,
            })
            .collect())
    }

    pub async fn tables(&self, connection: &str) -> Result<Vec<Relation>> {
        self.db(connection).await?.1.relations().await
    }

    pub async fn describe(&self, connection: &str, relation: &RelationRef) -> Result<TableInfo> {
        self.db(connection).await?.1.describe(relation).await
    }

    pub async fn sample(&self, connection: &str, relation: &RelationRef, rows: u32) -> Result<Page> {
        self.db(connection).await?.1.sample(relation, rows).await
    }

    pub async fn insights(&self, connection: &str) -> Result<Insights> {
        self.db(connection).await?.1.insights().await
    }

    pub async fn query(&self, connection: &str, sql: &str) -> Result<QueryResult> {
        let (id, db) = self.db(connection).await?;
        let started = Instant::now();
        let result = db.query(sql).await;
        self.record(
            id,
            sql.to_owned(),
            started,
            result.as_ref().ok().map(|r| r.rows.len() as u64),
            result.as_ref().err(),
        );
        result
    }

    pub async fn explain(&self, connection: &str, sql: &str, analyze: bool) -> Result<Plan> {
        let (id, db) = self.db(connection).await?;
        let started = Instant::now();
        let plan = db.explain(sql, analyze).await;
        let logged = format!("EXPLAIN{} {sql}", if analyze { " ANALYZE" } else { "" });
        self.record(id, logged, started, None, plan.as_ref().err());
        plan
    }

    fn record(&self, connection_id: String, sql: String, started: Instant, rows: Option<u64>, error: Option<&Error>) {
        let entry = NewAuditEntry {
            connection_id,
            source: Source::Agent,
            sql,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            rows,
            error: error.map(ToString::to_string),
        };
        if let Err(e) = self.audit.record(&entry) {
            eprintln!("audit log: {e}");
        }
    }

    /// By id or by name. Opens the connection on first use.
    async fn db(&self, connection: &str) -> Result<(String, Arc<ReadOnlyDb>)> {
        let all = self.store.list()?;
        let saved: SavedConnection = all
            .iter()
            .find(|c| c.id == connection)
            .or_else(|| all.iter().find(|c| c.name == connection))
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("connection “{connection}”")))?;
        if !saved.agent {
            self.open.lock().expect("agent connections poisoned").remove(&saved.id);
            return Err(Error::Invalid(format!(
                "“{}” is not open to agents. Tick “Agents can query” in its connection settings in Fabio.",
                saved.name
            )));
        }
        if let Some(db) = self.open.lock().expect("agent connections poisoned").get(&saved.id) {
            return Ok((saved.id, db.clone()));
        }
        let mut target = saved.target;
        if let Target::Postgres(pg) = &mut target {
            pg.password = self.keychain.get(&saved.id)?;
        }
        let db = Arc::new(ReadOnlyDb::open(&target, self.limits).await?);
        self.open.lock().expect("agent connections poisoned").insert(saved.id.clone(), db.clone());
        Ok((saved.id, db))
    }
}

#[cfg(test)]
mod tests {
    use super::first_keyword;

    #[test]
    fn first_keyword_skips_comments_and_parens() {
        assert_eq!(first_keyword("  select 1"), "select");
        assert_eq!(first_keyword("-- a\n-- b\n WITH x"), "WITH");
        assert_eq!(first_keyword("/* a /* nested */ still */ copy x"), "copy");
        assert_eq!(first_keyword("((select 1))"), "select");
        assert_eq!(first_keyword("-- only a comment"), "");
        assert_eq!(first_keyword("/* unterminated"), "");
    }
}
