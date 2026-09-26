//! Agent access: read-only no matter what SQL arrives, bounded in time and
//! rows, limited to connections marked for agents, and always audited.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{postgres, postgres_target, sqlite_target};
use fabio_core::{Agent, AuditLog, Limits, ReadOnlyDb, RelationRef, SavedConnection, Source, Store, Target};

fn limits() -> Limits {
    Limits { max_rows: 500, timeout: Duration::from_secs(1) }
}

async fn pg() -> ReadOnlyDb {
    ReadOnlyDb::open(&postgres_target(), limits()).await.unwrap()
}

async fn lite() -> ReadOnlyDb {
    ReadOnlyDb::open(&sqlite_target(), limits()).await.unwrap()
}

/// A real table (temp tables are writable even in read-only transactions),
/// created and dropped through a normal, writable connection.
struct Probe(String);

impl Probe {
    async fn create() -> Probe {
        let name = format!("perf.fabio_agent_probe_{}_{:x}", std::process::id(), rand());
        let db = postgres().await;
        db.set_writable(true).await.unwrap();
        db.query(&format!("create table {name} (x int)")).await.unwrap();
        Probe(name)
    }

    async fn rows(&self) -> usize {
        postgres().await.query(&format!("select * from {}", self.0)).await.unwrap().rows.len()
    }

    async fn drop(self) {
        let db = postgres().await;
        db.set_writable(true).await.unwrap();
        db.query(&format!("drop table {}", self.0)).await.unwrap();
    }
}

// --- Postgres guardrails ----------------------------------------------------

#[tokio::test]
async fn postgres_agent_reads() {
    let result = pg().await.query("select name from artist where artist_id = 1").await.unwrap();
    assert_eq!(result.rows, [[Some("AC/DC".to_string())]]);
}

