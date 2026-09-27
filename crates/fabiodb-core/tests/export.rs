//! Export formats, and whole-table export streamed to a file.

mod common;

use common::{postgres, sqlite};
use fabiodb_core::{ExportFormat, Filter, FilterOp, RelationRef, ResultColumn, Sort, format_rows};

fn col(name: &str, data_type: &str) -> ResultColumn {
    ResultColumn { name: name.into(), data_type: data_type.into() }
}

fn v(s: &str) -> Option<String> {
    Some(s.into())
}

fn sample() -> (Vec<ResultColumn>, Vec<Vec<Option<String>>>) {
    (
        vec![col("id", "integer"), col("name", "text"), col("ok", "boolean"), col("doc", "jsonb")],
        vec![
            vec![v("1"), v("plain"), v("t"), v(r#"{"a": 1}"#)],
            vec![v("2"), v("comma, \"quote\"\nnewline | pipe"), v("f"), None],
            vec![v("3"), v(""), None, v("[1, 2]")],
        ],
    )
}

fn table() -> RelationRef {
    RelationRef { schema: "public".into(), name: "we\"ird".into() }
}

#[test]
fn csv_quotes_only_what_needs_it_and_tells_null_from_empty() {
    let (columns, rows) = sample();
    let csv = format_rows(ExportFormat::Csv, &columns, &rows, None).unwrap();
    assert_eq!(
        csv,
        "id,name,ok,doc\r\n\
         1,plain,t,\"{\"\"a\"\": 1}\"\r\n\
         2,\"comma, \"\"quote\"\"\nnewline | pipe\",f,\r\n\
         3,\"\",,\"[1, 2]\"\r\n"
    );
}

#[test]
fn json_uses_column_types_when_known() {
    let (columns, rows) = sample();
    let json = format_rows(ExportFormat::Json, &columns, &rows, None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!([
            {"id": 1, "name": "plain", "ok": true, "doc": {"a": 1}},
            {"id": 2, "name": "comma, \"quote\"\nnewline | pipe", "ok": false, "doc": null},
            {"id": 3, "name": "", "ok": null, "doc": [1, 2]},
        ])
    );
}

#[test]
fn json_keeps_text_when_types_are_unknown_or_values_dont_fit() {
    // Query results have no types; numeric beyond f64 stays exact as text.
    let columns = vec![col("n", ""), col("big", "numeric")];
    let rows = vec![vec![v("1"), v("123456789012345678901234567890.5")]];
    let json = format_rows(ExportFormat::Json, &columns, &rows, None).unwrap();
    assert_eq!(json, r#"[{"n":"1","big":"123456789012345678901234567890.5"}]"#);
}

#[test]
fn markdown_escapes_pipes_and_line_breaks() {
    let (columns, rows) = sample();
    let md = format_rows(
        ExportFormat::Markdown,
        &columns[..2],
        &rows.iter().map(|r| r[..2].to_vec()).collect::<Vec<_>>(),
        None,
    )
    .unwrap();
    assert_eq!(
        md,
        "| id | name |\n\
         | --: | --- |\n\
         | 1 | plain |\n\
         | 2 | comma, \"quote\"<br>newline \\| pipe |\n\
         | 3 |  |\n"
    );
}

#[test]
fn insert_quotes_identifiers_and_literals() {
    let (columns, rows) = sample();
    let sql = format_rows(ExportFormat::Insert, &columns, &rows[1..2], Some(&table())).unwrap();
    assert_eq!(
        sql,
        "INSERT INTO \"public\".\"we\"\"ird\" (\"id\", \"name\", \"ok\", \"doc\") VALUES \
         (2, 'comma, \"quote\"\nnewline | pipe', false, NULL);\n"
    );
}

#[test]
fn insert_needs_a_table() {
    let (columns, rows) = sample();
    assert!(format_rows(ExportFormat::Insert, &columns, &rows, None).is_err());
}

fn temp(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("fabio-export-{}-{name}", std::process::id()))
}

#[tokio::test]
async fn postgres_exports_a_whole_table_with_filters_and_sort() {
    let path = temp("pg.csv");
    let relation = RelationRef { schema: "public".into(), name: "track".into() };
    let filters = vec![Filter { column: "album_id".into(), op: FilterOp::Eq, value: v("1") }];
    let sort = Some(Sort { column: "name".into(), descending: false });

    let rows =
        postgres().await.export_table(&relation, sort.as_ref(), &filters, ExportFormat::Csv, &path).await.unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(rows, 10);
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 11);
    assert!(lines[0].starts_with("track_id,name,album_id"), "{}", lines[0]);
    assert!(lines[1].contains("Breaking The Rules"), "{}", lines[1]);
}

#[tokio::test]
async fn postgres_export_is_not_capped() {
    let path = temp("big.json");
    let relation = RelationRef { schema: "public".into(), name: "invoice_line".into() };

    let rows = postgres().await.export_table(&relation, None, &[], ExportFormat::Json, &path).await.unwrap();

    let parsed: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(rows, 2240);
    assert_eq!(parsed.as_array().unwrap().len(), 2240);
    assert!(parsed[0]["unit_price"].is_number());
}

#[tokio::test]
async fn sqlite_exports_insert_statements() {
    let path = temp("genre.sql");
    let relation = RelationRef { schema: "main".into(), name: "Genre".into() };

    let rows = sqlite().await.export_table(&relation, None, &[], ExportFormat::Insert, &path).await.unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(rows, 25);
    assert!(text.starts_with("INSERT INTO \"main\".\"Genre\" (\"GenreId\", \"Name\") VALUES (1, 'Rock');\n"), "{text}");
}

#[tokio::test]
async fn failed_export_leaves_no_file() {
    let path = temp("missing.csv");
    let relation = RelationRef { schema: "public".into(), name: "no_such_table".into() };
    assert!(postgres().await.export_table(&relation, None, &[], ExportFormat::Csv, &path).await.is_err());
    assert!(!path.exists());
}

#[test]
fn json_keeps_column_order_and_repeated_names() {
    let columns = vec![col("z", ""), col("id", ""), col("id", "")];
    let json = format_rows(ExportFormat::Json, &columns, &[vec![v("1"), v("2"), v("3")]], None).unwrap();
    assert_eq!(json, r#"[{"z":"1","id":"2","id":"3"}]"#);
}

#[test]
fn tsv_is_for_pasting_into_spreadsheets() {
    let (columns, rows) = sample();
    let tsv =
        format_rows(ExportFormat::Tsv, &columns[..2], &rows.iter().map(|r| r[..2].to_vec()).collect::<Vec<_>>(), None)
            .unwrap();
    assert_eq!(tsv, "id\tname\n1\tplain\n2\t\"comma, \"\"quote\"\"\nnewline | pipe\"\n3\t\n");
}

#[test]
fn spreadsheet_exports_neutralize_formulas_but_keep_numbers() {
    let columns = vec![col("=header", "text"), col("amount", "numeric")];
    let rows = vec![vec![v("=HYPERLINK(\"https://evil.invalid\")"), v("-12.5")]];
    let csv = format_rows(ExportFormat::Csv, &columns, &rows, None).unwrap();
    assert_eq!(csv, "'=header,amount\r\n\"'=HYPERLINK(\"\"https://evil.invalid\"\")\",-12.5\r\n");
}
