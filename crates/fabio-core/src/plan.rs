//! Query plans from both engines as one tree, plus the few findings worth
//! saying out loud. Findings follow VOICE.md: one whistle per problem, with
//! the reason and the way out.

use serde::Serialize;
use serde_json::Value;

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanNode {
    /// "Hash Join", "Seq Scan", or SQLite's step text ("SCAN t").
    pub operation: String,
    /// `schema.table`, an index, a CTE — whatever the node reads.
    pub target: Option<String>,
    pub details: Vec<Detail>,
    /// Planner's estimate, per loop.
    pub estimated_rows: Option<f64>,
    /// Rows produced across all loops.
    pub actual_rows: Option<f64>,
    pub loops: Option<f64>,
    /// Inclusive time across all loops.
    pub total_ms: Option<f64>,
    /// `total_ms` minus the children's.
    pub self_ms: Option<f64>,
    pub cost: Option<f64>,
    pub shared_hit: Option<u64>,
    pub shared_read: Option<u64>,
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Where the time goes.
    Hot,
    /// Worth a look.
    Warn,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
    /// Child indexes from the root to the node this is about.
    pub path: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub root: PlanNode,
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    pub findings: Vec<Finding>,
    /// The engine's own output, for copying into other tools.
    pub raw: String,
}

// --- Postgres -------------------------------------------------------------

const DETAIL_KEYS: &[(&str, &str)] = &[
    ("Index Name", "Index"),
    ("Join Type", "Join"),
    ("Index Cond", "Index condition"),
    ("Recheck Cond", "Recheck"),
    ("Hash Cond", "Hash condition"),
    ("Merge Cond", "Merge condition"),
    ("Join Filter", "Join filter"),
    ("Filter", "Filter"),
    ("Rows Removed by Filter", "Rows removed by filter"),
    ("Rows Removed by Join Filter", "Rows removed by join filter"),
    ("Sort Key", "Sort key"),
    ("Sort Method", "Sort method"),
    ("Group Key", "Group key"),
    ("Strategy", "Strategy"),
    ("Workers Launched", "Workers"),
    ("Heap Fetches", "Heap fetches"),
];

pub fn from_postgres_json(raw: String, analyzed: bool) -> Result<Plan> {
    let json: Value = serde_json::from_str(&raw).map_err(|e| Error::Invalid(format!("unreadable plan: {e}")))?;
    let top = json.get(0).ok_or_else(|| Error::Invalid("empty plan".into()))?;
    let root = pg_node(&top["Plan"], false);
    let execution_ms = top["Execution Time"].as_f64();
    let findings = if analyzed { postgres_findings(&root, execution_ms.unwrap_or(0.0)) } else { vec![] };
    Ok(Plan { root, analyzed, planning_ms: top["Planning Time"].as_f64(), execution_ms, findings, raw })
}

/// `parallel`: under a Gather, where loops are workers running at the same time.
fn pg_node(v: &Value, parallel: bool) -> PlanNode {
    let text = |k: &str| v[k].as_str().map(str::to_owned);
    let target = match (text("Relation Name"), text("Schema")) {
        (Some(rel), Some(schema)) => Some(format!("{schema}.{rel}")),
        (Some(rel), None) => Some(rel),
        _ => text("CTE Name").or_else(|| text("Function Name")).or_else(|| text("Index Name")),
    };
    let details = DETAIL_KEYS
        .iter()
        .filter_map(|(key, label)| {
            let value = match &v[*key] {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                _ => return None,
            };
            Some(Detail { label: label.to_string(), value })
        })
        .collect();

    let operation = text("Node Type").unwrap_or_else(|| "?".into());
    let loops = v["Actual Loops"].as_f64();
    let per_loop = |key: &str| v[key].as_f64().map(|x| x * loops.unwrap_or(1.0));
    let gathers = parallel || operation.starts_with("Gather");
    let children: Vec<PlanNode> =
        v["Plans"].as_array().map(|a| a.iter().map(|c| pg_node(c, gathers)).collect()).unwrap_or_default();
    // Sequential loops add up; parallel workers overlap, so their wall time is one loop's.
    let total_ms = if parallel { v["Actual Total Time"].as_f64() } else { per_loop("Actual Total Time") };
    let self_ms = total_ms.map(|t| (t - children.iter().filter_map(|c| c.total_ms).sum::<f64>()).max(0.0));

    PlanNode {
        operation,
        target,
        details,
        estimated_rows: v["Plan Rows"].as_f64(),
        actual_rows: per_loop("Actual Rows"),
        loops,
        total_ms,
        self_ms,
        cost: v["Total Cost"].as_f64(),
        shared_hit: v["Shared Hit Blocks"].as_u64(),
        shared_read: v["Shared Read Blocks"].as_u64(),
        children,
    }
}

