//! Where a connection's diagram lives. Without a DBML file it's Fabio's own
//! business (a layout in the app's folder). Once exported, the model and its
//! layout sit side by side wherever the user put them — `schema.dbml` and
//! `schema.layout.json` — and `links.json` remembers where, so nothing has
//! to search the disk.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// A DBML file a workspace was exported to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub dbml: PathBuf,
    /// macOS bookmark (hex), which still finds the file after the folder is
    /// moved or renamed. The path alone is the fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
}

/// `links.json`: workspace id (`connection` or `connection#database`) → link.
pub struct Links {
    path: PathBuf,
}

impl Links {
    pub fn new(path: impl Into<PathBuf>) -> Links {
        Links { path: path.into() }
    }

    pub fn get(&self, id: &str) -> Result<Option<Link>> {
        Ok(self.read()?.remove(id))
    }

    pub fn set(&self, id: &str, link: Link) -> Result<()> {
        let mut all = self.read()?;
        all.insert(id.to_owned(), link);
        self.write(&all)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut all = self.read()?;
        if all.remove(id).is_some() {
            self.write(&all)?;
        }
        Ok(())
    }

    fn read(&self) -> Result<BTreeMap<String, Link>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| invalid(&self.path, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(invalid(&self.path, e)),
        }
    }

    fn write(&self, all: &BTreeMap<String, Link>) -> Result<()> {
        let json = serde_json::to_string_pretty(all).map_err(|e| invalid(&self.path, e))?;
        write_atomic(&self.path, &json)
    }
}

/// Table positions, keyed as DBML names them (`album`, `perf.big`). Whole
/// pixels and sorted keys, so a moved table is a one-line diff.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub tables: BTreeMap<String, [i32; 2]>,
}

impl Layout {
    /// A missing file is an empty layout: the diagram lays itself out.
    pub fn read(path: &Path) -> Result<Layout> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| invalid(path, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Layout::default()),
            Err(e) => Err(invalid(path, e)),
        }
    }

    /// One table per line.
    pub fn write(&self, path: &Path) -> Result<()> {
        let mut out = String::from("{\n  \"tables\": {");
        for (i, (name, [x, y])) in self.tables.iter().enumerate() {
            let comma = if i + 1 < self.tables.len() { "," } else { "" };
            let _ = write!(out, "\n    {}: [{x}, {y}]{comma}", serde_json::Value::String(name.clone()));
        }
        out.push_str(if self.tables.is_empty() { "}\n}\n" } else { "\n  }\n}\n" });
        write_atomic(path, &out)
    }
}

/// `schema.dbml` → `schema.layout.json`, next to it.
pub fn layout_path(dbml: &Path) -> PathBuf {
    let stem = dbml.file_stem().map_or_else(|| "schema".into(), |s| s.to_string_lossy().into_owned());
    dbml.with_file_name(format!("{stem}.layout.json"))
}

/// Written whole or not at all: a crash mid-write leaves the old file.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| invalid(dir, e))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| invalid(path, e))?;
    std::fs::rename(&tmp, path).map_err(|e| invalid(path, e))
}

fn invalid(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{}: {e}", path.display()))
}
