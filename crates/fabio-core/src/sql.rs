//! Statement building shared by both engines. Identifiers are only ever taken
//! from the described table, never from the request as-is.

use crate::{Column, Error, Filter, FilterOp, PageRequest, Result};

/// Double-quoted identifier; valid in Postgres and SQLite.
pub fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

pub fn column<'a>(columns: &'a [Column], name: &str) -> Result<&'a Column> {
    columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| Error::Invalid(format!("unknown column {name:?}")))
}

/// How each engine spells the pieces that differ.
pub struct Dialect {
    /// Placeholder for the n-th (1-based) parameter.
    pub param: fn(usize) -> String,
    /// Comparison against a parameter, e.g. `"col" = $1`.
    pub compare: fn(&Column, &str, &str) -> String,
    /// Case-insensitive substring test of a column against a parameter.
    pub contains: fn(&Column, &str) -> String,
    /// Select-list expression for a column.
    pub select: fn(&Column) -> String,
}

/// ` WHERE …` (or empty) for AND-ed filters, and its parameters.
pub fn where_clause(dialect: &Dialect, columns: &[Column], filters: &[Filter]) -> Result<(String, Vec<String>)> {
    let mut params = Vec::new();
    let mut conditions = Vec::new();
    for filter in filters {
        let col = column(columns, &filter.column)?;
        let op = match filter.op {
            FilterOp::IsNull => {
                conditions.push(format!("{} IS NULL", quote(&col.name)));
                continue;
            }
            FilterOp::IsNotNull => {
                conditions.push(format!("{} IS NOT NULL", quote(&col.name)));
                continue;
            }
            FilterOp::Eq => "=",
            FilterOp::Ne => "<>",
            FilterOp::Lt => "<",
            FilterOp::Le => "<=",
            FilterOp::Gt => ">",
            FilterOp::Ge => ">=",
            FilterOp::Contains => "",
        };
        let value = filter
            .value
            .clone()
            .ok_or_else(|| Error::Invalid(format!("filter on {:?} needs a value", col.name)))?;
        params.push(value);
        let placeholder = (dialect.param)(params.len());
        conditions.push(match filter.op {
            FilterOp::Contains => (dialect.contains)(col, &placeholder),
            _ => (dialect.compare)(col, op, &placeholder),
        });
    }
    let clause = if conditions.is_empty() { String::new() } else { format!(" WHERE {}", conditions.join(" AND ")) };
    Ok((clause, params))
}

/// `SELECT count(*)` under the same filters as a page.
pub fn count_statement(dialect: &Dialect, from: &str, columns: &[Column], filters: &[Filter]) -> Result<(String, Vec<String>)> {
    let (clause, params) = where_clause(dialect, columns, filters)?;
    Ok((format!("/* fabio */ SELECT count(*) FROM {from}{clause}"), params))
}

/// Builds `SELECT … LIMIT limit+1 OFFSET …` and its parameters. Fetching one
/// extra row is how the caller learns whether there is a next page.
pub fn page_statement(
    dialect: &Dialect,
    from: &str,
    columns: &[Column],
    request: &PageRequest,
) -> Result<(String, Vec<String>)> {
    let (clause, params) = where_clause(dialect, columns, &request.filters)?;

    // Sort column first, then the primary key so paging is stable. Qualified,
    // because a bare name would bind to the select-list alias (Postgres casts
    // those to text, which sorts "10" before "9").
    let mut order = Vec::new();
    if let Some(sort) = &request.sort {
        let col = column(columns, &sort.column)?;
        order.push(format!("{from}.{} {}", quote(&col.name), if sort.descending { "DESC" } else { "ASC" }));
    }
    for pk in columns.iter().filter(|c| c.primary_key) {
        if request.sort.as_ref().is_none_or(|s| s.column != pk.name) {
            order.push(format!("{from}.{} ASC", quote(&pk.name)));
        }
    }

    let select: Vec<_> = columns.iter().map(dialect.select).collect();
    let mut sql = format!("/* fabio */ SELECT {} FROM {from}{clause}", select.join(", "));
    if !order.is_empty() {
        sql += &format!(" ORDER BY {}", order.join(", "));
    }
    sql += &format!(" LIMIT {} OFFSET {}", u64::from(request.limit) + 1, request.offset);
    Ok((sql, params))
}

#[cfg(test)]
mod tests {
    use super::quote;

    #[test]
    fn quote_doubles_embedded_quotes() {
        assert_eq!(quote(r#"we"ird"#), r#""we""ird""#);
    }
}
