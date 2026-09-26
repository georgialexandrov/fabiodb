//! Phase 3: query plans, findings, and Postgres statistics.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{postgres, sqlite};

// --- plans ----------------------------------------------------------------

#[tokio::test]
async fn postgres_explain_analyze_returns_a_timed_tree() {
    let plan = postgres().await.explain("select * from track where album_id = 1", true).await.unwrap();

    assert!(plan.analyzed);
    assert!(plan.execution_ms.unwrap() > 0.0);
    let scan = find(&plan.root, |n| n.target.as_deref() == Some("public.track")).expect("scan on track");
    assert_eq!(scan.actual_rows, Some(10.0));
    assert!(scan.estimated_rows.is_some());
    assert!(scan.total_ms.is_some() && scan.self_ms.is_some());
}

#[tokio::test]
async fn postgres_explain_without_analyze_has_no_timings() {
    let plan = postgres().await.explain("select * from track", false).await.unwrap();

    assert!(!plan.analyzed);
    assert_eq!(plan.root.operation, "Seq Scan");
    assert_eq!(plan.root.actual_rows, None);
    assert!(plan.root.estimated_rows.unwrap() > 3000.0);
}

#[tokio::test]
async fn self_time_excludes_children() {
    let plan = postgres()
        .await
        .explain("select a.name, count(*) from artist a join album al using (artist_id) group by 1", true)
        .await
        .unwrap();

    fn check(n: &fabio_core::PlanNode) {
        let children: f64 = n.children.iter().filter_map(|c| c.total_ms).sum();
        if let (Some(total), Some(own)) = (n.total_ms, n.self_ms) {
            assert!(own >= 0.0 && (own + children - total).abs() < 0.01 || own == 0.0, "{}: {own} + {children} vs {total}", n.operation);
        }
        n.children.iter().for_each(check);
    }
    check(&plan.root);
}

#[tokio::test]
async fn explain_analyze_of_a_write_is_rolled_back() {
    let table = format!("fabio_explain_{}", std::process::id());
    let db = postgres().await;
    db.set_writable(true).await.unwrap();
    db.query(&format!("create table {table} (x int)")).await.unwrap();

    db.explain(&format!("insert into {table} values (1)"), true).await.unwrap();
    let rows = db.query(&format!("select count(*) from {table}")).await.unwrap().rows;
    db.query(&format!("drop table {table}")).await.unwrap();

    assert_eq!(rows, [[Some("0".into())]]);
}

#[tokio::test]
async fn read_only_session_refuses_to_analyze_a_write() {
    let err = postgres().await.explain("insert into artist (artist_id, name) values (9999, 'x')", true).await.unwrap_err();
    assert!(err.to_string().contains("read-only"), "{err}");
}

#[tokio::test]
async fn failed_explain_leaves_the_session_usable() {
    let db = postgres().await;
    assert!(db.explain("select nme from artist", true).await.is_err());
    assert_eq!(db.query("select 1").await.unwrap().rows, [[Some("1".into())]]);
}

#[tokio::test]
async fn sqlite_query_plan_is_a_tree_of_steps() {
    let plan = sqlite()
        .await
        .explain("select * from Track t join Album a on a.AlbumId = t.AlbumId where a.Title like 'B%'", false)
        .await
        .unwrap();

    assert!(!plan.analyzed);
    let ops = all(&plan.root).into_iter().map(|n| n.operation.clone()).collect::<Vec<_>>().join(" | ");
    assert!(ops.contains("SCAN") && ops.contains("SEARCH"), "{ops}");
}

// --- findings -------------------------------------------------------------

#[tokio::test]
async fn filtered_seq_scan_on_a_big_table_suggests_an_index() {
    let plan = postgres().await.explain("select * from perf.big where bucket = 7", true).await.unwrap();

    let finding = plan.findings.iter().find(|f| f.message.contains("perf.big")).expect("a finding about perf.big");
    assert!(finding.message.contains("bucket"), "{}", finding.message);
    assert!(finding.message.contains("index"), "{}", finding.message);
    // Parallel workers overlap; their time must not add up past 100%.
    assert!(!finding.message.contains("% of") || percent(&finding.message) <= 100, "{}", finding.message);
}

fn percent(message: &str) -> u32 {
    let at = message.find('%').unwrap();
    message[..at].rsplit(' ').next().unwrap().parse().unwrap()
}

#[tokio::test]
async fn cheap_plans_have_nothing_to_say() {
    let plan = postgres().await.explain("select * from artist where artist_id = 1", true).await.unwrap();
    assert!(plan.findings.is_empty(), "{:?}", plan.findings);
}

// --- insights -------------------------------------------------------------

#[tokio::test]
async fn insights_show_other_sessions_that_are_running() {
    let busy = Arc::new(postgres().await);
    let sleeping = tokio::spawn({
        let busy = busy.clone();
        async move { busy.query("select pg_sleep(1.5)").await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    let insights = postgres().await.insights().await.unwrap();
    sleeping.await.unwrap().unwrap();

    let activity = insights.activity.iter().find(|a| a.query.contains("pg_sleep(1.5)")).expect("the sleeping session");
    assert_eq!(activity.state.as_deref(), Some("active"));
    assert!(activity.running_ms.unwrap() > 100.0);
}

#[tokio::test]
async fn insights_list_top_statements_from_pg_stat_statements() {
    let db = postgres().await;
    db.query("select count(*) from invoice_line").await.unwrap();

    let top = db.insights().await.unwrap().top_statements.expect("pg_stat_statements is installed in dev");
    assert!(!top.is_empty());
    assert!(top.windows(2).all(|w| w[0].total_ms >= w[1].total_ms));
}

#[tokio::test]
async fn unused_indexes_exclude_primary_and_unique() {
    let insights = postgres().await.insights().await.unwrap();
    assert!(insights.unused_indexes.iter().all(|i| i.scans == 0 && !i.name.ends_with("_pkey")));
}

#[tokio::test]
async fn sqlite_has_no_insights() {
    let err = sqlite().await.insights().await.unwrap_err();
    assert!(err.to_string().contains("SQLite"), "{err}");
}

// --- helpers --------------------------------------------------------------

fn all(n: &fabio_core::PlanNode) -> Vec<&fabio_core::PlanNode> {
    let mut out = vec![n];
    for c in &n.children {
        out.extend(all(c));
    }
    out
}

fn find(n: &fabio_core::PlanNode, pred: impl Fn(&fabio_core::PlanNode) -> bool + Copy) -> Option<&fabio_core::PlanNode> {
    all(n).into_iter().find(|n| pred(n))
}
