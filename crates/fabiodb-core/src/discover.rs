//! Databases in a project folder: Postgres services in Docker Compose files
//! and SQLite files. Nothing is connected to or saved here; the app shows
//! what was found and the user picks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use yaml_rust2::{Yaml, YamlLoader};

use crate::{PgTarget, Result, SslMode, Target};

const COMPOSE_FILES: &[&str] = &["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];
const SQLITE_EXTENSIONS: &[&str] = &["db", "sqlite", "sqlite3", "db3"];
const SKIPPED_DIRS: &[&str] =
    &["node_modules", ".git", "target", "vendor", "dist", "build", ".venv", "venv", "__pycache__", ".next"];
const SQLITE_DEPTH: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct Discovered {
    /// "project · service", or the SQLite file's path in the folder.
    pub name: String,
    /// With the password, when the Compose file has one.
    pub target: Target,
    /// The file it was found in.
    pub source: String,
    /// Why it may not connect as is.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Discovery {
    pub found: Vec<Discovered>,
    /// Files that couldn't be read, with the reason.
    pub problems: Vec<String>,
}

pub fn discover(dir: &Path) -> Result<Discovery> {
    let mut discovery = Discovery::default();
    let mut vars = read_env_file(&dir.join(".env"));
    // As in Compose, the shell's environment wins over .env.
    vars.extend(std::env::vars());
    for file in COMPOSE_FILES.iter().map(|f| dir.join(f)).filter(|p| p.is_file()) {
        match compose_services(dir, &file, &vars) {
            Ok(found) => discovery.found.extend(found),
            Err(problem) => discovery.problems.push(problem),
        }
    }
    sqlite_files(dir, dir, 0, &mut discovery.found);
    Ok(discovery)
}

fn compose_services(
    dir: &Path,
    file: &Path,
    vars: &HashMap<String, String>,
) -> std::result::Result<Vec<Discovered>, String> {
    let fail = |why: String| format!("{}: {why}", file.display());
    let text = std::fs::read_to_string(file).map_err(|e| fail(e.to_string()))?;
    let docs = YamlLoader::load_from_str(&text).map_err(|e| fail(e.to_string()))?;
    let Some(doc) = docs.first() else { return Ok(vec![]) };
    let project = doc["name"]
        .as_str()
        .map(|n| interpolate(n, vars))
        .unwrap_or_else(|| dir.file_name().map_or_else(|| "compose".into(), |n| n.to_string_lossy().into_owned()));
    let Some(services) = doc["services"].as_hash() else { return Ok(vec![]) };

    let mut found = Vec::new();
    for (name, service) in services {
        let Some(name) = name.as_str() else { continue };
        let env = service_env(dir, service, vars);
        let image = service["image"].as_str().map(|i| interpolate(i, vars).to_lowercase()).unwrap_or_default();
        let is_postgres = ["postgres", "postgis", "timescaledb", "pgvector"].iter().any(|p| image.contains(p))
            || env.contains_key("POSTGRES_PASSWORD");
        if !is_postgres {
            continue;
        }
        let user = env.get("POSTGRES_USER").cloned().unwrap_or_else(|| "postgres".into());
        let container_port = env.get("PGPORT").and_then(|p| p.parse().ok()).unwrap_or(5432);
        let published = published_port(&service["ports"], container_port, vars);
        let (host, port) = published.clone().unwrap_or_else(|| ("localhost".into(), 5432));
        found.push(Discovered {
            name: format!("{project} · {name}"),
            target: Target::Postgres(PgTarget {
                host,
                port,
                database: env.get("POSTGRES_DB").cloned().unwrap_or_else(|| user.clone()),
                user,
                password: env.get("POSTGRES_PASSWORD").cloned(),
                // Local containers don't serve TLS unless configured to.
                ssl: SslMode::Disable,
                ca_cert: None,
                ssh: None,
            }),
            source: file.display().to_string(),
            note: published.is_none().then(|| {
                format!(
                    "Port {container_port} is not published on the host. Add ports: [\"{container_port}:{container_port}\"] to the service to reach it."
                )
            }),
        });
    }
    Ok(found)
}

/// env_file entries first, then `environment` (map or KEY=VALUE list) on top.
fn service_env(dir: &Path, service: &Yaml, vars: &HashMap<String, String>) -> HashMap<String, String> {
    let mut env = HashMap::new();
    let env_files: Vec<&Yaml> = match &service["env_file"] {
        Yaml::Array(files) => files.iter().collect(),
        other => vec![other],
    };
    for f in env_files {
        let path = f.as_str().or_else(|| f["path"].as_str());
        if let Some(path) = path {
            env.extend(read_env_file(&dir.join(interpolate(path, vars))));
        }
    }
    match &service["environment"] {
        Yaml::Hash(map) => {
            for (k, v) in map {
                if let (Some(k), Some(v)) = (k.as_str(), scalar(v)) {
                    env.insert(k.to_owned(), interpolate(&v, vars));
                }
            }
        }
        Yaml::Array(list) => {
            for item in list.iter().filter_map(Yaml::as_str) {
                if let Some((k, v)) = item.split_once('=') {
                    env.insert(k.to_owned(), interpolate(v, vars));
                }
            }
        }
        _ => {}
    }
    env
}

/// The host side of the mapping to `container_port`, from short ("[ip:]host:container")
/// or long ({target, published, host_ip}) syntax.
fn published_port(ports: &Yaml, container_port: u16, vars: &HashMap<String, String>) -> Option<(String, u16)> {
    let host = |ip: Option<String>| match ip.as_deref() {
        None | Some("") | Some("0.0.0.0") | Some("::") => "localhost".to_owned(),
        Some(ip) => ip.trim_matches(['[', ']']).to_owned(),
    };
    for entry in ports.as_vec()? {
        match entry {
            Yaml::Hash(_) => {
                let target = scalar(&entry["target"]).and_then(|t| t.parse::<u16>().ok());
                let published = scalar(&entry["published"]).map(|p| interpolate(&p, vars));
                if target == Some(container_port)
                    && let Some(port) = published.and_then(|p| p.parse().ok())
                {
                    return Some((host(entry["host_ip"].as_str().map(str::to_owned)), port));
                }
            }
            other => {
                let Some(text) = scalar(other).map(|s| interpolate(&s, vars)) else { continue };
                let text = text.split('/').next().unwrap_or_default();
                // Split from the right: an IPv6 host_ip has colons of its own.
                let parts: Vec<&str> = text.rsplitn(3, ':').collect();
                if parts.len() < 2 || parts[0].parse::<u16>().ok() != Some(container_port) {
                    continue;
                }
                if let Ok(port) = parts[1].parse() {
                    return Some((host(parts.get(2).map(|s| s.to_string())), port));
                }
            }
        }
    }
    None
}

fn scalar(v: &Yaml) -> Option<String> {
    match v {
        Yaml::String(s) | Yaml::Real(s) => Some(s.clone()),
        Yaml::Integer(i) => Some(i.to_string()),
        Yaml::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `KEY=VALUE` lines; `#` comments, `export `, and surrounding quotes allowed.
fn read_env_file(path: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(path) else { return HashMap::new() };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.strip_prefix("export ").unwrap_or(l).split_once('='))
        .map(|(k, v)| {
            let v = v.trim();
            let unquoted = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(v);
            (k.trim().to_owned(), unquoted.to_owned())
        })
        .collect()
}

/// Compose interpolation: `$VAR`, `${VAR}`, `${VAR:-default}`, `${VAR-default}`,
/// `${VAR:?error}` (left empty), and `$$` for a literal dollar.
fn interpolate(text: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix('$') {
            out.push('$');
            rest = r;
        } else if let Some(r) = rest.strip_prefix('{') {
            let Some(end) = closing_brace(r) else {
                out.push_str("${");
                rest = r;
                continue;
            };
            let expr = &r[..end];
            rest = &r[end + 1..];
            let (name, fallback) = if let Some((n, d)) = expr.split_once(":-") {
                (n, vars.get(n).filter(|v| !v.is_empty()).is_none().then_some(d))
            } else if let Some((n, d)) = expr.split_once('-') {
                (n, (!vars.contains_key(n)).then_some(d))
            } else {
                (expr.split([':', '?']).next().unwrap_or(expr), None)
            };
            match fallback {
                Some(d) => out.push_str(&interpolate(d, vars)),
                None => out.push_str(vars.get(name).map_or("", String::as_str)),
            }
        } else {
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
            if end == 0 {
                out.push('$');
            } else {
                out.push_str(vars.get(&rest[..end]).map_or("", String::as_str));
            }
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// Where the `}` closing an already-opened `${` is, counting nested ones.
fn closing_brace(text: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(i),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// SQLite files by their header, a few levels down, skipping dependency folders.
fn sqlite_files(root: &Path, dir: &Path, depth: usize, found: &mut Vec<Discovered>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if path.is_dir() {
            if depth < SQLITE_DEPTH && !SKIPPED_DIRS.contains(&name.as_str()) {
                sqlite_files(root, &path, depth + 1, found);
            }
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if SQLITE_EXTENSIONS.contains(&ext.as_str()) && is_sqlite(&path) {
            found.push(Discovered {
                name: path.strip_prefix(root).unwrap_or(&path).display().to_string(),
                source: path.display().to_string(),
                target: Target::Sqlite { path },
                note: None,
            });
        }
    }
}

fn is_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let mut header = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut header)).is_ok() && &header == b"SQLite format 3\0"
}

#[cfg(test)]
mod tests {
    use super::interpolate;
    use std::collections::HashMap;

    #[test]
    fn interpolation_follows_compose() {
        let vars: HashMap<String, String> = [("A".into(), "1".into()), ("EMPTY".into(), String::new())].into();
        assert_eq!(interpolate("x${A}y$A-$$", &vars), "x1y1-$");
        assert_eq!(interpolate("${MISSING:-d}|${EMPTY:-d}|${EMPTY-d}|${MISSING-d}", &vars), "d|d||d");
        assert_eq!(interpolate("${A:?required}", &vars), "1");
        assert_eq!(interpolate("${B:-${A}}", &vars), "1");
    }
}
