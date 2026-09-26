//! Query-tab behaviour: row cap, cancel, write mode, error positions.

mod common;

use std::time::{Duration, Instant};

use common::{postgres, sqlite, sqlite_target};
use fabiodb_core::{Db, Target};

#[tokio::test]
async fn postgres_result_is_capped_and_flagged() {
    let db = postgres().await;
    let result = db.query_limited("select generate_series(1, 100000) as n", 1000).await.unwrap();

    assert_eq!(result.rows.len(), 1000);
    assert!(result.truncated);
    assert_eq!(result.rows[999][0].as_deref(), Some("1000"));
    // The rest of the stream is cancelled and the session stays usable.
    assert_eq!(db.query("select 1").await.unwrap().rows, [[Some("1".into())]]);
}

#[tokio::test]
async fn small_result_is_not_truncated() {
    let result = postgres().await.query_limited("select 1", 1000).await.unwrap();
    assert!(!result.truncated);
}

#[tokio::test]
async fn sqlite_result_is_capped_and_flagged() {
    let result = sqlite().await.query_limited("select * from Track", 10).await.unwrap();

    assert_eq!(result.rows.len(), 10);
    assert!(result.truncated);
}

#[tokio::test]
async fn postgres_query_can_be_cancelled() {
    let db = std::sync::Arc::new(postgres().await);
    let canceller = db.canceller();
    let started = Instant::now();
    let running = tokio::spawn({
        let db = db.clone();
        async move { db.query("select pg_sleep(10)").await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    canceller.cancel().await.unwrap();

    let err = running.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("cancel"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn sqlite_query_can_be_cancelled() {
    let db = std::sync::Arc::new(sqlite().await);
    let canceller = db.canceller();
    let running = tokio::spawn({
        let db = db.clone();
        async move {
            db.query("with recursive c(x) as (select 1 union all select x + 1 from c) select count(*) from c").await
        }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    canceller.cancel().await.unwrap();

    let err = running.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("interrupt"), "{err}");
}

#[tokio::test]
async fn postgres_write_mode_is_explicit() {
    // Temp tables are writable even in read-only transactions, so probe with a real one.
    let table = format!("fabio_probe_{}", std::process::id());
    let db = postgres().await;
    assert!(db.query(&format!("create table {table} (x int)")).await.is_err());

    db.set_writable(true).await.unwrap();
    db.query(&format!("create table {table} (x int)")).await.unwrap();

    db.set_writable(false).await.unwrap();
    let blocked = db.query(&format!("insert into {table} values (1)")).await;

    db.set_writable(true).await.unwrap();
    db.query(&format!("drop table {table}")).await.unwrap();
    assert!(blocked.is_err());
}

#[tokio::test]
async fn sqlite_write_mode_reopens_the_file_writable() {
    let Target::Sqlite { path } = sqlite_target() else { unreachable!() };
    let copy = std::env::temp_dir().join(format!("fabio-write-{}.sqlite", std::process::id()));
    std::fs::copy(path, &copy).unwrap();
    let db = Db::open(&Target::Sqlite { path: copy.clone() }).await.unwrap();

    assert!(db.query("create table probe (x)").await.is_err());
    db.set_writable(true).await.unwrap();
    db.query("create table probe (x)").await.unwrap();
    db.set_writable(false).await.unwrap();
    assert!(db.query("insert into probe values (1)").await.is_err());

    std::fs::remove_file(copy).unwrap();
}

#[tokio::test]
async fn postgres_error_reports_its_position() {
    let err = postgres().await.query("select nme from artist").await.unwrap_err();

    assert_eq!(err.position(), Some(8), "{err}");
}

#[tokio::test]
async fn postgres_knows_when_its_connection_is_gone() {
    let db = postgres().await;
    assert!(!db.is_closed());
    let pid = db.query("select pg_backend_pid()").await.unwrap().rows[0][0].clone().unwrap();

    postgres().await.query(&format!("select pg_terminate_backend({pid})")).await.unwrap();
    // The next use fails, and from then on the connection says it's closed.
    assert!(db.query("select 1").await.is_err());
    for _ in 0..50 {
        if db.is_closed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(db.is_closed());
}
