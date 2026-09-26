//! Saved connections, as a JSON file. Passwords never touch this file; the app
//! keeps them in the OS keychain under the connection id.

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
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| Error::Invalid(format!("{}: {e}", self.path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(Error::Invalid(e.to_string())),
        }
    }

    pub fn get(&self, id: &str) -> Result<SavedConnection> {
        self.list()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::NotFound(format!("connection {id}")))
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
        self.write(&all)?;
        Ok(connection)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let mut all = self.list()?;
        all.retain(|c| c.id != id);
        self.write(&all)
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

    fn write(&self, all: &[SavedConnection]) -> Result<()> {
        let io = |e: std::io::Error| Error::Invalid(format!("{}: {e}", self.path.display()));
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let json = serde_json::to_vec_pretty(all).map_err(|e| Error::Invalid(e.to_string()))?;
        // Write-then-rename so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }
}

fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{nanos:x}")
}
