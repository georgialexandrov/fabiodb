mod common;

use common::{postgres, sqlite};
use fabiodb_core::dbml;

#[tokio::test]
async fn postgres_schema_as_dbml() {
    let schema = postgres().await.schema().await.unwrap();
    let text = dbml(&schema, Some("postgres://localhost:54329/chinook"));

    assert!(text.starts_with(
        "Project chinook {\n  database_type: 'PostgreSQL'\n  Note: 'fabio: postgres://localhost:54329/chinook'\n}\n"
    ));
    assert!(text.contains(
        "\nTable album {
  album_id integer [pk]
  title \"character varying(160)\" [not null]
  artist_id integer [not null]

  indexes {
    artist_id [name: 'album_artist_id_idx']
  }
}\n"
    ));
    // A composite primary key is a table index; the columns are only not null.
    assert!(text.contains(
        "\nTable playlist_track {
  playlist_id integer [not null]
  track_id integer [not null]

  indexes {
    (playlist_id, track_id) [pk]"
    ));
    assert!(text.contains("\nRef track_album_id_fkey: track.album_id > album.album_id\n"));
    // Other schemas are named; public isn't.
    assert!(text.contains("\nTable perf.big {\n"));
    assert_eq!(text.matches("\nRef ").count(), 11);
}

#[tokio::test]
async fn sqlite_schema_as_dbml() {
    let schema = sqlite().await.schema().await.unwrap();
    let text = dbml(&schema, None);

    assert!(text.starts_with("Project \"Chinook_Sqlite.sqlite\" {\n  database_type: 'SQLite'\n}\n"));
    assert_eq!(schema.tables.len(), 11);
    assert!(text.contains("\nTable Album {\n  AlbumId INTEGER [pk]\n  Title NVARCHAR(160) [not null]\n"));
    assert_eq!(text.matches("\nRef").count(), 11);
}
