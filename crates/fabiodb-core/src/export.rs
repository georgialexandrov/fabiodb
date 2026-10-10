//! Rows out of Fabio: CSV, JSON, Markdown, INSERT statements and ASCII tables,
//! written as they arrive so a whole table can stream to a file (an ASCII
//! table waits for its last row, which sets the column widths).
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
    /// CSV that spreadsheets open as text: a text value starting with `=`, `+`,
    /// `-` or `@` gets a leading `'`, so it can't run as a formula. Changes
    /// the data; plain `Csv` is the faithful one.
    #[serde(rename = "spreadsheet_csv")]
    SpreadsheetCsv,
    /// Tab-separated, for pasting into spreadsheets. NULL and "" are both empty.
    Tsv,
    Json,
    Markdown,
    /// One `INSERT` per row; needs the table name.
    Insert,
    /// A box drawn in `+`, `-` and `|`, as psql and mysql print results.
    Ascii,
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
    /// Ascii: every row, until `finish` knows how wide each column is.
    held: Vec<Vec<Option<String>>>,
    /// Columns without a type (query results).
    untyped: Vec<bool>,
}

impl<W: Write> RowWriter<W> {
    pub fn new(
        format: ExportFormat,
        columns: &[ResultColumn],
        table: Option<&RelationRef>,
        mut out: W,
    ) -> Result<Self> {
        let cols: Vec<_> = columns.iter().map(|c| (c.name.clone(), kind(&c.data_type))).collect();
        let untyped = columns.iter().map(|c| c.data_type.is_empty()).collect();
        let mut insert_prefix = String::new();
        match format {
            ExportFormat::Csv | ExportFormat::SpreadsheetCsv => {
                let safe = format == ExportFormat::SpreadsheetCsv;
                let header: Vec<_> = cols.iter().map(|(n, _)| csv_field(safe, Kind::Text, Some(n))).collect();
                write!(out, "{}\r\n", header.join(","))?;
            }
            ExportFormat::Tsv => {
                let header: Vec<_> = cols.iter().map(|(n, _)| tsv_field(Some(n))).collect();
                writeln!(out, "{}", header.join("\t"))?;
            }
            ExportFormat::Json => out.write_all(b"[")?,
            ExportFormat::Markdown => {
                let names: Vec<_> = cols.iter().map(|(n, _)| markdown_cell(n)).collect();
                let align: Vec<_> = cols.iter().map(|(_, k)| if *k == Kind::Number { "--:" } else { "---" }).collect();
                write!(out, "| {} |\n| {} |\n", names.join(" | "), align.join(" | "))?;
            }
            ExportFormat::Ascii => {}
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
        Ok(RowWriter { format, columns: cols, insert_prefix, out, rows: 0, held: Vec::new(), untyped })
    }

    pub fn row(&mut self, row: &[Option<String>]) -> Result<()> {
        let values = self.columns.iter().zip(row);
        match self.format {
            ExportFormat::Csv | ExportFormat::SpreadsheetCsv => {
                let safe = self.format == ExportFormat::SpreadsheetCsv;
                let fields: Vec<_> = values.map(|((_, kind), v)| csv_field(safe, *kind, v.as_deref())).collect();
                write!(self.out, "{}\r\n", fields.join(","))?;
            }
            ExportFormat::Tsv => {
                let fields: Vec<_> = values.map(|(_, v)| tsv_field(v.as_deref())).collect();
                writeln!(self.out, "{}", fields.join("\t"))?;
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
            ExportFormat::Ascii => self.held.push(row.iter().map(|v| v.as_deref().map(ascii_cell)).collect()),
        }
        self.rows += 1;
        Ok(())
    }

    /// The writer back, and how many rows went through it.
    pub fn finish(mut self) -> Result<(W, u64)> {
        match self.format {
            ExportFormat::Json => self.out.write_all(b"]")?,
            ExportFormat::Ascii => write_ascii(&mut self.out, &self.columns, &self.untyped, &self.held)?,
            _ => {}
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
fn csv_field(safe: bool, kind: Kind, value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some("") => "\"\"".into(),
        Some(v) => {
            let v = if safe { spreadsheet_safe(kind, v) } else { v.to_owned() };
            if v.contains([',', '"', '\n', '\r']) { format!("\"{}\"", v.replace('"', "\"\"")) } else { v }
        }
    }
}

/// Quoted only when it holds a tab, quote or line break, as spreadsheets expect.
fn tsv_field(value: Option<&str>) -> String {
    match value {
        None => String::new(),
        Some(v) if v.contains(['\t', '"', '\n', '\r']) => format!("\"{}\"", v.replace('"', "\"\"")),
        Some(v) => v.to_owned(),
    }
}

/// Excel and similar programs interpret these prefixes as formulas even in a
/// quoted CSV field. Prefix text with an apostrophe; keep typed numbers intact.
fn spreadsheet_safe(kind: Kind, value: &str) -> String {
    if kind == Kind::Text && value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>")
}

/// One line per row: line breaks and tabs are shown escaped.
fn ascii_cell(value: &str) -> String {
    value.replace('\\', "\\\\").replace("\r\n", "\\n").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// Numbers sit on the right: typed number columns, and untyped ones (query
/// results) whose every value reads as a number. Only the layout depends on it.
fn write_ascii(
    out: &mut impl Write,
    columns: &[(String, Kind)],
    untyped: &[bool],
    rows: &[Vec<Option<String>>],
) -> Result<()> {
    const NULL: &str = "NULL";
    let width = |s: &str| s.chars().count();
    let right: Vec<bool> = (0..columns.len())
        .map(|c| match columns[c].1 {
            Kind::Number => true,
            _ if untyped[c] => {
                rows.iter().any(|r| r[c].is_some()) && rows.iter().filter_map(|r| r[c].as_deref()).all(is_number)
            }
            _ => false,
        })
        .collect();
    let widths: Vec<usize> = (0..columns.len())
        .map(|c| rows.iter().map(|r| width(r[c].as_deref().unwrap_or(NULL))).fold(width(&columns[c].0), usize::max))
        .collect();
    let rule = format!("+{}+\n", widths.iter().map(|w| "-".repeat(w + 2)).collect::<Vec<_>>().join("+"));
    let line = |cells: Vec<(&str, bool)>| {
        let padded: Vec<_> = cells
            .iter()
            .zip(&widths)
            .map(|((v, right), w)| {
                let pad = " ".repeat(w - width(v));
                if *right { format!(" {pad}{v} ") } else { format!(" {v}{pad} ") }
            })
            .collect();
        format!("|{}|\n", padded.join("|"))
    };
    out.write_all(rule.as_bytes())?;
    out.write_all(line(columns.iter().map(|(n, _)| (n.as_str(), false)).collect()).as_bytes())?;
    out.write_all(rule.as_bytes())?;
    for row in rows {
        let cells = row.iter().zip(&right).map(|(v, r)| (v.as_deref().unwrap_or(NULL), *r)).collect();
        out.write_all(line(cells).as_bytes())?;
    }
    out.write_all(rule.as_bytes())?;
    Ok(())
}

fn is_number(v: &str) -> bool {
    !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || "+-.eE".contains(c)) && v.parse::<f64>().is_ok()
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

/// A SQL literal for a value of a column of this type, as shown to people.
pub(crate) fn literal(data_type: &str, value: Option<&str>) -> String {
    sql_literal(kind(data_type), value)
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
