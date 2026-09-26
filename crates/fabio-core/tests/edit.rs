//! Cell editing: UPDATEs keyed on the primary key, checked against the value
//! the user saw, all in one transaction.

mod common;

use common::{postgres, sqlite_target};
use fabio_core::{CellChange, ColumnValue, Db, RelationRef, RowUpdate, Target};

fn cv(column: &str, value: Option<&str>) -> ColumnValue {
    ColumnValue { column: column.into(), value: value.map(Into::into) }
}

fn change(column: &str, old: Option<&str>, new: Option<&str>) -> CellChange {
    CellChange { column: column.into(), old: old.map(Into::into), new: new.map(Into::into) }
}

fn unique() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A scratch table in perf, dropped when the test ends.
struct Scratch {
    db: Db,
    relation: RelationRef,
}

impl Scratch {
    async fn create(ddl: &str, rows: &str) -> Scratch {
        let name = format!("fabio_edit_{}_{}", std::process::id(), unique());
        let db = postgres().await;
        db.set_writable(true).await.unwrap();
        db.query(&format!("create table perf.{name} {ddl}")).await.unwrap();
        db.query(&format!("insert into perf.{name} values {rows}")).await.unwrap();
        Scratch { db, relation: RelationRef { schema: "perf".into(), name } }
    }

    async fn rows(&self, order: &str) -> Vec<Vec<Option<String>>> {
        let sql = format!("select * from perf.{} order by {order}", self.relation.name);
        self.db.query(&sql).await.unwrap().rows
    }

    async fn drop(self) {
        self.db.query(&format!("drop table perf.{}", self.relation.name)).await.unwrap();
    }
}

fn s(v: &str) -> Option<String> {
    Some(v.into())
}

