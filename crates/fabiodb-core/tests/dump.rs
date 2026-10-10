//! Whole databases out and back in: pg_dump / pg_restore / psql for Postgres,
//! Fabio's own dump for SQLite. Postgres needs `dev/pg.sh start` and the
//! Postgres client tools (any version at least the server's).

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use common::{postgres, postgres_target, sqlite_target};
use fabiodb_core::{Db, DumpContent, DumpFormat, DumpOptions, Progress, Target, dump, restore};

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fabio-dump-{}-{name}", std::process::id()))
}

fn quiet() -> Progress {
    Arc::new(|_: &str| {})
}

fn never() -> std::future::Pending<()> {
    std::future::pending()
}

fn public_only() -> DumpOptions {
    // Chinook's database also holds perf.big (5M rows); the tests don't need it.
    DumpOptions { schemas: vec!["public".into()], ..DumpOptions::default() }
}

/// A new, empty database on the dev server, dropped when the guard goes.
struct Scratch {
    name: String,
}

impl Scratch {
    async fn new(tag: &str) -> Scratch {
        let name = format!("fabio_dump_{tag}_{}", std::process::id());
        let admin = postgres().await;
        admin.set_writable(true).await.unwrap();
        admin.query(&format!("DROP DATABASE IF EXISTS {name}")).await.unwrap();
        admin.query(&format!("CREATE DATABASE {name}")).await.unwrap();
        Scratch { name }
    }

    fn target(&self) -> Target {
        let Target::Postgres(mut pg) = postgres_target() else { unreachable!() };
        pg.database = self.name.clone();
        Target::Postgres(pg)
    }

    async fn drop(self) {
        let admin = postgres().await;
        admin.set_writable(true).await.unwrap();
        admin.query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name)).await.unwrap();
    }
}

async fn count(target: &Target, table: &str) -> String {
    let db = Db::open(target).await.unwrap();
    db.query(&format!("select count(*) from {table}")).await.unwrap().rows[0][0].clone().unwrap()
}

#[tokio::test]
async fn postgres_dump_restores_into_an_empty_database() {
    let path = temp("chinook.sql");
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = lines.clone();
    let progress: Progress = Arc::new(move |line: &str| seen.lock().unwrap().push(line.to_owned()));

    let done = dump(&postgres_target(), &public_only(), &path, progress, never()).await.unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(done.tool.as_deref().unwrap().starts_with("pg_dump "), "{:?}", done.tool);
    assert!(text.contains("CREATE TABLE public.track"), "schema is in the dump");
    assert!(!text.contains("perf.big"), "only the schemas asked for");
    assert!(!text.contains("OWNER TO"), "owners are left out by default");
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("public.track")), "progress names tables");

    let scratch = Scratch::new("sql").await;
    restore(&scratch.target(), &path, quiet(), never()).await.unwrap();
    assert_eq!(count(&scratch.target(), "track").await, "3503");
    assert_eq!(count(&scratch.target(), "invoice_line").await, "2240");
    scratch.drop().await;
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn postgres_archive_restores_with_pg_restore() {
    let path = temp("chinook.dump");
    let options = DumpOptions { format: DumpFormat::Archive, ..public_only() };
    dump(&postgres_target(), &options, &path, quiet(), never()).await.unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[..5], b"PGDMP");

    let scratch = Scratch::new("archive").await;
    let done = restore(&scratch.target(), &path, quiet(), never()).await.unwrap();
    assert!(done.tool.as_deref().unwrap().starts_with("pg_restore "), "{:?}", done.tool);
    assert_eq!(count(&scratch.target(), "album").await, "347");
    scratch.drop().await;
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn postgres_structure_only_and_data_only() {
    let schema = temp("schema.sql");
    let data = temp("data.sql");
    dump(&postgres_target(), &DumpOptions { content: DumpContent::Schema, ..public_only() }, &schema, quiet(), never())
        .await
        .unwrap();
    dump(&postgres_target(), &DumpOptions { content: DumpContent::Data, ..public_only() }, &data, quiet(), never())
        .await
        .unwrap();
    let (schema_text, data_text) = (std::fs::read_to_string(&schema).unwrap(), std::fs::read_to_string(&data).unwrap());
    assert!(schema_text.contains("CREATE TABLE public.track") && !schema_text.contains("COPY public.track"));
    assert!(data_text.contains("COPY public.track") && !data_text.contains("CREATE TABLE"));

    // Structure, then data: the same database again.
    let scratch = Scratch::new("parts").await;
    restore(&scratch.target(), &schema, quiet(), never()).await.unwrap();
    assert_eq!(count(&scratch.target(), "track").await, "0");
    restore(&scratch.target(), &data, quiet(), never()).await.unwrap();
    assert_eq!(count(&scratch.target(), "track").await, "3503");
    scratch.drop().await;
    std::fs::remove_file(&schema).unwrap();
    std::fs::remove_file(&data).unwrap();
}

