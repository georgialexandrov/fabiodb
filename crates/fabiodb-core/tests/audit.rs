use fabiodb_core::{AuditLog, NewAuditEntry, Source};

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

#[test]
fn agent_entries_after_an_id_newest_first() {
    let log = AuditLog::open(":memory:").unwrap();
    let mut agent = entry("a", "select 1");
    agent.source = Source::Agent;
    let first = log.record(&agent).unwrap();
    log.record(&entry("a", "human")).unwrap();
    agent.sql = "select 2".into();
    log.record(&agent).unwrap();

    let all: Vec<_> = log.agent_since(0, 10).unwrap().into_iter().map(|e| e.sql).collect();
    assert_eq!(all, ["select 2", "select 1"]);
    let newer: Vec<_> = log.agent_since(first, 10).unwrap().into_iter().map(|e| e.sql).collect();
    assert_eq!(newer, ["select 2"]);
}

#[test]
fn a_second_log_on_the_same_file_sees_new_entries() {
    // The MCP server writes, the app reads: two connections, one file.
    let path = std::env::temp_dir().join(format!("fabio-audit-{}.sqlite", std::process::id()));
    let reader = AuditLog::open(&path).unwrap();
    let writer = AuditLog::open(&path).unwrap();
    assert!(reader.agent_since(0, 10).unwrap().is_empty());

    let mut e = entry("a", "select 1");
    e.source = Source::Agent;
    writer.record(&e).unwrap();

    assert_eq!(reader.agent_since(0, 10).unwrap().len(), 1);
    drop((reader, writer));
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}
