mod common;

use common::{postgres, sqlite};
use fabio_core::{Filter, FilterOp, PageRequest, PgTarget, RelationKind, RelationRef, SslMode, Sort};

fn rel(schema: &str, name: &str) -> RelationRef {
    RelationRef { schema: schema.into(), name: name.into() }
}

fn page(relation: RelationRef) -> PageRequest {
    PageRequest { relation, sort: None, filters: vec![], offset: 0, limit: 100 }
}

fn filter(column: &str, op: FilterOp, value: &str) -> Filter {
    Filter { column: column.into(), op, value: Some(value.into()) }
}

// --- connection URL -------------------------------------------------------

#[test]
fn parses_a_postgres_url_into_fields() {
    let t = PgTarget::from_url("postgres://me:s%40cret@db.local:6543/app?sslmode=require").unwrap();

    assert_eq!(t.host, "db.local");
    assert_eq!(t.port, 6543);
    assert_eq!(t.user, "me");
    assert_eq!(t.password.as_deref(), Some("s@cret"));
    assert_eq!(t.database, "app");
    assert_eq!(t.ssl, SslMode::Require);
}

#[test]
fn url_defaults_match_libpq() {
    let t = PgTarget::from_url("postgres://me@localhost/app").unwrap();

    assert_eq!(t.port, 5432);
    assert_eq!(t.ssl, SslMode::Prefer);
    assert_eq!(t.password, None);
}

// --- relations ------------------------------------------------------------

#[tokio::test]
async fn postgres_lists_tables_with_row_estimates() {
    let relations = postgres().await.relations().await.unwrap();
    let track = relations.iter().find(|r| r.name == "track").unwrap();

    assert_eq!(track.schema, "public");
    assert_eq!(track.kind, RelationKind::Table);
    assert_eq!(track.estimated_rows, Some(3503));
    let tables = relations.iter().filter(|r| r.schema == "public" && r.kind == RelationKind::Table);
    assert_eq!(tables.count(), 11);
    assert!(relations.iter().all(|r| r.schema != "pg_catalog" && r.schema != "information_schema"));
}

#[tokio::test]
async fn postgres_unanalyzed_table_has_no_estimate() {
    let relations = postgres().await.relations().await.unwrap();
    let genre = relations.iter().find(|r| r.name == "genre").unwrap();

    assert_eq!(genre.estimated_rows, None);
}

#[tokio::test]
async fn sqlite_lists_user_tables_in_main() {
    let relations = sqlite().await.relations().await.unwrap();

    assert_eq!(relations.len(), 11);
    assert!(relations.iter().all(|r| r.schema == "main" && r.kind == RelationKind::Table));
    assert!(relations.iter().any(|r| r.name == "Track"));
    assert!(relations.iter().all(|r| !r.name.starts_with("sqlite_")));
}

// --- describe -------------------------------------------------------------

#[tokio::test]
async fn postgres_describes_columns_indexes_and_foreign_keys() {
    let info = postgres().await.describe(&rel("public", "track")).await.unwrap();

    let names: Vec<_> = info.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[..3], ["track_id", "name", "album_id"]);

    let price = info.columns.iter().find(|c| c.name == "unit_price").unwrap();
    assert_eq!(price.data_type, "numeric(10,2)");
    assert!(!price.nullable);

    let id = &info.columns[0];
    assert!(id.primary_key);
    assert!(!info.columns[1].primary_key);

    let pkey = info.indexes.iter().find(|i| i.name == "track_pkey").unwrap();
    assert!(pkey.primary && pkey.unique);
    assert_eq!(pkey.columns, ["track_id"]);
    assert_eq!(info.indexes.len(), 4);

    let album = info.foreign_keys.iter().find(|f| f.columns == ["album_id"]).unwrap();
    assert_eq!((album.ref_table.as_str(), album.ref_columns.as_slice()), ("album", &["album_id".to_string()][..]));
}

#[tokio::test]
async fn postgres_composite_primary_key() {
    let info = postgres().await.describe(&rel("public", "playlist_track")).await.unwrap();

    assert!(info.columns.iter().all(|c| c.primary_key));
    let pkey = info.indexes.iter().find(|i| i.primary).unwrap();
    assert_eq!(pkey.columns, ["playlist_id", "track_id"]);
}