#[tokio::test]
async fn postgres_failed_restore_changes_nothing_and_says_why() {
    let path = temp("half.sql");
    std::fs::write(
        &path,
        "CREATE TABLE fabio_half (id int);\nINSERT INTO fabio_half VALUES (1);\nSELECT nope FROM nowhere;\n",
    )
    .unwrap();
    let scratch = Scratch::new("fail").await;

    let error = restore(&scratch.target(), &path, quiet(), never()).await.unwrap_err().to_string();
    assert!(error.contains(r#"relation "nowhere" does not exist"#), "{error}");
    assert!(error.contains(":3:"), "names the line: {error}");
    let db = Db::open(&scratch.target()).await.unwrap();
    let left = db.query("select count(*) from pg_class where relname = 'fabio_half'").await.unwrap();
    assert_eq!(left.rows[0][0].as_deref(), Some("0"), "rolled back as one transaction");
    drop(db);
    scratch.drop().await;
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn postgres_clean_dump_restores_over_an_existing_copy() {
    let path = temp("clean.sql");
    let options = DumpOptions { clean: true, ..public_only() };
    dump(&postgres_target(), &options, &path, quiet(), never()).await.unwrap();
    let scratch = Scratch::new("clean").await;
    restore(&scratch.target(), &path, quiet(), never()).await.unwrap();
    restore(&scratch.target(), &path, quiet(), never()).await.unwrap();
    assert_eq!(count(&scratch.target(), "genre").await, "25");
    scratch.drop().await;
    std::fs::remove_file(&path).unwrap();
}

#[tokio::test]
async fn cancelled_dump_leaves_no_file() {
    let path = temp("cancelled.sql");
    // Everything, perf.big included: long enough to be stopped part way.
    let error = dump(&postgres_target(), &DumpOptions::default(), &path, quiet(), async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await
    })
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "Cancelled.");
    assert!(!path.exists());
}

#[tokio::test]
async fn postgres_dump_error_is_passed_through() {
    let Target::Postgres(mut pg) = postgres_target() else { unreachable!() };
    pg.database = "fabio_no_such_database".into();
    let path = temp("missing.sql");
    let error =
        dump(&Target::Postgres(pg), &DumpOptions::default(), &path, quiet(), never()).await.unwrap_err().to_string();
    assert!(error.contains(r#"database "fabio_no_such_database" does not exist"#), "{error}");
    assert!(!path.exists());
}

fn sqlite_copy(name: &str) -> PathBuf {
    let path = temp(name);
    let Target::Sqlite { path: chinook } = sqlite_target() else { unreachable!() };
    std::fs::copy(chinook, &path).unwrap();
    path
}

fn empty_sqlite(name: &str) -> PathBuf {
    let path = temp(name);
    std::fs::write(&path, b"").unwrap(); // a zero-byte file is an empty database
    path
}

#[tokio::test]
async fn sqlite_sql_dump_restores_into_an_empty_file() {
    let path = temp("lite.sql");
    let done = dump(&sqlite_target(), &DumpOptions::default(), &path, quiet(), never()).await.unwrap();
    assert_eq!(done.tool, None);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCREATE TABLE [Album]"), "{}", &text[..80]);
    assert!(text.contains("INSERT INTO \"Genre\" (\"GenreId\", \"Name\") VALUES(1,'Rock');"));
    assert!(text.trim_end().ends_with("COMMIT;"));

    let into = empty_sqlite("restored.sqlite");
    let target = Target::Sqlite { path: into.clone() };
    restore(&target, &path, quiet(), never()).await.unwrap();
    for (table, rows) in [("Track", "3503"), ("InvoiceLine", "2240"), ("Genre", "25")] {
        assert_eq!(count(&target, table).await, rows, "{table}");
    }
    let db = Db::open(&target).await.unwrap();
    let indexes =
        db.query("select count(*) from sqlite_schema where type = 'index' and sql is not null").await.unwrap();
    assert_ne!(indexes.rows[0][0].as_deref(), Some("0"), "indexes come along");
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&into).unwrap();
}

/// id, r, s, b, n, twice
type Values = (i64, f64, String, Vec<u8>, Option<i64>, i64);

#[tokio::test]
async fn sqlite_dump_keeps_every_kind_of_value() {
    let source = empty_sqlite("values.sqlite");
    {
        let conn = rusqlite::Connection::open(&source).unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, r REAL, s TEXT, b BLOB, n,
                             twice INTEGER GENERATED ALWAYS AS (id * 2));
             INSERT INTO t (r, s, b, n) VALUES (0.1, 'it''s', x'00ff', NULL), (1e300, '', x'', 7);
             CREATE VIEW v AS SELECT id FROM t;
             CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 1; END;",
        )
        .unwrap();
    }
    let path = temp("values.sql");
    dump(&Target::Sqlite { path: source.clone() }, &DumpOptions::default(), &path, quiet(), never()).await.unwrap();

    let into = empty_sqlite("values-restored.sqlite");
    restore(&Target::Sqlite { path: into.clone() }, &path, quiet(), never()).await.unwrap();
    let conn = rusqlite::Connection::open(&into).unwrap();
    let rows: Vec<Values> = conn
        .prepare("SELECT id, r, s, b, n, twice FROM t ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(1, 0.1, "it's".into(), vec![0, 255], None, 2), (2, 1e300, String::new(), vec![], Some(7), 4)]
    );
    let seq: i64 = conn.query_row("SELECT seq FROM sqlite_sequence WHERE name = 't'", [], |r| r.get(0)).unwrap();
    assert_eq!(seq, 2, "AUTOINCREMENT carries on where it was");
    let objects: i64 =
        conn.query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('v', 'tr')", [], |r| r.get(0)).unwrap();
    assert_eq!(objects, 2);
    for p in [source, path, into] {
        std::fs::remove_file(p).unwrap();
    }
}

