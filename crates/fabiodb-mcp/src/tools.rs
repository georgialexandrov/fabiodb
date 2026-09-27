//! The tools, their schemas, and how their results read to an agent: JSON for
//! data, plain text for plans.

use std::fmt::Write;

use fabiodb_core::{Agent, Plan, PlanNode, RelationRef};
use serde_json::{Value, json};

pub fn list() -> Value {
    let connection = json!({"type": "string", "description": "Connection name or id, from list_connections."});
    let table =
        json!({"type": "string", "description": "Table or view name, optionally schema-qualified (\"public.track\")."});
    let sql = json!({"type": "string", "description": "One statement. SELECT, WITH, VALUES, TABLE, SHOW or EXPLAIN."});
    json!([
        {
            "name": "list_connections",
            "description": "Databases the user opened to agents: id, name, engine (postgres or sqlite).",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "schema",
            "description": "The whole schema as DBML in one call: every table with its columns (type, not null, default, comment), primary keys, indexes, foreign keys as Ref lines, and enums. The quickest way to understand a database; views and functions aren't included.",
            "inputSchema": {"type": "object", "properties": {"connection": connection}, "required": ["connection"]},
        },
        {
            "name": "list_tables",
            "description": "Tables and views in a connection, with the planner's row estimate where there is one.",
            "inputSchema": {"type": "object", "properties": {"connection": connection}, "required": ["connection"]},
        },
        {
            "name": "describe_table",
            "description": "Columns (type, nullable, default, primary key), indexes and foreign keys of a table.",
            "inputSchema": {"type": "object", "properties": {"connection": connection, "table": table}, "required": ["connection", "table"]},
        },
        {
            "name": "sample_rows",
            "description": "The first rows of a table in primary-key order, to see what the data looks like.",
            "inputSchema": {
                "type": "object",
                "properties": {"connection": connection, "table": table, "rows": {"type": "integer", "description": "How many (default 20, at most 500)."}},
                "required": ["connection", "table"],
            },
        },
        {
            "name": "query",
            "description": "Runs one read-only statement. Results are capped at 500 rows (`truncated` says when more existed) and stop after 10 s. Values come back as text; null is null.",
            "inputSchema": {"type": "object", "properties": {"connection": connection, "sql": sql}, "required": ["connection", "sql"]},
        },
        {
            "name": "explain",
            "description": "The query plan with Fabio's findings: where the time goes, full scans that filter most rows away (and which columns to index), row estimates that are far off. With analyze (the default, Postgres) the statement really runs, read-only and rolled back, so the plan has actual times and rows. SQLite shows its query plan only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "connection": connection,
                    "sql": {"type": "string", "description": "The statement to explain, without EXPLAIN."},
                    "analyze": {"type": "boolean", "description": "Run it for actual times (default true)."},
                },
                "required": ["connection", "sql"],
            },
        },
        {
            "name": "insights",
            "description": "Postgres only. Running sessions and who blocks whom, the statements with the most total time (pg_stat_statements, if installed), tables read by full scans, and unused indexes.",
            "inputSchema": {"type": "object", "properties": {"connection": connection}, "required": ["connection"]},
        },
    ])
}

