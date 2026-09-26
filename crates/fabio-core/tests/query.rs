//! Runs against the Chinook fixture in both engines. Postgres needs `dev/pg.sh start`.

use fabio_core::{Target, run};

fn postgres() -> Target {
    Target::Postgres {
        url: std::env::var("FABIO_TEST_PG_URL")
            .unwrap_or_else(|_| "postgres://fabio@localhost:54329/chinook".into()),
    }
}

fn sqlite() -> Target {
    Target::Sqlite {
        path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../dev/data/Chinook_Sqlite.sqlite").into(),
    }
}

const FIRST_ARTISTS: &str =
    "select artist_id, name from artist order by artist_id limit 3";
const FIRST_ARTISTS_SQLITE: &str =
    "select ArtistId, Name from Artist order by ArtistId limit 3";

#[tokio::test]
async fn postgres_returns_columns_and_rows_as_text() {
    let result = run(&postgres(), FIRST_ARTISTS).await.unwrap();

    assert_eq!(result.columns, ["artist_id", "name"]);
    assert_eq!(result.rows[0], [Some("1".into()), Some("AC/DC".into())]);
    assert_eq!(result.rows.len(), 3);
}

#[tokio::test]
async fn sqlite_returns_the_same_data_as_postgres() {
    let pg = run(&postgres(), FIRST_ARTISTS).await.unwrap();
    let lite = run(&sqlite(), FIRST_ARTISTS_SQLITE).await.unwrap();

    assert_eq!(lite.columns, ["ArtistId", "Name"]);
    assert_eq!(lite.rows, pg.rows);
}

#[tokio::test]
async fn null_is_distinct_from_empty_string() {
    let pg = run(&postgres(), "select null::text as a, ''::text as b").await.unwrap();
    let lite = run(&sqlite(), "select null as a, '' as b").await.unwrap();

    for result in [pg, lite] {
        assert_eq!(result.rows[0], [None, Some(String::new())]);
    }
}

#[tokio::test]
async fn sqlite_is_opened_read_only() {
    let err = run(&sqlite(), "delete from Artist").await.unwrap_err();

    assert!(err.to_string().contains("readonly"), "{err}");
}

#[tokio::test]
async fn postgres_error_carries_the_server_message() {
    let err = run(&postgres(), "select * from no_such_table").await.unwrap_err();

    assert!(err.to_string().contains("no_such_table"), "{err}");
}
