//! Rows out of Fabio: CSV, JSON, Markdown and INSERT statements, written as
//! they arrive so a whole table can stream to a file.
//!
//! Values are the engine's text. Where the column type is known, JSON and
//! INSERT use it (numbers, booleans, embedded JSON); where it isn't (query
//! results), values stay text — never a guess.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::sql::quote;
use crate::{Error, RelationRef, Result, ResultColumn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
    Markdown,
    /// One `INSERT` per row; needs the table name.
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Bool,
    Json,
    Text,
}

fn kind(data_type: &str) -> Kind {
    let t = data_type.to_ascii_lowercase();
    const NUMBERS: &[&str] = &[
        "smallint",
        "integer",
        "bigint",
        "int",
        "numeric",
        "decimal",
        "real",
        "double",
        "float",
        "tinyint",
        "mediumint",
    ];
    if NUMBERS.iter().any(|n| t.starts_with(n)) {
        Kind::Number
    } else if t == "boolean" || t == "bool" {
        Kind::Bool
    } else if t == "json" || t == "jsonb" {
        Kind::Json
    } else {
        Kind::Text
    }
}

/// Writes one row at a time; `finish` closes the document.
pub struct RowWriter<W: Write> {
    format: ExportFormat,
    columns: Vec<(String, Kind)>,
    /// `INSERT INTO "schema"."table" ("a", "b") VALUES ` for Insert.
    insert_prefix: String,
    out: W,
    rows: u64,
}

impl<W: Write> RowWriter<W> {
    pub fn new(
        format: ExportFormat,
        columns: &[ResultColumn],
        table: Option<&RelationRef>,
        mut out: W,
    ) -> Result<Self> {
        let cols: Vec<_> = columns.iter().map(|c| (c.name.clone(), kind(&c.data_type))).collect();
        let mut insert_prefix = String::new();
        match format {
            ExportFormat::Csv => {
                let header: Vec<_> = cols.iter().map(|(n, _)| csv_field(Some(n))).collect();
                write!(out, "{}\r\n", header.join(","))?;
            }
            ExportFormat::Json => out.write_all(b"[")?,
            ExportFormat::Markdown => {
                let names: Vec<_> = cols.iter().map(|(n, _)| markdown_cell(n)).collect();
                let align: Vec<_> = cols.iter().map(|(_, k)| if *k == Kind::Number { "--:" } else { "---" }).collect();
                write!(out, "| {} |\n| {} |\n", names.join(" | "), align.join(" | "))?;
            }
            ExportFormat::Insert => {
                let table = table.ok_or_else(|| Error::Invalid("INSERT statements need a table name.".into()))?;
                let names: Vec<_> = cols.iter().map(|(n, _)| quote(n)).collect();
                insert_prefix = format!(
                    "INSERT INTO {}.{} ({}) VALUES ",
                    quote(&table.schema),
                    quote(&table.name),
                    names.join(", ")
                );
            }
        }
        Ok(RowWriter { format, columns: cols, insert_prefix, out, rows: 0 })
    }

    pub fn row(&mut self, row: &[Option<String>]) -> Result<()> {
        let values = self.columns.iter().zip(row);
        match self.format {
            ExportFormat::Csv => {
                let fields: Vec<_> = values.map(|(_, v)| csv_field(v.as_deref())).collect();
                write!(self.out, "{}\r\n", fields.join(","))?;
            }
            ExportFormat::Json => {
                // Written by hand: a map would sort the keys and drop a
                // repeated column name (`select a.id, b.id …`).
                let fields: Vec<_> = values
                    .map(|((name, kind), v)| {
                        format!("{}:{}", serde_json::Value::String(name.clone()), json_value(*kind, v.as_deref()))
                    })
                    .collect();
                if self.rows > 0 {
                    self.out.write_all(b",")?;
                }
                write!(self.out, "{{{}}}", fields.join(","))?;
            }
            ExportFormat::Markdown => {
                let cells: Vec<_> = values.map(|(_, v)| v.as_deref().map_or_else(String::new, markdown_cell)).collect();
                writeln!(self.out, "| {} |", cells.join(" | "))?;
            }
            ExportFormat::Insert => {
                let literals: Vec<_> = values.map(|((_, kind), v)| sql_literal(*kind, v.as_deref())).collect();
                writeln!(self.out, "{}({});", self.insert_prefix, literals.join(", "))?;
            }
        }
        self.rows += 1;
        Ok(())
    }

