use fabio_core::{AuditLog, NewAuditEntry, Source};

fn entry(connection: &str, sql: &str) -> NewAuditEntry {
    NewAuditEntry {
        connection_id: connection.into(),
        source: Source::Human,
        sql: sql.into(),
        elapsed_ms: 1.5,
        rows: Some(3),
        error: None,
    }
}

#[test]
fn records_and_returns_newest_first() {
    let log = AuditLog::open(":memory:").unwrap();
    log.record(&entry("a", "select 1")).unwrap();
    log.record(&entry("a", "select 2")).unwrap();

    let recent = log.recent(Some("a"), 10).unwrap();
    let sql: Vec<_> = recent.iter().map(|e| e.sql.as_str()).collect();
    assert_eq!(sql, ["select 2", "select 1"]);
    assert!(recent[0].at_ms > 0);
}

#[test]
fn filters_by_connection_and_limits() {
    let log = AuditLog::open(":memory:").unwrap();
    for i in 0..5 {
        log.record(&entry("a", &format!("select {i}"))).unwrap();
    }
    log.record(&entry("b", "select 'b'")).unwrap();

    assert_eq!(log.recent(Some("a"), 3).unwrap().len(), 3);
    assert_eq!(log.recent(Some("b"), 10).unwrap().len(), 1);
    assert_eq!(log.recent(None, 100).unwrap().len(), 6);
}

#[test]
fn keeps_errors_and_source() {
    let log = AuditLog::open(":memory:").unwrap();
    let mut e = entry("a", "select nme from artist");
    e.source = Source::Agent;
    e.rows = None;
    e.error = Some("column \"nme\" does not exist".into());
    log.record(&e).unwrap();

    let got = &log.recent(None, 1).unwrap()[0];
    assert_eq!(got.source, Source::Agent);
    assert_eq!(got.error.as_deref(), Some("column \"nme\" does not exist"));
    assert_eq!(got.rows, None);
}