#[tokio::test]
async fn sqlite_file_copy_and_restore_from_it() {
    let copy = temp("copy.sqlite");
    let options = DumpOptions { format: DumpFormat::Archive, ..DumpOptions::default() };
    dump(&sqlite_target(), &options, &copy, quiet(), never()).await.unwrap();
    assert_eq!(count(&Target::Sqlite { path: copy.clone() }, "Track").await, "3503");

    // Restoring a database file replaces what's there.
    let into = empty_sqlite("replaced.sqlite");
    {
        rusqlite::Connection::open(&into).unwrap().execute_batch("CREATE TABLE old (x)").unwrap();
    }
    let target = Target::Sqlite { path: into.clone() };
    restore(&target, &copy, quiet(), never()).await.unwrap();
    assert_eq!(count(&target, "Album").await, "347");
    let db = Db::open(&target).await.unwrap();
    assert!(db.query("select * from old").await.is_err(), "the old table is gone");
    drop(db);
    std::fs::remove_file(&copy).unwrap();
    std::fs::remove_file(&into).unwrap();
}

#[tokio::test]
async fn sqlite_failed_restore_changes_nothing() {
    let into = sqlite_copy("fail.sqlite");
    let path = temp("bad.sql");
    std::fs::write(&path, "BEGIN TRANSACTION;\nDELETE FROM Genre;\nINSERT INTO Nowhere VALUES (1);\nCOMMIT;\n")
        .unwrap();
    let target = Target::Sqlite { path: into.clone() };

    let error = restore(&target, &path, quiet(), never()).await.unwrap_err().to_string();
    assert!(error.contains("no such table: Nowhere"), "{error}");
    assert!(error.contains("line 3"), "{error}");
    assert_eq!(count(&target, "Genre").await, "25");
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_file(&into).unwrap();
}

#[tokio::test]
async fn sqlite_copy_needs_everything() {
    let options = DumpOptions { format: DumpFormat::Archive, content: DumpContent::Schema, ..DumpOptions::default() };
    assert!(dump(&sqlite_target(), &options, &temp("nope.sqlite"), quiet(), never()).await.is_err());
}
