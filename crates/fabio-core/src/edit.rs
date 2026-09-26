//! Cell edits become one `UPDATE` per row, keyed on the whole primary key and
//! guarded by the values the user saw: if the row changed meanwhile, the
//! UPDATE matches nothing and the whole save is rolled back.

use serde::{Deserialize, Serialize};

use crate::export::literal;
use crate::sql::{column, quote};
use crate::{Column, Error, RelationRef, Result, TableInfo};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnValue {
    pub column: String,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellChange {
    pub column: String,
    /// What the grid showed; the UPDATE only applies if it's still this.
    pub old: Option<String>,
    pub new: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowUpdate {
    /// The row's primary-key values as loaded.
    pub key: Vec<ColumnValue>,
    pub changes: Vec<CellChange>,
}

/// How each engine spells the three kinds of condition.
pub(crate) struct EditDialect {
    pub param: fn(usize) -> String,
    /// `"col" = <param>`, cast to the column's type.
    pub assign: fn(&Column, &str) -> String,
    /// Key match, cast so the primary-key index is used.
    pub key: fn(&Column, &str) -> String,
    /// NULL-safe "still has the value the user saw".
    pub unchanged: fn(&Column, &str) -> String,
}

pub(crate) fn validate(info: &TableInfo, relation: &RelationRef, updates: &[RowUpdate]) -> Result<()> {
    let mut pk: Vec<&str> = info.columns.iter().filter(|c| c.primary_key).map(|c| c.name.as_str()).collect();
    if pk.is_empty() {
        return Err(Error::Invalid(format!(
            "{}.{} has no primary key, so a row can't be picked out safely. Edit it with an UPDATE in a query tab.",
            relation.schema, relation.name
        )));
    }
    pk.sort_unstable();
    for update in updates {
        let mut key: Vec<&str> = update.key.iter().map(|k| k.column.as_str()).collect();
        key.sort_unstable();
        if key != pk {
            return Err(Error::Invalid(format!("an edit must name the whole primary key ({})", pk.join(", "))));
        }
        if update.changes.is_empty() {
            return Err(Error::Invalid("an edit with no changes".into()));
        }
        for c in &update.changes {
            column(&info.columns, &c.column)?;
        }
    }
    Ok(())
}

/// The parameterized UPDATE for one row, and its parameters.
pub(crate) fn statement(
    dialect: &EditDialect,
    from: &str,
    info: &TableInfo,
    update: &RowUpdate,
) -> Result<(String, Vec<Option<String>>)> {
    let mut params = Vec::new();
    let mut next = |value: &Option<String>| {
        params.push(value.clone());
        (dialect.param)(params.len())
    };
    let mut set = Vec::new();
    for c in &update.changes {
        set.push((dialect.assign)(column(&info.columns, &c.column)?, &next(&c.new)));
    }
    let mut conditions = Vec::new();
    for k in &update.key {
        conditions.push((dialect.key)(column(&info.columns, &k.column)?, &next(&k.value)));
    }
    for c in &update.changes {
        conditions.push((dialect.unchanged)(column(&info.columns, &c.column)?, &next(&c.old)));
    }
    Ok((format!("UPDATE {from} SET {} WHERE {}", set.join(", "), conditions.join(" AND ")), params))
}

/// What the user is shown before saving: literals instead of parameters, and
/// without the "unchanged" guard, which is how Fabio runs it, not what it does.
pub(crate) fn display(from: &str, info: &TableInfo, update: &RowUpdate) -> Result<String> {
    let mut set = Vec::new();
    for c in &update.changes {
        let col = column(&info.columns, &c.column)?;
        set.push(format!("{} = {}", quote(&col.name), literal(&col.data_type, c.new.as_deref())));
    }
    Ok(format!("UPDATE {from} SET {} WHERE {};", set.join(", "), key_text(info, update, " AND ")?))
}

/// `id = 2`, for messages about a row.
pub(crate) fn key_text(info: &TableInfo, update: &RowUpdate, separator: &str) -> Result<String> {
    let mut parts = Vec::new();
    for k in &update.key {
        let col = column(&info.columns, &k.column)?;
        parts.push(match &k.value {
            None => format!("{} IS NULL", quote(&col.name)),
            Some(v) => format!("{} = {}", quote(&col.name), literal(&col.data_type, Some(v))),
        });
    }
    Ok(parts.join(separator))
}

/// The error when an UPDATE matched no row.
pub(crate) fn stale(info: &TableInfo, update: &RowUpdate) -> Error {
    let key = key_text(info, update, ", ").unwrap_or_default().replace('"', "");
    Error::Invalid(format!(
        "The row with {key} has changed or is gone since it was loaded. Nothing was saved; reload to see it now."
    ))
}