    /// The writer back, and how many rows went through it.
    pub fn finish(mut self) -> Result<(W, u64)> {
        if self.format == ExportFormat::Json {
            self.out.write_all(b"]")?;
        }
        self.out.flush()?;
        Ok((self.out, self.rows))
    }
}

/// For the clipboard: the rows already on screen, formatted.
pub fn format_rows(
    format: ExportFormat,
    columns: &[ResultColumn],
    rows: &[Vec<Option<String>>],
    table: Option<&RelationRef>,
) -> Result<String> {
    let mut writer = RowWriter::new(format, columns, table, Vec::new())?;
    for row in rows {
        writer.row(row)?;
    }
    let (bytes, _) = writer.finish()?;
    String::from_utf8(bytes).map_err(|e| Error::Invalid(e.to_string()))
}

/// A buffered file for an export.
pub(crate) fn create(path: &Path) -> Result<std::io::BufWriter<std::fs::File>> {
    Ok(std::io::BufWriter::with_capacity(1 << 16, std::fs::File::create(path)?))
}

/// A failed export leaves no half-written file behind.
pub(crate) fn remove_on_error<T>(path: &Path, result: Result<T>) -> Result<T> {
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

/// RFC 4180. NULL is an empty field; an empty string is `""`, so the two stay apart.
fn csv_field(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some("") => "\"\"".into(),
        Some(v) if v.contains([',', '"', '\n', '\r']) => format!("\"{}\"", v.replace('"', "\"\"")),
        Some(v) => v.to_owned(),
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>")
}

fn json_value(kind: Kind, value: Option<&str>) -> serde_json::Value {
    use serde_json::Value;
    let Some(v) = value else { return Value::Null };
    match kind {
        Kind::Number => exact_number(v).unwrap_or_else(|| Value::String(v.into())),
        Kind::Bool => match v {
            "t" | "true" | "1" => Value::Bool(true),
            "f" | "false" | "0" => Value::Bool(false),
            _ => Value::String(v.into()),
        },
        Kind::Json => serde_json::from_str(v).unwrap_or_else(|_| Value::String(v.into())),
        Kind::Text => Value::String(v.into()),
    }
}

/// A JSON number only when it reads back the same: integers that fit in 64
/// bits, and decimals with at most 15 significant digits (what an f64 holds).
fn exact_number(v: &str) -> Option<serde_json::Value> {
    if let Ok(i) = v.parse::<i64>() {
        return Some(i.into());
    }
    if let Ok(u) = v.parse::<u64>() {
        return Some(u.into());
    }
    let digits = v.chars().take_while(|c| *c != 'e' && *c != 'E').filter(char::is_ascii_digit).count();
    let f: f64 = v.parse().ok()?;
    if digits <= 15 && v.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) {
        serde_json::Number::from_f64(f).map(Into::into)
    } else {
        None
    }
}

fn sql_literal(kind: Kind, value: Option<&str>) -> String {
    let Some(v) = value else { return "NULL".into() };
    let numeric = !v.is_empty()
        && v.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c))
        && v.parse::<f64>().is_ok_and(f64::is_finite);
    match kind {
        Kind::Number if numeric => v.to_owned(),
        Kind::Bool if v == "t" || v == "true" => "true".into(),
        Kind::Bool if v == "f" || v == "false" => "false".into(),
        _ => format!("'{}'", v.replace('\'', "''")),
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Invalid(e.to_string())
    }
}
