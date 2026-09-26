//! Saved connections and snippets, as JSON files. Passwords never touch these
//! files; the app keeps them in the OS keychain under the connection id.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{Error, Result, Target};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedConnection {
    /// Empty for a connection that hasn't been saved yet.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub target: Target,
    /// Agents (through the MCP server) may query this connection, read-only.
    #[serde(default)]
    pub agent: bool,
}

pub struct Store {
    path: PathBuf,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into() }
    }

    pub fn list(&self) -> Result<Vec<SavedConnection>> {
        read_list(&self.path)
    }

    pub fn get(&self, id: &str) -> Result<SavedConnection> {
        self.list()?.into_iter().find(|c| c.id == id).ok_or_else(|| Error::NotFound(format!("connection {id}")))
    }

    /// Inserts or replaces by id, assigning one if empty. The password is stripped.
    pub fn save(&self, mut connection: SavedConnection) -> Result<SavedConnection> {
        if connection.id.is_empty() {
            connection.id = new_id();
        }
        if let Target::Postgres(pg) = &mut connection.target {
            pg.password = None;
        }
        let mut all = self.list()?;
        match all.iter_mut().find(|c| c.id == connection.id) {
            Some(existing) => *existing = connection.clone(),
            None => all.push(connection.clone()),
        }
        write_list(&self.path, &all)?;
        Ok(connection)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut all = self.list()?;
        all.retain(|c| c.id != id);
        write_list(&self.path, &all)
    }

    /// The saved connection for a SQLite file, created on first open.
    pub fn sqlite_file(&self, path: &Path) -> Result<SavedConnection> {
        let existing = self.list()?.into_iter().find(|c| match &c.target {
            Target::Sqlite { path: p } => p == path,
            _ => false,
        });
        match existing {
            Some(c) => Ok(c),
            None => self.save(SavedConnection {
                id: String::new(),
                name: path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()),
                target: Target::Sqlite { path: path.to_owned() },
                agent: false,
            }),
        }
    }
}

/// A saved SQL statement, reachable from ⌘K.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    /// Empty for one that hasn't been saved yet.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub sql: String,
}

pub struct Snippets {
    path: PathBuf,
}

impl Snippets {
    pub fn new(path: impl Into<PathBuf>) -> Snippets {
        Snippets { path: path.into() }
    }

    /// By name, case-insensitively.
    pub fn list(&self) -> Result<Vec<Snippet>> {
        let mut all: Vec<Snippet> = read_list(&self.path)?;
        all.sort_by_key(|s| s.name.to_lowercase());
        Ok(all)
    }

    /// Inserts or replaces by id, assigning one if empty.
    pub fn save(&self, mut snippet: Snippet) -> Result<Snippet> {
        snippet.name = snippet.name.trim().to_owned();
        if snippet.name.is_empty() {
            return Err(Error::Invalid("A snippet needs a name.".into()));
        }
        if snippet.id.is_empty() {
            snippet.id = new_id();
        }
        let mut all: Vec<Snippet> = read_list(&self.path)?;
        match all.iter_mut().find(|s| s.id == snippet.id) {
            Some(existing) => *existing = snippet.clone(),
            None => all.push(snippet.clone()),
        }
        write_list(&self.path, &all)?;
        Ok(snippet)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut all: Vec<Snippet> = read_list(&self.path)?;
        all.retain(|s| s.id != id);
        write_list(&self.path, &all)
    }
}

fn read_list<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| Error::Invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(Error::Invalid(e.to_string())),
    }
}

fn write_list<T: Serialize>(path: &Path, all: &[T]) -> Result<()> {
    let io = |e: std::io::Error| Error::Invalid(format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let json = serde_json::to_vec_pretty(all).map_err(|e| Error::Invalid(e.to_string()))?;
    // Write-then-rename so a crash never leaves a half-written file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{nanos:x}")
}