#[tokio::test]
async fn postgres_agent_cannot_write_by_any_route() {
    let probe = Probe::create().await;
    let t = &probe.0;
    let db = pg().await;
    let attempts = [
        format!("insert into {t} values (1)"),
        format!("set default_transaction_read_only = off; insert into {t} values (1)"),
        format!("commit; insert into {t} values (1)"),
        format!("with w as (insert into {t} values (1) returning x) select * from w"),
        format!("begin read write; insert into {t} values (1); commit"),
    ];
    for sql in &attempts {
        assert!(db.query(sql).await.is_err(), "allowed: {sql}");
    }
    // Session state can't be changed for a later statement either.
    for sql in ["set default_transaction_read_only = off", "begin read write", "set session characteristics as transaction read write"] {
        let _ = db.query(sql).await;
        assert!(db.query(&format!("insert into {t} values (1)")).await.is_err(), "allowed after: {sql}");
    }
    let rows = probe.rows().await;
    probe.drop().await;
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn postgres_agent_runs_only_reading_statements() {
    // A read-only transaction doesn't stop these; a superuser's COPY TO PROGRAM runs a shell.
    let probe = std::env::temp_dir().join(format!("fabio-copy-probe-{}", std::process::id()));
    let db = pg().await;
    for sql in [
        format!("copy (select 1) to program 'touch {}'", probe.display()),
        format!("copy artist to '{}'", probe.display()),
        "do $$ begin perform 1; end $$".to_string(),
        "call nothing()".to_string(),
    ] {
        let err = db.query(&sql).await.unwrap_err();
        assert!(err.to_string().contains("only read"), "{sql}: {err}");
    }
    assert!(!probe.exists());

    for sql in ["/* hi */ select 1", "-- hi\n(select 1)", "with a as (select 1) select * from a", "values (1)", "table artist", "show work_mem", "explain select 1"] {
        db.query(sql).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
}

#[tokio::test]
async fn postgres_agent_runs_one_statement_at_a_time() {
    let err = pg().await.query("select 1; select 2").await.unwrap_err();
    assert!(err.to_string().contains("one statement"), "{err}");
}

#[tokio::test]
async fn postgres_agent_statements_time_out() {
    let db = pg().await;
    let started = Instant::now();
    let err = db.query("select pg_sleep(5)").await.unwrap_err();
    assert!(err.to_string().contains("statement timeout"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));

    // Turning the timeout off doesn't outlive the statement that did it.
    let _ = db.query("set statement_timeout = 0").await;
    assert!(db.query("select pg_sleep(5)").await.is_err());
}

#[tokio::test]
async fn postgres_agent_rows_are_capped() {
    let result = pg().await.query("select generate_series(1, 100000)").await.unwrap();
    assert_eq!(result.rows.len(), 500);
    assert!(result.truncated);
}

#[tokio::test]
async fn postgres_agent_explain_analyze_cannot_write() {
    let probe = Probe::create().await;
    let db = pg().await;
    let t = &probe.0;
    assert!(db.explain(&format!("insert into {t} values (1)"), true).await.is_err());
    assert!(db.explain(&format!("select 1; insert into {t} values (1)"), true).await.is_err());
    let plan = db.explain("select * from perf.big where label = 'x'", false).await.unwrap();
    let rows = probe.rows().await;
    probe.drop().await;
    assert_eq!(rows, 0);
    assert!(!plan.analyzed);
}

#[tokio::test]
async fn postgres_agent_samples_rows() {
    let db = pg().await;
    let page = db.sample(&RelationRef { schema: "public".into(), name: "artist".into() }, 5).await.unwrap();
    assert_eq!(page.rows.len(), 5);
    assert_eq!(page.columns[0].name, "artist_id");
}

// --- SQLite guardrails ------------------------------------------------------

#[tokio::test]
async fn sqlite_agent_cannot_write_or_attach() {
    let Target::Sqlite { path } = sqlite_target() else { unreachable!() };
    let copy = std::env::temp_dir().join(format!("fabio-agent-{}.sqlite", std::process::id()));
    std::fs::copy(path, &copy).unwrap();
    let db = ReadOnlyDb::open(&Target::Sqlite { path: copy.clone() }, limits()).await.unwrap();

    assert!(db.query("insert into Genre (GenreId, Name) values (99, 'x')").await.is_err());
    let _ = db.query("pragma query_only = off").await;
    assert!(db.query("delete from Genre").await.is_err());
    let other = std::env::temp_dir().join(format!("fabio-agent-attach-{}.sqlite", std::process::id()));
    assert!(db.query(&format!("attach '{}' as other", other.display())).await.is_err());
    assert!(!other.exists());
    assert_eq!(db.query("select count(*) from Genre").await.unwrap().rows, [[Some("25".to_string())]]);

    std::fs::remove_file(copy).unwrap();
}

#[tokio::test]
async fn sqlite_agent_runs_one_statement_at_a_time() {
    let err = lite().await.query("select 1; select 2").await.unwrap_err();
    assert!(err.to_string().contains("one statement"), "{err}");
}

#[tokio::test]
async fn sqlite_agent_statements_time_out() {
    let started = Instant::now();
    let err = lite()
        .await
        .query("with recursive c(x) as (select 1 union all select x + 1 from c) select count(*) from c")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("interrupt"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn sqlite_agent_rows_are_capped() {
    let result = lite().await.query("select * from Track").await.unwrap();
    assert_eq!(result.rows.len(), 500);
    assert!(result.truncated);
}

// --- Allowlist and audit ----------------------------------------------------

fn agent_with(connections: &[(&str, bool, Target)]) -> (Agent, Arc<AuditLog>) {
    let dir = std::env::temp_dir().join(format!("fabio-agent-store-{}-{:x}", std::process::id(), rand()));
    let store = Store::new(dir.join("connections.json"));
    for (name, agent, target) in connections {
        store
            .save(SavedConnection { id: name.to_string(), name: name.to_string(), target: target.clone(), agent: *agent })
            .unwrap();
    }
    let audit = Arc::new(AuditLog::open(":memory:").unwrap());
    (Agent::new(store, audit.clone(), limits(), Box::new(|_| Ok(None))), audit)
}

fn rand() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

#[tokio::test]
async fn agent_sees_only_connections_marked_for_agents() {
    let (agent, _) = agent_with(&[("chinook", true, postgres_target()), ("private", false, sqlite_target())]);

    let names: Vec<_> = agent.connections().unwrap().into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["chinook"]);
    let err = agent.query("private", "select 1").await.unwrap_err();
    assert!(err.to_string().contains("not open to agents"), "{err}");
}

#[tokio::test]
async fn agent_statements_are_audited() {
    let (agent, audit) = agent_with(&[("chinook", true, postgres_target())]);

    agent.query("chinook", "select 1").await.unwrap();
    agent.query("chinook", "select nme from artist").await.unwrap_err();
    agent.explain("chinook", "select * from artist", true).await.unwrap();
    // Browsing isn't audited, same as in the app.
    agent.tables("chinook").await.unwrap();

    let log = audit.recent(Some("chinook"), 10).unwrap();
    let sql: Vec<_> = log.iter().map(|e| e.sql.as_str()).collect();
    assert_eq!(sql, ["EXPLAIN ANALYZE select * from artist", "select nme from artist", "select 1"]);
    assert!(log.iter().all(|e| e.source == Source::Agent));
    assert_eq!(log[2].rows, Some(1));
    assert!(log[1].error.as_deref().unwrap().contains("nme"));
}
