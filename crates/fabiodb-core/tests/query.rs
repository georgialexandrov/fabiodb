mod common;

use common::{postgres, sqlite};

const FIRST_ARTISTS: &str = "select artist_id, name from artist order by artist_id limit 3";
const FIRST_ARTISTS_SQLITE: &str = "select ArtistId, Name from Artist order by ArtistId limit 3";

#[tokio::test]
async fn postgres_returns_columns_and_rows_as_text() {
    let result = postgres().await.query(FIRST_ARTISTS).await.unwrap();

    assert_eq!(result.columns, ["artist_id", "name"]);
    assert_eq!(result.rows[0], [Some("1".into()), Some("AC/DC".into())]);
    assert_eq!(result.rows.len(), 3);
}

#[tokio::test]
async fn sqlite_returns_the_same_data_as_postgres() {
    let pg = postgres().await.query(FIRST_ARTISTS).await.unwrap();
    let lite = sqlite().await.query(FIRST_ARTISTS_SQLITE).await.unwrap();

    assert_eq!(lite.columns, ["ArtistId", "Name"]);
    assert_eq!(lite.rows, pg.rows);
}

#[tokio::test]
async fn null_is_distinct_from_empty_string() {
    let pg = postgres().await.query("select null::text as a, ''::text as b").await.unwrap();
    let lite = sqlite().await.query("select null as a, '' as b").await.unwrap();

    for result in [pg, lite] {
        assert_eq!(result.rows[0], [None, Some(String::new())]);
    }
}

#[tokio::test]
async fn sqlite_is_opened_read_only() {
    let err = sqlite().await.query("delete from Artist").await.unwrap_err();

    assert!(err.to_string().contains("readonly"), "{err}");
}

#[tokio::test]
async fn postgres_error_carries_the_server_message() {
    let err = postgres().await.query("select * from no_such_table").await.unwrap_err();

    assert!(err.to_string().contains("no_such_table"), "{err}");
}

#[tokio::test]
async fn one_connection_serves_many_queries() {
    let db = postgres().await;
    for _ in 0..3 {
        assert_eq!(db.query("select 1").await.unwrap().rows, [[Some("1".into())]]);
    }
}

#[tokio::test]
async fn postgres_is_read_only_by_default() {
    let err = postgres().await.query("create table fabio_probe (id int)").await.unwrap_err();

    assert!(err.to_string().contains("read-only"), "{err}");
}