#[tokio::test]
async fn sqlite_describes_columns_indexes_and_foreign_keys() {
    let info = sqlite().await.describe(&rel("main", "Track")).await.unwrap();

    assert_eq!(info.columns[0].name, "TrackId");
    assert!(info.columns[0].primary_key);
    let price = info.columns.iter().find(|c| c.name == "UnitPrice").unwrap();
    assert_eq!(price.data_type, "NUMERIC(10,2)");
    assert!(!price.nullable);

    let album_idx = info.indexes.iter().find(|i| i.name == "IFK_TrackAlbumId").unwrap();
    assert_eq!(album_idx.columns, ["AlbumId"]);
    assert!(!album_idx.unique);

    assert_eq!(info.foreign_keys.len(), 3);
    let genre = info.foreign_keys.iter().find(|f| f.columns == ["GenreId"]).unwrap();
    assert_eq!(genre.ref_table, "Genre");
}

#[tokio::test]
async fn describing_a_missing_table_is_an_error() {
    assert!(postgres().await.describe(&rel("public", "nope")).await.is_err());
    assert!(sqlite().await.describe(&rel("main", "Nope")).await.is_err());
}

// --- paging ---------------------------------------------------------------

#[tokio::test]
async fn first_page_is_ordered_by_primary_key_and_reports_more() {
    let p = postgres().await.page(&page(rel("public", "track"))).await.unwrap();

    assert_eq!(p.rows.len(), 100);
    assert!(p.has_more);
    assert_eq!(p.rows[0][0].as_deref(), Some("1"));
    assert_eq!(p.rows[99][0].as_deref(), Some("100"));
    assert_eq!(p.columns[8].name, "unit_price");
    assert_eq!(p.columns[8].data_type, "numeric(10,2)");
    assert_eq!(p.rows[0][8].as_deref(), Some("0.99"), "numeric keeps its scale");
}

#[tokio::test]
async fn last_page_has_no_more() {
    let mut req = page(rel("public", "track"));
    req.offset = 3500;
    let p = postgres().await.page(&req).await.unwrap();

    assert_eq!(p.rows.len(), 3);
    assert!(!p.has_more);
}

#[tokio::test]
async fn both_engines_page_the_same_data() {
    let mut pg_req = page(rel("public", "track"));
    pg_req.sort = Some(Sort { column: "milliseconds".into(), descending: true });
    pg_req.limit = 5;
    let mut lite_req = page(rel("main", "Track"));
    lite_req.sort = Some(Sort { column: "Milliseconds".into(), descending: true });
    lite_req.limit = 5;

    let pg = postgres().await.page(&pg_req).await.unwrap();
    let lite = sqlite().await.page(&lite_req).await.unwrap();

    let ids = |p: &fabio_core::Page| p.rows.iter().map(|r| r[0].clone()).collect::<Vec<_>>();
    assert_eq!(ids(&pg), ids(&lite));
    assert_eq!(pg.rows[0][1].as_deref(), Some("Occupation / Precipice"));
}

#[tokio::test]
async fn filters_combine_with_and() {
    let mut req = page(rel("public", "track"));
    req.filters = vec![
        filter("album_id", FilterOp::Eq, "1"),
        filter("milliseconds", FilterOp::Gt, "300000"),
    ];
    let p = postgres().await.page(&req).await.unwrap();

    // AC/DC "For Those About To Rock": one track runs past 5 minutes.
    assert_eq!(p.rows.len(), 1);
    assert!(p.rows.iter().all(|r| r[3].as_deref() == Some("1")));
}

#[tokio::test]
async fn contains_filter_is_case_insensitive_in_both_engines() {
    let mut pg_req = page(rel("public", "artist"));
    pg_req.filters = vec![filter("name", FilterOp::Contains, "zeppelin")];
    let mut lite_req = page(rel("main", "Artist"));
    lite_req.filters = vec![filter("Name", FilterOp::Contains, "zeppelin")];

    let pg = postgres().await.page(&pg_req).await.unwrap();
    let lite = sqlite().await.page(&lite_req).await.unwrap();

    let names: Vec<_> = pg.rows.iter().map(|r| r[1].as_deref().unwrap()).collect();
    assert_eq!(names, ["Led Zeppelin", "Dread Zeppelin"]);
    assert_eq!(lite.rows, pg.rows);
}

#[tokio::test]
async fn contains_treats_wildcards_literally() {
    let mut req = page(rel("public", "artist"));
    req.filters = vec![filter("name", FilterOp::Contains, "%")];

    assert!(postgres().await.page(&req).await.unwrap().rows.is_empty());
}