#[tokio::test]
async fn postgres_shows_the_updates_it_will_run() {
    let t = Scratch::create("(id int primary key, name text, price numeric(6,2))", "(1, 'a', 1.50)").await;
    let updates = [RowUpdate {
        key: vec![cv("id", Some("1"))],
        changes: vec![change("name", Some("a"), Some("it's")), change("price", Some("1.50"), None)],
    }];

    let sql = t.db.update_statements(&t.relation, &updates).await.unwrap();

    let name = t.relation.name.clone();
    t.drop().await;
    assert_eq!(sql, [format!(r#"UPDATE "perf"."{name}" SET "name" = 'it''s', "price" = NULL WHERE "id" = 1;"#)]);
}

#[tokio::test]
async fn postgres_applies_updates_in_one_transaction() {
    let t = Scratch::create(
        "(id int primary key, name text, price numeric(6,2), tags text[])",
        "(1, 'a', 1.50, '{x}'), (2, 'b', 2.00, null)",
    )
    .await;
    let updates = [
        RowUpdate {
            key: vec![cv("id", Some("1"))],
            changes: vec![change("name", Some("a"), Some("A")), change("tags", Some("{x}"), Some("{x,y}"))],
        },
        RowUpdate { key: vec![cv("id", Some("2"))], changes: vec![change("price", Some("2.00"), Some("9.99"))] },
    ];

    let rows = t.db.apply_updates(&t.relation, &updates).await.unwrap();

    let after = t.rows("id").await;
    t.drop().await;
    assert_eq!(rows, 2);
    assert_eq!(after, [[s("1"), s("A"), s("1.50"), s("{x,y}")], [s("2"), s("b"), s("9.99"), None]]);
}

#[tokio::test]
async fn postgres_refuses_when_the_row_changed_meanwhile() {
    let t = Scratch::create("(id int primary key, name text)", "(1, 'a'), (2, 'b')").await;
    let updates = [
        RowUpdate { key: vec![cv("id", Some("1"))], changes: vec![change("name", Some("a"), Some("A"))] },
        // Someone else already changed row 2 from 'x' to 'b'.
        RowUpdate { key: vec![cv("id", Some("2"))], changes: vec![change("name", Some("x"), Some("B"))] },
    ];

    let err = t.db.apply_updates(&t.relation, &updates).await.unwrap_err();

    let after = t.rows("id").await;
    t.drop().await;
    assert!(err.to_string().contains("id = 2"), "{err}");
    assert!(err.to_string().contains("Nothing was saved"), "{err}");
    assert_eq!(after, [[s("1"), s("a")], [s("2"), s("b")]]);
}

#[tokio::test]
async fn postgres_composite_keys_and_bad_values() {
    let t = Scratch::create("(a int, b text, n int, primary key (a, b))", "(1, 'x', 0), (1, 'y', 0)").await;
    let ok = [RowUpdate {
        key: vec![cv("a", Some("1")), cv("b", Some("y"))],
        changes: vec![change("n", Some("0"), Some("5"))],
    }];
    let bad = [RowUpdate {
        key: vec![cv("a", Some("1")), cv("b", Some("x"))],
        changes: vec![change("n", Some("0"), Some("five"))],
    }];

    t.db.apply_updates(&t.relation, &ok).await.unwrap();
    let err = t.db.apply_updates(&t.relation, &bad).await.unwrap_err();

    let after = t.rows("b").await;
    t.drop().await;
    assert!(err.to_string().contains("invalid input syntax for type integer"), "{err}");
    assert_eq!(after, [[s("1"), s("x"), s("0")], [s("1"), s("y"), s("5")]]);
}

#[tokio::test]
async fn postgres_needs_the_whole_primary_key() {
    let t = Scratch::create("(a int, b text, n int, primary key (a, b))", "(1, 'x', 0)").await;
    let partial = [RowUpdate { key: vec![cv("a", Some("1"))], changes: vec![change("n", Some("0"), Some("1"))] }];
    let err = t.db.apply_updates(&t.relation, &partial).await.unwrap_err();
    t.drop().await;
    assert!(err.to_string().contains("primary key"), "{err}");
}

#[tokio::test]
async fn postgres_tables_without_a_primary_key_are_not_editable() {
    let t = Scratch::create("(id int, name text)", "(1, 'a')").await;
    let updates = [RowUpdate { key: vec![cv("id", Some("1"))], changes: vec![change("name", Some("a"), Some("b"))] }];
    let err = t.db.apply_updates(&t.relation, &updates).await.unwrap_err();
    t.drop().await;
    assert!(err.to_string().contains("no primary key"), "{err}");
}

#[tokio::test]
async fn sqlite_applies_and_checks_updates() {
    let Target::Sqlite { path } = sqlite_target() else { unreachable!() };
    let copy = std::env::temp_dir().join(format!("fabio-edit-{}.sqlite", std::process::id()));
    std::fs::copy(path, &copy).unwrap();
    let db = Db::open(&Target::Sqlite { path: copy.clone() }).await.unwrap();
    db.set_writable(true).await.unwrap();
    let genre = RelationRef { schema: "main".into(), name: "Genre".into() };

    let ok = [RowUpdate {
        key: vec![cv("GenreId", Some("1"))],
        changes: vec![change("Name", Some("Rock"), Some("Rock & Roll"))],
    }];
    let stale =
        [RowUpdate { key: vec![cv("GenreId", Some("2"))], changes: vec![change("Name", Some("Not Jazz"), Some("x"))] }];
    assert_eq!(db.apply_updates(&genre, &ok).await.unwrap(), 1);
    assert!(db.apply_updates(&genre, &stale).await.is_err());

    let names = db.query("select Name from Genre where GenreId in (1, 2) order by GenreId").await.unwrap().rows;
    std::fs::remove_file(copy).unwrap();
    assert_eq!(names, [[s("Rock & Roll")], [s("Jazz")]]);
}

// --- inserts and deletes ---------------------------------------------------

use fabio_core::Changes;

#[tokio::test]
async fn postgres_inserts_with_defaults_and_deletes_by_key_in_one_transaction() {
    let t =
        Scratch::create("(id serial primary key, name text not null, note text default 'n/a')", "(10, 'old')").await;
    let changes = Changes {
        updates: vec![],
        inserts: vec![vec![cv("name", Some("new"))]],
        deletes: vec![vec![cv("id", Some("10"))]],
    };

    let preview = t.db.change_statements(&t.relation, &changes).await.unwrap();
    let applied = t.db.apply_changes(&t.relation, &changes).await.unwrap();

    let after = t.rows("id").await;
    let name = t.relation.name.clone();
    t.drop().await;
    assert_eq!(
        preview,
        [
            format!(r#"DELETE FROM "perf"."{name}" WHERE "id" = 10;"#),
            format!(r#"INSERT INTO "perf"."{name}" ("name") VALUES ('new');"#),
        ]
    );
    assert_eq!(applied, 2);
    assert_eq!(after, [[s("1"), s("new"), s("n/a")]]);
}

#[tokio::test]
async fn postgres_a_row_already_gone_undoes_the_whole_save() {
    let t = Scratch::create("(id int primary key, name text)", "(1, 'a')").await;
    let changes = Changes {
        updates: vec![RowUpdate {
            key: vec![cv("id", Some("1"))],
            changes: vec![change("name", Some("a"), Some("A"))],
        }],
        inserts: vec![vec![cv("id", Some("2")), cv("name", Some("b"))]],
        deletes: vec![vec![cv("id", Some("99"))]],
    };

    let err = t.db.apply_changes(&t.relation, &changes).await.unwrap_err();

    let after = t.rows("id").await;
    t.drop().await;
    assert!(err.to_string().contains("id = 99"), "{err}");
    assert_eq!(after, [[s("1"), s("a")]]);
}

#[tokio::test]
async fn postgres_insert_with_nothing_set_uses_default_values() {
    let t = Scratch::create("(id serial primary key, created text default 'now')", "(default, default)").await;
    let changes = Changes { updates: vec![], inserts: vec![vec![]], deletes: vec![] };
    let preview = t.db.change_statements(&t.relation, &changes).await.unwrap();
    t.db.apply_changes(&t.relation, &changes).await.unwrap();
    let rows = t.rows("id").await.len();
    t.drop().await;
    assert!(preview[0].ends_with("DEFAULT VALUES;"), "{preview:?}");
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn sqlite_inserts_and_deletes() {
    let Target::Sqlite { path } = sqlite_target() else { unreachable!() };
    let copy = std::env::temp_dir().join(format!("fabio-rows-{}.sqlite", std::process::id()));
    std::fs::copy(path, &copy).unwrap();
    let db = Db::open(&Target::Sqlite { path: copy.clone() }).await.unwrap();
    db.set_writable(true).await.unwrap();
    let genre = RelationRef { schema: "main".into(), name: "Genre".into() };

    let insert = Changes {
        updates: vec![],
        inserts: vec![
            vec![cv("GenreId", Some("100")), cv("Name", Some("Yodel"))],
            vec![cv("GenreId", Some("101")), cv("Name", Some("Kazoo"))],
        ],
        deletes: vec![],
    };
    assert_eq!(db.apply_changes(&genre, &insert).await.unwrap(), 2);
    let delete = Changes { deletes: vec![vec![cv("GenreId", Some("101"))]], ..Default::default() };
    assert_eq!(db.apply_changes(&genre, &delete).await.unwrap(), 1);
    // Still referenced by tracks: the engine's own foreign-key error comes through.
    let referenced = Changes { deletes: vec![vec![cv("GenreId", Some("25"))]], ..Default::default() };
    let err = db.apply_changes(&genre, &referenced).await.unwrap_err();

    let rows =
        db.query("select GenreId, Name from Genre where GenreId in (25, 100, 101) order by 1").await.unwrap().rows;
    std::fs::remove_file(copy).unwrap();
    assert!(err.to_string().contains("FOREIGN KEY"), "{err}");
    assert_eq!(rows, [[s("25"), s("Opera")], [s("100"), s("Yodel")]]);
}
