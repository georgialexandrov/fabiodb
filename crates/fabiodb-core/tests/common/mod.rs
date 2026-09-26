//! Chinook in both engines. Postgres needs `dev/pg.sh start`.
#![allow(dead_code)]

use fabiodb_core::{Db, PgTarget, Target};

pub fn postgres_target() -> Target {
    let url = std::env::var("FABIO_TEST_PG_URL").unwrap_or_else(|_| "postgres://fabio@localhost:54329/chinook".into());
    Target::Postgres(PgTarget::from_url(&url).unwrap())
}

pub fn sqlite_target() -> Target {
    Target::Sqlite { path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/data/Chinook_Sqlite.sqlite").into() }
}

pub async fn postgres() -> Db {
    Db::open(&postgres_target()).await.unwrap()
}

pub async fn sqlite() -> Db {
    Db::open(&sqlite_target()).await.unwrap()
}