#[tokio::test]
async fn null_filters() {
    let mut req = page(rel("public", "track"));
    req.filters = vec![Filter { column: "composer".into(), op: FilterOp::IsNull, value: None }];
    req.limit = 10_000;
    let p = postgres().await.page(&req).await.unwrap();

    assert_eq!(p.rows.len(), 977);
    assert!(p.rows.iter().all(|r| r[5].is_none()));
}

#[tokio::test]
async fn unknown_columns_are_rejected_not_interpolated() {
    let mut req = page(rel("public", "track"));
    req.sort = Some(Sort { column: "1; drop table track; --".into(), descending: false });

    let err = postgres().await.page(&req).await.unwrap_err();
    assert!(err.to_string().contains("unknown column"), "{err}");
}

#[tokio::test]
async fn filter_values_are_parameters_not_sql() {
    let mut req = page(rel("public", "artist"));
    req.filters = vec![filter("name", FilterOp::Eq, "x' or '1'='1")];

    assert!(postgres().await.page(&req).await.unwrap().rows.is_empty());
}

#[tokio::test]
async fn bad_filter_value_reports_the_type_error() {
    let mut req = page(rel("public", "track"));
    req.filters = vec![filter("album_id", FilterOp::Eq, "abc")];

    let err = postgres().await.page(&req).await.unwrap_err();
    assert!(err.to_string().contains("integer"), "{err}");
}

#[tokio::test]
async fn page_reports_the_sql_it_ran() {
    let p = postgres().await.page(&page(rel("public", "artist"))).await.unwrap();

    assert!(p.sql.contains(r#"FROM "public"."artist""#), "{}", p.sql);
}

#[tokio::test]
async fn numeric_columns_sort_as_numbers_not_text() {
    let mut req = page(rel("public", "track"));
    req.sort = Some(Sort { column: "milliseconds".into(), descending: false });
    req.limit = 3;
    let p = postgres().await.page(&req).await.unwrap();

    let ms: Vec<i64> = p.rows.iter().map(|r| r[6].as_deref().unwrap().parse().unwrap()).collect();
    assert!(ms.is_sorted(), "{ms:?}");
    assert_eq!(ms[0], 1071);
}

// --- counting -------------------------------------------------------------

use std::time::Duration;

const PLENTY: Duration = Duration::from_secs(5);

#[tokio::test]
async fn counts_all_rows_exactly() {
    let pg = postgres().await.count(&rel("public", "track"), &[], PLENTY).await.unwrap();
    let lite = sqlite().await.count(&rel("main", "Track"), &[], PLENTY).await.unwrap();

    assert_eq!((pg.rows, pg.exact), (Some(3503), true));
    assert_eq!((lite.rows, lite.exact), (Some(3503), true));
}

#[tokio::test]
async fn count_applies_filters() {
    let filters = [Filter { column: "composer".into(), op: FilterOp::IsNull, value: None }];
    let count = postgres().await.count(&rel("public", "track"), &filters, PLENTY).await.unwrap();

    assert_eq!(count.rows, Some(977));
}

#[tokio::test]
async fn slow_postgres_count_falls_back_to_the_estimate() {
    let db = postgres().await;
    let count = db.count(&rel("perf", "big"), &[], Duration::from_millis(5)).await.unwrap();

    assert!(!count.exact);
    assert!(count.rows.is_some_and(|n| n > 4_000_000), "{count:?}");
    // The cancelled count must not poison the connection.
    assert_eq!(db.query("select 1").await.unwrap().rows, [[Some("1".into())]]);
}

#[tokio::test]
async fn slow_filtered_count_is_unknown() {
    let filters = [filter("bucket", FilterOp::Gt, "10")];
    let count = postgres().await.count(&rel("perf", "big"), &filters, Duration::from_millis(5)).await.unwrap();

    assert_eq!((count.rows, count.exact), (None, false));
}

// --- autocomplete schema --------------------------------------------------

#[tokio::test]
async fn completion_schema_lists_every_relation_with_its_columns() {
    let pg = postgres().await.completion_schema().await.unwrap();
    let track = pg.iter().find(|t| t.schema == "public" && t.name == "track").unwrap();
    assert_eq!(track.columns[..2], ["track_id", "name"]);
    assert!(pg.iter().any(|t| t.schema == "perf" && t.name == "big"));

    let lite = sqlite().await.completion_schema().await.unwrap();
    let track = lite.iter().find(|t| t.name == "Track").unwrap();
    assert_eq!(track.schema, "main");
    assert_eq!(track.columns[..2], ["TrackId", "Name"]);
    assert_eq!(lite.len(), 11);
}