pub async fn call(agent: &Agent, tool: &str, args: &Value) -> Result<String, String> {
    let text = |key: &str| args[key].as_str().ok_or_else(|| format!("missing argument: {key}"));
    let fail = |e: fabiodb_core::Error| e.to_string();
    match tool {
        "list_connections" => to_json(&agent.connections().map_err(fail)?),
        "list_tables" => {
            let mut out = String::new();
            for r in agent.tables(text("connection")?).await.map_err(fail)? {
                let kind =
                    serde_json::to_value(r.kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
                let _ = write!(out, "{}.{} ({kind}", r.schema, r.name);
                if let Some(n) = r.estimated_rows {
                    let _ = write!(out, ", ~{n} rows");
                }
                out.push_str(")\n");
            }
            Ok(out)
        }
        "describe_table" => {
            let connection = text("connection")?;
            let relation = resolve(agent, connection, text("table")?).await?;
            to_json(&agent.describe(connection, &relation).await.map_err(fail)?)
        }
        "sample_rows" => {
            let connection = text("connection")?;
            let relation = resolve(agent, connection, text("table")?).await?;
            let rows = args["rows"].as_u64().unwrap_or(20).min(u32::MAX as u64) as u32;
            let page = agent.sample(connection, &relation, rows).await.map_err(fail)?;
            let columns: Vec<_> = page.columns.iter().map(|c| &c.name).collect();
            to_json(&json!({"columns": columns, "rows": page.rows, "more": page.has_more}))
        }
        "query" => to_json(&agent.query(text("connection")?, text("sql")?).await.map_err(fail)?),
        "explain" => {
            let analyze = args["analyze"].as_bool().unwrap_or(true);
            Ok(plan_text(&agent.explain(text("connection")?, text("sql")?, analyze).await.map_err(fail)?))
        }
        "insights" => to_json(&agent.insights(text("connection")?).await.map_err(fail)?),
        "schema" => agent.schema_dbml(text("connection")?).await.map_err(fail),
        other => Err(format!("unknown tool: {other}")),
    }
}

fn to_json(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

/// "schema.table", or a bare name looked up among the connection's relations.
async fn resolve(agent: &Agent, connection: &str, table: &str) -> Result<RelationRef, String> {
    let relations = agent.tables(connection).await.map_err(|e| e.to_string())?;
    let found = |schema: Option<&str>, name: &str| {
        relations
            .iter()
            .filter(|r| r.name == name && schema.is_none_or(|s| r.schema == s))
            .map(|r| RelationRef { schema: r.schema.clone(), name: r.name.clone() })
            .collect::<Vec<_>>()
    };
    let mut matches = found(None, table);
    if matches.is_empty()
        && let Some((schema, name)) = table.split_once('.')
    {
        matches = found(Some(schema), name);
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(format!("table “{table}” not found. list_tables shows what's there.")),
        _ => Err(format!(
            "“{table}” is in several schemas: {}. Qualify it, e.g. “{}.{table}”.",
            matches.iter().map(|r| r.schema.as_str()).collect::<Vec<_>>().join(", "),
            matches[0].schema
        )),
    }
}

/// Timing, findings, then the tree, one node per line.
fn plan_text(plan: &Plan) -> String {
    let mut out = String::new();
    match (plan.analyzed, plan.planning_ms, plan.execution_ms) {
        (true, planning, Some(execution)) => {
            let _ = writeln!(
                out,
                "Execution {execution:.1} ms · planning {:.1} ms (ran read-only, rolled back)",
                planning.unwrap_or(0.0)
            );
        }
        _ => out.push_str("Estimate only, not run.\n"),
    }
    if plan.findings.is_empty() {
        out.push_str("\nFindings: none.\n");
    } else {
        out.push_str("\nFindings:\n");
        for f in &plan.findings {
            let severity =
                serde_json::to_value(f.severity).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
            let _ = writeln!(out, "- [{severity}] {}", f.message);
        }
    }
    out.push_str("\nPlan:\n");
    node_text(&plan.root, 0, &mut out);
    out
}

fn node_text(node: &PlanNode, depth: usize, out: &mut String) {
    let _ = write!(out, "{}{}", "  ".repeat(depth), node.operation);
    if let Some(target) = &node.target {
        let _ = write!(out, " on {target}");
    }
    match (node.actual_rows, node.estimated_rows) {
        (Some(actual), Some(estimate)) => {
            let _ = write!(out, " · rows {actual:.0} (estimate {:.0})", estimate * node.loops.unwrap_or(1.0));
        }
        (None, Some(estimate)) => {
            let _ = write!(out, " · ~{estimate:.0} rows");
        }
        _ => {}
    }
    if let (Some(total), Some(own)) = (node.total_ms, node.self_ms) {
        let _ = write!(out, " · {total:.1} ms ({own:.1} ms own)");
    }
    if let (Some(hit), Some(read)) = (node.shared_hit, node.shared_read)
        && hit + read > 0
    {
        let _ = write!(out, " · buffers hit {hit} read {read}");
    }
    for d in &node.details {
        let _ = write!(out, " · {}: {}", d.label, d.value);
    }
    out.push('\n');
    for child in &node.children {
        node_text(child, depth + 1, out);
    }
}