fn postgres_findings(root: &PlanNode, execution_ms: f64) -> Vec<Finding> {
    let mut findings = Vec::new();
    walk(root, &mut vec![], &mut |node, path| {
        let mut parts: Vec<String> = Vec::new();
        let mut severity = Severity::Warn;
        let name = match &node.target {
            Some(t) => format!("{} on {t}", node.operation),
            None => node.operation.clone(),
        };

        // Where the time goes: only worth saying when there's time to save.
        if let Some(own) = node.self_ms
            && execution_ms >= 50.0
            && own / execution_ms >= 0.6
        {
            severity = Severity::Hot;
            let share = (own / execution_ms * 100.0).min(100.0);
            parts.push(format!("{name} takes {share:.0}% of the {}", ms(execution_ms)));
        }

        // A scan that throws most rows away is an index waiting to happen.
        if node.operation.contains("Seq Scan") {
            let removed = detail(node, "Rows removed by filter").and_then(|r| r.parse::<f64>().ok());
            if let (Some(removed), Some(kept), Some(filter)) = (removed, node.actual_rows, detail(node, "Filter")) {
                let removed = removed * node.loops.unwrap_or(1.0);
                if removed >= 10_000.0 && removed >= 9.0 * kept {
                    let columns = filter_columns(filter);
                    let read = format!("reads {} rows to keep {}", count(removed + kept), count(kept));
                    let hint = if columns.is_empty() {
                        "An index on the filtered columns could help.".to_string()
                    } else {
                        format!("An index on ({}) could help.", columns.join(", "))
                    };
                    if parts.is_empty() {
                        parts.push(format!("{name} {read}. {hint}"));
                    } else {
                        parts.push(format!("it {read}. {hint}"));
                    }
                }
            }
        }

        // Big misestimates send the planner down the wrong path.
        if let (Some(est), Some(actual)) = (node.estimated_rows, node.actual_rows) {
            let per_loop = actual / node.loops.unwrap_or(1.0).max(1.0);
            let (hi, lo) = (est.max(per_loop), est.min(per_loop).max(1.0));
            if hi >= 1000.0 && hi / lo >= 10.0 {
                let table = node.target.as_deref().filter(|t| t.contains('.'));
                let fix = table.map_or_else(
                    || "Statistics may be stale.".to_string(),
                    |t| format!("Statistics may be stale — try ANALYZE {t}."),
                );
                parts.push(format!(
                    "The planner expected {} rows from {name} and got {}. {fix}",
                    count(est),
                    count(per_loop)
                ));
            }
        }

        if !parts.is_empty() {
            let mut message = parts.join(": ");
            if !message.ends_with('.') {
                message.push('.');
            }
            findings.push(Finding { severity, message, path: path.to_vec() });
        }
    });
    findings.sort_by_key(|f| f.severity != Severity::Hot);
    findings
}

// --- SQLite ---------------------------------------------------------------

/// `EXPLAIN QUERY PLAN` rows: (id, parent, detail).
pub fn from_sqlite_rows(rows: Vec<(i64, i64, String)>) -> Plan {
    fn build(parent: i64, rows: &[(i64, i64, String)]) -> Vec<PlanNode> {
        rows.iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, detail)| PlanNode {
                operation: detail.clone(),
                target: None,
                details: vec![],
                estimated_rows: None,
                actual_rows: None,
                loops: None,
                total_ms: None,
                self_ms: None,
                cost: None,
                shared_hit: None,
                shared_read: None,
                children: build(*id, rows),
            })
            .collect()
    }
    let raw = rows.iter().map(|(id, parent, d)| format!("{id}|{parent}|{d}")).collect::<Vec<_>>().join("\n");
    let mut top = build(0, &rows);
    let root = if top.len() == 1 {
        top.remove(0)
    } else {
        PlanNode {
            operation: "QUERY PLAN".into(),
            target: None,
            details: vec![],
            estimated_rows: None,
            actual_rows: None,
            loops: None,
            total_ms: None,
            self_ms: None,
            cost: None,
            shared_hit: None,
            shared_read: None,
            children: top,
        }
    };
    Plan { root, analyzed: false, planning_ms: None, execution_ms: None, findings: vec![], raw }
}

// --- helpers --------------------------------------------------------------

fn walk(node: &PlanNode, path: &mut Vec<usize>, f: &mut impl FnMut(&PlanNode, &[usize])) {
    f(node, path);
    for (i, child) in node.children.iter().enumerate() {
        path.push(i);
        walk(child, path, f);
        path.pop();
    }
}

fn detail<'a>(node: &'a PlanNode, label: &str) -> Option<&'a str> {
    node.details.iter().find(|d| d.label == label).map(|d| d.value.as_str())
}

/// Columns compared in a Postgres filter expression, e.g.
/// `((name)::text ~~ 'B%'::text) AND (bucket = 7)` → `name, bucket`.
fn filter_columns(filter: &str) -> Vec<String> {
    const OPS: &[&str] = &[" = ", " <> ", " < ", " > ", " <= ", " >= ", " ~~ ", " ~~* ", " !~~ ", " IS "];
    let mut found: Vec<(usize, String)> = Vec::new();
    for op in OPS {
        for (at, _) in filter.match_indices(op) {
            let left = filter[..at].trim_end();
            let token = left.rsplit([' ', '(']).find(|t| !t.is_empty()).unwrap_or("");
            let token = token.split("::").next().unwrap_or("").trim_matches(|c| c == '(' || c == ')' || c == '"');
            let column = token.rsplit('.').next().unwrap_or(token);
            let is_ident = !column.is_empty()
                && column.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && column.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_ident && !found.iter().any(|(_, c)| c == column) {
                found.push((at, column.to_string()));
            }
        }
    }
    // In the order they appear, which is the order an index would list them.
    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, c)| c).collect()
}

fn count(n: f64) -> String {
    let n = n.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn ms(value: f64) -> String {
    if value >= 1000.0 { format!("{:.1} s", value / 1000.0) } else { format!("{value:.0} ms") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_compared_columns_in_filters() {
        assert_eq!(filter_columns("(bucket = 7)"), ["bucket"]);
        assert_eq!(filter_columns("(((name)::text ~~ 'B%'::text) AND (t.album_id > 3))"), ["name", "album_id"]);
        assert_eq!(filter_columns("(composer IS NULL)"), ["composer"]);
    }

    #[test]
    fn counts_have_thousands_separators() {
        assert_eq!(count(4_995_000.0), "4,995,000");
        assert_eq!(count(999.0), "999");
    }
}
