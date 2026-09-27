//! The schema as DBML: one plain-text model a person or an agent reads in one
//! go. Fabio writes it from the database and never runs it, so it carries
//! tables, columns, keys, indexes, enums and comments, and leaves out what
//! DBML can't say (views, functions, triggers, check constraints).
//!
//! Positions for a diagram live in a separate layout file, so the model stays
//! readable and its diff stays about the schema.

use std::fmt::Write;

use crate::{Engine, Schema, SchemaTable};

/// `source` names the database the file came from (no password), so a copy
/// found later can be matched to a saved connection.
pub fn dbml(schema: &Schema, source: Option<&str>) -> String {
    let default_schema = match schema.engine {
        Engine::Postgres => "public",
        Engine::Sqlite => "main",
    };
    let name = |s: &str, t: &str| {
        if s == default_schema { ident(t) } else { format!("{}.{}", ident(s), ident(t)) }
    };
    let mut out = String::new();

    let database_type = match schema.engine {
        Engine::Postgres => "PostgreSQL",
        Engine::Sqlite => "SQLite",
    };
    let _ = writeln!(out, "Project {} {{\n  database_type: '{database_type}'", ident(&schema.database));
    if let Some(source) = source {
        let _ = writeln!(out, "  Note: {}", string(&format!("fabio: {source}")));
    }
    out.push_str("}\n");

    for e in &schema.enums {
        let _ = writeln!(out, "\nEnum {} {{", name(&e.schema, &e.name));
        for v in &e.values {
            let _ = writeln!(out, "  {}", ident(v));
        }
        out.push_str("}\n");
    }

    for t in &schema.tables {
        let _ = writeln!(out, "\nTable {} {{", name(&t.schema, &t.name));
        table_body(&mut out, t);
        out.push_str("}\n");
    }

    let mut refs = String::new();
    for t in &schema.tables {
        for fk in &t.info.foreign_keys {
            let label = fk.name.as_deref().map(|n| format!(" {}", ident(n))).unwrap_or_default();
            let _ = writeln!(
                refs,
                "Ref{label}: {}.{} > {}.{}",
                name(&t.schema, &t.name),
                columns(&fk.columns),
                name(&fk.ref_schema, &fk.ref_table),
                columns(&fk.ref_columns)
            );
        }
    }
    if !refs.is_empty() {
        out.push('\n');
        out.push_str(&refs);
    }
    out
}

fn table_body(out: &mut String, t: &SchemaTable) {
    let pk: Vec<_> = t.info.columns.iter().filter(|c| c.primary_key).collect();
    for c in &t.info.columns {
        let mut settings = Vec::new();
        if c.primary_key && pk.len() == 1 {
            settings.push("pk".to_owned());
        } else if !c.nullable {
            settings.push("not null".to_owned());
        }
        if let Some(d) = &c.default {
            settings.push(format!("default: {}", default(d)));
        }
        if let Some(note) = &c.comment {
            settings.push(format!("note: {}", string(note)));
        }
        let settings = if settings.is_empty() { String::new() } else { format!(" [{}]", settings.join(", ")) };
        let _ = writeln!(out, "  {} {}{settings}", ident(&c.name), data_type(&c.data_type));
    }

    let mut indexes = Vec::new();
    if pk.len() > 1 {
        let names: Vec<_> = pk.iter().map(|c| c.name.clone()).collect();
        indexes.push(format!("{} [pk]", columns(&names)));
    }
    for i in t.info.indexes.iter().filter(|i| !i.primary) {
        let parts: Vec<_> = i
            .columns
            .iter()
            .map(|c| {
                let bare = c.trim_matches('"');
                if t.info.columns.iter().any(|col| col.name == bare) { ident(bare) } else { format!("`{c}`") }
            })
            .collect();
        let cols = if parts.len() == 1 { parts[0].clone() } else { format!("({})", parts.join(", ")) };
        let unique = if i.unique { "unique, " } else { "" };
        indexes.push(format!("{cols} [{unique}name: {}]", string(&i.name)));
    }
    if !indexes.is_empty() {
        out.push_str("\n  indexes {\n");
        for i in indexes {
            let _ = writeln!(out, "    {i}");
        }
        out.push_str("  }\n");
    }

    if let Some(note) = &t.comment {
        let _ = writeln!(out, "\n  Note: {}", string(note));
    }
}

/// `col` or `(a, b)`, as refs and composite indexes write them.
fn columns(names: &[String]) -> String {
    match names {
        [one] => ident(one),
        many => format!("({})", many.iter().map(|n| ident(n)).collect::<Vec<_>>().join(", ")),
    }
}

fn is_plain(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn ident(s: &str) -> String {
    if is_plain(s) { s.to_owned() } else { format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")) }
}

/// `integer`, `varchar(160)` and `int[]` stay bare; `character varying(160)` is quoted.
fn data_type(t: &str) -> String {
    if t.is_empty() {
        // A SQLite column declared without a type.
        return "any".into();
    }
    let base = t.split_once('(').map_or(t, |(b, _)| b).trim_end_matches("[]");
    if is_plain(base) && !t.contains(' ') { t.to_owned() } else { format!("\"{}\"", t.replace('"', "\\\"")) }
}

/// Numbers and booleans as they are; anything else is an expression.
fn default(d: &str) -> String {
    let literal = d.parse::<f64>().is_ok() || matches!(d.to_ascii_lowercase().as_str(), "true" | "false" | "null");
    if literal { d.to_owned() } else { format!("`{}`", d.replace('`', "'")) }
}

fn string(s: &str) -> String {
    if s.contains('\n') {
        format!("'''{}'''", s.replace('\\', "\\\\").replace("'''", "\\'''"))
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, EnumType, Index, TableInfo};

    fn column(name: &str, data_type: &str) -> Column {
        Column {
            name: name.into(),
            data_type: data_type.into(),
            nullable: true,
            default: None,
            primary_key: false,
            comment: None,
            base_type: String::new(),
        }
    }

    #[test]
    fn quotes_what_dbml_would_misread() {
        let mut id = column("id", "uuid");
        id.primary_key = true;
        id.default = Some("gen_random_uuid()".into());
        let mut qty = column("Order Qty", "integer");
        qty.nullable = false;
        qty.default = Some("0".into());
        qty.comment = Some("Can't be negative".into());
        let schema = Schema {
            engine: Engine::Postgres,
            database: "shop".into(),
            tables: vec![SchemaTable {
                schema: "sales".into(),
                name: "order".into(),
                comment: Some("One per checkout.\nNever deleted.".into()),
                info: TableInfo {
                    columns: vec![id, qty, column("tags", "text[]"), column("status", "mood")],
                    indexes: vec![Index {
                        name: "order_lower_idx".into(),
                        columns: vec!["lower((\"Order Qty\")::text)".into()],
                        unique: true,
                        primary: false,
                    }],
                    foreign_keys: vec![],
                },
            }],
            enums: vec![EnumType {
                schema: "public".into(),
                name: "mood".into(),
                values: vec!["ok".into(), "not ok".into()],
            }],
        };

        assert_eq!(
            dbml(&schema, None),
            "Project shop {
  database_type: 'PostgreSQL'
}

Enum mood {
  ok
  \"not ok\"
}

Table sales.order {
  id uuid [pk, default: `gen_random_uuid()`]
  \"Order Qty\" integer [not null, default: 0, note: 'Can\\'t be negative']
  tags text[]
  status mood

  indexes {
    `lower((\"Order Qty\")::text)` [unique, name: 'order_lower_idx']
  }

  Note: '''One per checkout.
Never deleted.'''
}
"
        );
    }
}
