//! Whole databases out and back in.
//!
//! Postgres goes through its own tools — pg_dump, pg_restore and psql — found
//! on this machine (the newest one, since pg_dump refuses a newer server).
//! They connect the way Fabio does: same host, SSH tunnel, TLS mode, and the
//! password through a private passfile, never the command line or environment.
//!
//! SQLite needs no tools: its dump is written here, as `sqlite3 .dump` would,
//! and a database file copy is `VACUUM INTO`.
//!
//! A restore runs as one transaction: if a statement fails, nothing changes.

use std::collections::HashSet;
use std::ffi::OsString;
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::postgres::{PgTarget, SslMode};
use crate::sql::quote;
use crate::{Error, Result, Target};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DumpContent {
    #[default]
    All,
    Schema,
    Data,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DumpFormat {
    /// Plain SQL: readable, restores with psql (Postgres) or Fabio.
    #[default]
    Sql,
    /// Postgres: pg_dump's custom archive (compressed, restores with pg_restore).
    /// SQLite: a copy of the database file.
    Archive,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DumpOptions {
    #[serde(default)]
    pub content: DumpContent,
    #[serde(default)]
    pub format: DumpFormat,
    /// Postgres: only these schemas; every schema when empty.
    #[serde(default)]
    pub schemas: Vec<String>,
    /// Postgres: keep `OWNER TO` and `GRANT`. Off, the dump restores under any role.
    #[serde(default)]
    pub owners: bool,
    /// Drop each object before creating it, so the dump restores over an existing copy.
    #[serde(default)]
    pub clean: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DumpReport {
    pub elapsed_ms: f64,
    /// The size of the dump written, or of the file restored from.
    pub bytes: u64,
    /// The program that did it, e.g. "pg_dump 18.4"; `None` when Fabio did.
    pub tool: Option<String>,
}

/// Called with one line about what's happening now ("public.track").
pub type Progress = Arc<dyn Fn(&str) + Send + Sync>;

/// Writes the database `target` points at to `path`. Resolving `cancel`
/// stops it; a dump that didn't finish leaves no file.
pub async fn dump(
    target: &Target,
    options: &DumpOptions,
    path: &Path,
    progress: Progress,
    cancel: impl Future<Output = ()>,
) -> Result<DumpReport> {
    let started = Instant::now();
    let result = match target {
        Target::Postgres(pg) => pg_dump(pg, options, path, progress, cancel).await,
        Target::Sqlite { path: file } => sqlite_dump(file, options, path, progress, cancel).await.map(|()| None),
    };
    let tool = crate::export::remove_on_error(path, result)?;
    Ok(DumpReport { elapsed_ms: ms(started), bytes: std::fs::metadata(path)?.len(), tool })
}

/// Runs the dump at `path` against the database `target` points at, in one
/// transaction. A Postgres archive goes to pg_restore, SQL to psql; for
/// SQLite, SQL runs here and a database file replaces the contents.
pub async fn restore(
    target: &Target,
    path: &Path,
    progress: Progress,
    cancel: impl Future<Output = ()>,
) -> Result<DumpReport> {
    let started = Instant::now();
    let bytes = std::fs::metadata(path)?.len();
    let tool = match target {
        Target::Postgres(pg) => pg_restore(pg, path, progress, cancel).await?,
        Target::Sqlite { path: file } => {
            sqlite_restore(file, path, progress, cancel).await?;
            None
        }
    };
    Ok(DumpReport { elapsed_ms: ms(started), bytes, tool })
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn cancelled() -> Error {
    Error::Invalid("Cancelled.".into())
}

// ---------------------------------------------------------------- Postgres

async fn pg_dump(
    target: &PgTarget,
    options: &DumpOptions,
    path: &Path,
    progress: Progress,
    cancel: impl Future<Output = ()>,
) -> Result<Option<String>> {
    let tool = Tool::find("pg_dump").await?;
    let mut args: Vec<OsString> = vec!["--verbose".into(), "--file".into(), path.into()];
    args.push(match options.format {
        DumpFormat::Sql => "--format=plain".into(),
        DumpFormat::Archive => "--format=custom".into(),
    });
    match options.content {
        DumpContent::All => {}
        DumpContent::Schema => args.push("--schema-only".into()),
        DumpContent::Data => args.push("--data-only".into()),
    }
    // Schemas are picked by leaving the others out: `--schema=public` would
    // also write `CREATE SCHEMA public`, which fails on any new database.
    for schema in excluded_schemas(target, &options.schemas).await? {
        // A quoted pattern is the exact name, not a pattern.
        args.push(format!("--exclude-schema={}", quote(&schema)).into());
    }
    if !options.owners {
        args.extend(["--no-owner".into(), "--no-privileges".into()]);
    }
    if options.clean && options.content != DumpContent::Data {
        args.extend(["--clean".into(), "--if-exists".into()]);
    }
    tool.run(target, args, progress, cancel).await?;
    Ok(Some(tool.label()))
}

async fn excluded_schemas(target: &PgTarget, keep: &[String]) -> Result<Vec<String>> {
    if keep.is_empty() {
        return Ok(Vec::new());
    }
    let pg = crate::postgres::Pg::connect(target).await?;
    let rows = pg
        .client()
        .query(
            "SELECT /* fabio */ nspname::text FROM pg_namespace
              WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema' AND NOT nspname = ANY($1)",
            &[&keep],
        )
        .await?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

async fn pg_restore(
    target: &PgTarget,
    path: &Path,
    progress: Progress,
    cancel: impl Future<Output = ()>,
) -> Result<Option<String>> {
    let mut magic = [0u8; 5];
    let archive = {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        file.read(&mut magic)? == 5 && &magic == b"PGDMP"
    };
    let (tool, args): (Tool, Vec<OsString>) = if archive {
        let args = vec![
            "--verbose".into(),
            "--single-transaction".into(),
            "--exit-on-error".into(),
            format!("--dbname={}", target.database).into(),
            path.into(),
        ];
        (Tool::find("pg_restore").await?, args)
    } else {
        // -X: no ~/.psqlrc. Output goes nowhere; errors come on stderr with the line.
        let args = vec![
            "-X".into(),
            "--quiet".into(),
            "--no-password".into(),
            "--single-transaction".into(),
            "--set=ON_ERROR_STOP=1".into(),
            "--file".into(),
            path.into(),
        ];
        progress(&format!("Running {}", path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into())));
        (Tool::find("psql").await?, args)
    };
    tool.run(target, args, progress, cancel).await?;
    Ok(Some(tool.label()))
}

/// A Postgres client program and its version.
struct Tool {
    name: &'static str,
    path: PathBuf,
    version: (u32, u32),
}

impl Tool {
    /// The newest `name` on this machine. Apps opened from the Finder get a
    /// bare PATH, so the usual install places are searched too.
    async fn find(name: &'static str) -> Result<Tool> {
        let mut best: Option<Tool> = None;
        let mut seen = HashSet::new();
        for dir in tool_dirs() {
            let path = dir.join(name);
            let Ok(real) = std::fs::canonicalize(&path) else { continue };
            if !seen.insert(real) {
                continue;
            }
            let Ok(out) = tokio::process::Command::new(&path).arg("--version").stdin(Stdio::null()).output().await
            else {
                continue;
            };
            let Some(version) = parse_version(&String::from_utf8_lossy(&out.stdout)) else { continue };
            if best.as_ref().is_none_or(|b| version > b.version) {
                best = Some(Tool { name, path, version });
            }
        }
        best.ok_or_else(|| {
            Error::Invalid(format!(
                "{name} isn't installed. Fabio uses the Postgres client tools; with Homebrew: brew install libpq"
            ))
        })
    }

    fn label(&self) -> String {
        format!("{} {}.{}", self.name, self.version.0, self.version.1)
    }

    async fn run(
        &self,
        target: &PgTarget,
        args: Vec<OsString>,
        progress: Progress,
        cancel: impl Future<Output = ()>,
    ) -> Result<()> {
        let tunnel = match &target.ssh {
            Some(ssh) => Some(crate::tunnel::open(ssh, &target.host, target.port).await?),
            None => None,
        };
        let secrets = Secrets::new(target)?;
        let mut command = tokio::process::Command::new(&self.path);
        // Only what Fabio sets: a PGSERVICE or PGOPTIONS from the shell would change the connection.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("PG") {
                command.env_remove(key);
            }
        }
        command
            .env("PGHOST", &target.host)
            .env("PGPORT", tunnel.as_ref().map_or(target.port, |t| t.local_port).to_string())
            .env("PGUSER", &target.user)
            .env("PGDATABASE", &target.database)
            .env("PGAPPNAME", "fabio")
            .env("PGCONNECT_TIMEOUT", "10")
            .env("PGPASSFILE", &secrets.passfile)
            .env(
                "PGSSLMODE",
                match target.ssl {
                    SslMode::Disable => "disable",
                    SslMode::Prefer => "prefer",
                    SslMode::Require => "require",
                    SslMode::VerifyCa => "verify-ca",
                    SslMode::VerifyFull => "verify-full",
                },
            );
        if tunnel.is_some() {
            // TLS still checks the certificate against `host`.
            command.env("PGHOSTADDR", "127.0.0.1");
        }
        if let Some(root) = &secrets.root_cert {
            command.env("PGSSLROOTCERT", root);
        }
        command.args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);

        let mut child = command.spawn().map_err(|e| Error::Invalid(format!("{}: {e}", self.path.display())))?;
        let mut lines = BufReader::new(child.stderr.take().expect("stderr is piped")).lines();
        let mut problems: Vec<String> = Vec::new();
        let read = async {
            while let Ok(Some(line)) = lines.next_line().await {
                match verbose_line(self.name, &line) {
                    Some(step) => progress(step),
                    None => problems.push(line),
                }
            }
            child.wait().await
        };
        let status = tokio::select! {
            status = read => status?,
            () = cancel => return Err(cancelled()), // the child is killed on drop
        };
        if status.success() {
            return Ok(());
        }
        let mut message = problems.iter().rev().take(12).rev().cloned().collect::<Vec<_>>().join("\n");
        if message.is_empty() {
            message = format!("{} failed ({status})", self.name);
        }
        if message.contains("server version mismatch") {
            message += &format!(
                "\nFabio used {} at {}. Install the Postgres client tools of the server's version or newer.",
                self.label(),
                self.path.display()
            );
        }
        Err(Error::Invalid(message))
    }
}

/// pg_dump and pg_restore narrate with `--verbose`: "pg_dump: dumping contents
/// of table "public.track"". Errors, details and hints are kept for the message.
fn verbose_line<'a>(tool: &str, line: &'a str) -> Option<&'a str> {
    let rest = line.strip_prefix(tool)?.strip_prefix(": ")?;
    let problem = ["error:", "detail:", "hint:", "warning:"].iter().any(|p| rest.starts_with(p));
    (!problem).then_some(rest)
}

/// "pg_dump (PostgreSQL) 18.4 (Homebrew)" → (18, 4); "16beta2" → (16, 0).
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let after = text.split(") ").nth(1)?;
    let number: String = after.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let mut parts = number.split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0)))
}

fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("FABIO_PG_BIN") {
        dirs.push(dir.into());
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    for fixed in ["/opt/homebrew/bin", "/usr/local/bin", "/opt/homebrew/opt/libpq/bin", "/usr/local/opt/libpq/bin"] {
        dirs.push(fixed.into());
    }
    // Versioned installs: Homebrew's postgresql@N, Postgres.app, EDB, Debian/Ubuntu, MacPorts.
    let versioned: [(&str, &str, &str); 6] = [
        ("/opt/homebrew/opt", "postgresql", "bin"),
        ("/usr/local/opt", "postgresql", "bin"),
        ("/Applications/Postgres.app/Contents/Versions", "", "bin"),
        ("/Library/PostgreSQL", "", "bin"),
        ("/usr/lib/postgresql", "", "bin"),
        ("/opt/local/lib", "postgresql", "bin"),
    ];
    for (parent, prefix, bin) in versioned {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(prefix) {
                    dirs.push(entry.path().join(bin));
                }
            }
        }
    }
    dirs
}

/// A private folder holding the passfile (and the RDS CA bundle when needed),
/// removed when the run ends.
struct Secrets {
    dir: PathBuf,
    passfile: PathBuf,
    root_cert: Option<PathBuf>,
}

impl Secrets {
    fn new(target: &PgTarget) -> Result<Secrets> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("fabio-pg-{}-{nanos}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let mut secrets = Secrets { passfile: dir.join("pgpass"), root_cert: None, dir };
        let private = |path: &Path| std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path);
        // `*:*:*:*:password`, with `:` and `\` escaped.
        let password = target.password.as_deref().unwrap_or("").replace('\\', "\\\\").replace(':', "\\:");
        writeln!(private(&secrets.passfile)?, "*:*:*:*:{password}")?;

        secrets.root_cert = match (target.ssl, &target.ca_cert) {
            (SslMode::VerifyCa | SslMode::VerifyFull, Some(path)) => Some(path.clone()),
            (SslMode::VerifyCa | SslMode::VerifyFull, None) if crate::postgres::is_rds(&target.host) => {
                let path = secrets.dir.join("rds.pem");
                private(&path)?.write_all(crate::postgres::RDS_BUNDLE)?;
                Some(path)
            }
            // libpq 16+: the system's CAs, as Fabio uses.
            (SslMode::VerifyCa | SslMode::VerifyFull, None) => Some("system".into()),
            _ => None,
        };
        Ok(secrets)
    }
}

impl Drop for Secrets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------- SQLite

/// Runs `work` on the blocking pool with its own connection; `cancel` interrupts it.
async fn sqlite_job(
    open: impl FnOnce() -> Result<Connection> + Send + 'static,
    work: impl FnOnce(&mut Connection) -> Result<()> + Send + 'static,
    cancel: impl Future<Output = ()>,
) -> Result<()> {
    let (handle_tx, handle_rx) = tokio::sync::oneshot::channel();
    let job = tokio::task::spawn_blocking(move || {
        let mut conn = open()?;
        let _ = handle_tx.send(conn.get_interrupt_handle());
        work(&mut conn)
    });
    tokio::pin!(job);
    tokio::select! {
        result = &mut job => result?,
        () = cancel => {
            if let Ok(handle) = handle_rx.await {
                handle.interrupt();
            }
            let _ = job.await;
            Err(cancelled())
        }
    }
}

async fn sqlite_dump(
    file: &Path,
    options: &DumpOptions,
    path: &Path,
    progress: Progress,
    cancel: impl Future<Output = ()>,
) -> Result<()> {
    let (file, path, options) = (file.to_owned(), path.to_owned(), options.clone());
    if options.format == DumpFormat::Archive && options.content != DumpContent::All {
        return Err(Error::Invalid(
            "A copy of the database file holds everything. Pick SQL to dump only the structure or the data.".into(),
        ));
    }
    let open = move || Ok(Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY)?);
    sqlite_job(
        open,
        move |conn: &mut Connection| match options.format {
            DumpFormat::Archive => {
                // VACUUM INTO won't overwrite; the save dialog already asked.
                if path.exists() {
                    std::fs::remove_file(&path)?;
                }
                progress("Copying the database");
                conn.execute("VACUUM INTO ?1", [path.to_string_lossy()])?;
                Ok(())
            }
            DumpFormat::Sql => {
                let mut out = crate::export::create(&path)?;
                write_sqlite_dump(conn, &options, &mut out, &progress)?;
                out.flush()?;
                Ok(())
            }
        },
        cancel,
    )
    .await
}

/// What `sqlite3 .dump` writes: tables with their rows, then indexes,
/// triggers and views, in one transaction.
fn write_sqlite_dump(
    conn: &Connection,
    options: &DumpOptions,
    out: &mut impl Write,
    progress: &Progress,
) -> Result<()> {
    let schema = options.content != DumpContent::Data;
    let data = options.content != DumpContent::Schema;
    // Shadow tables (FTS's) are made by their virtual table, not by the dump.
    let shadow: HashSet<String> = conn
        .prepare("SELECT name FROM pragma_table_list WHERE schema = 'main' AND type = 'shadow'")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let objects: Vec<(String, String, String)> = conn
        .prepare("SELECT type, name, sql FROM main.sqlite_schema WHERE sql IS NOT NULL ORDER BY rowid")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let tables: Vec<_> = objects
        .iter()
        .filter(|(kind, name, _)| kind == "table" && !name.starts_with("sqlite_") && !shadow.contains(name))
        .collect();

    writeln!(out, "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;")?;
    if schema && options.clean {
        for (kind, name, _) in objects.iter().filter(|(k, ..)| k == "view").chain(tables.iter().copied()) {
            writeln!(out, "DROP {} IF EXISTS {};", kind.to_uppercase(), quote(name))?;
        }
    }
    for (_, name, sql) in &tables {
        progress(name);
        if schema {
            writeln!(out, "{sql};")?;
        }
        if data {
            write_rows(conn, name, out)?;
        }
    }
    let has_sequence = objects.iter().any(|(_, name, _)| name == "sqlite_sequence");
    if data && has_sequence {
        writeln!(out, "DELETE FROM sqlite_sequence;")?;
        write_rows(conn, "sqlite_sequence", out)?;
    }
    if schema {
        for (_, _, sql) in
            objects.iter().filter(|(k, n, _)| matches!(k.as_str(), "index" | "trigger" | "view") && !shadow.contains(n))
        {
            writeln!(out, "{sql};")?;
        }
    }
    writeln!(out, "COMMIT;")?;
    Ok(())
}

/// `INSERT INTO "t" ("a", "b") VALUES(…);` per row. Generated columns are left
/// out: they can't be inserted, and come back by themselves.
fn write_rows(conn: &Connection, table: &str, out: &mut impl Write) -> Result<()> {
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_xinfo(?1) WHERE hidden = 0 ORDER BY cid")?
        .query_map([table], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let names: Vec<_> = columns.iter().map(|c| quote(c)).collect();
    let prefix = format!("INSERT INTO {} ({}) VALUES(", quote(table), names.join(", "));
    let mut statement = conn.prepare(&format!("SELECT {} FROM {}", names.join(", "), quote(table)))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let mut line = prefix.clone();
        for i in 0..columns.len() {
            if i > 0 {
                line.push(',');
            }
            line += &sqlite_literal(row.get_ref(i)?);
        }
        line += ");\n";
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

/// A literal that reads back as the same value and storage class.
fn sqlite_literal(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => "NULL".into(),
        ValueRef::Integer(i) => i.to_string(),
        // Rust prints the shortest text that parses back to the same f64.
        ValueRef::Real(f) if f.is_nan() => "NULL".into(),
        ValueRef::Real(f) if f.is_infinite() => if f > 0.0 { "9e999" } else { "-9e999" }.into(),
        ValueRef::Real(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{f:.1}"),
        ValueRef::Real(f) => format!("{f:e}"),
        ValueRef::Text(t) => format!("'{}'", String::from_utf8_lossy(t).replace('\'', "''")),
        ValueRef::Blob(b) => format!("X'{}'", b.iter().map(|x| format!("{x:02X}")).collect::<String>()),
    }
}

async fn sqlite_restore(file: &Path, path: &Path, progress: Progress, cancel: impl Future<Output = ()>) -> Result<()> {
    let (file, path) = (file.to_owned(), path.to_owned());
    let mut magic = [0u8; 16];
    let is_database = {
        use std::io::Read;
        std::fs::File::open(&path)?.read(&mut magic)? == 16 && &magic == b"SQLite format 3\0"
    };
    let open = move || Ok(Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_WRITE)?);
    sqlite_job(
        open,
        move |conn: &mut Connection| {
            if is_database {
                progress("Copying the database in");
                conn.restore(rusqlite::MAIN_DB, &path, None::<fn(rusqlite::backup::Progress)>)?;
                return Ok(());
            }
            let sql = std::fs::read_to_string(&path)?;
            run_script(conn, &sql, &progress)
        },
        cancel,
    )
    .await
}

/// Every statement of `sql` inside one transaction. The script's own
/// BEGIN/COMMIT (a dump has them) are skipped so the whole file is one unit.
/// Foreign keys are checked once at the end, as a dump's
/// `PRAGMA foreign_keys=OFF` intends: tables can come in any order.
fn run_script(conn: &Connection, sql: &str, progress: &Progress) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF; BEGIN")?;
    let mut batch = rusqlite::Batch::new(conn, sql);
    let (mut at, mut count) = (0usize, 0u64);
    let line = |at: usize| sql[..at].matches('\n').count() + 1;
    let result = (|| -> Result<()> {
        loop {
            at = skip_blank(sql, at);
            let mut statement = match batch.next() {
                Ok(Some(s)) => s,
                Ok(None) => break,
                Err(e) => return Err(Error::Invalid(format!("{e} (line {})", line(at)))),
            };
            // The statement's own text, from where the last one ended.
            let text = statement.expanded_sql().unwrap_or_default();
            if let Some(found) = sql[at..].find(text.trim()) {
                at += found;
            }
            if !matches!(first_keyword(&text).as_str(), "BEGIN" | "COMMIT" | "END") {
                let mut rows = statement.raw_query();
                while rows.next().map_err(|e| Error::Invalid(format!("{e} (line {})", line(at))))?.is_some() {}
                count += 1;
                if count % 5000 == 0 {
                    progress(&format!("{count} statements"));
                }
            }
            at = (at + text.trim().len()).min(sql.len());
        }
        foreign_key_check(conn)
    })();
    let finished = match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(Error::from),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    };
    let _ = conn.execute_batch("PRAGMA foreign_keys = ON");
    finished
}

fn foreign_key_check(conn: &Connection) -> Result<()> {
    let mut check = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = check.query([])?;
    let Some(row) = rows.next()? else { return Ok(()) };
    let (table, parent): (String, String) = (row.get(0)?, row.get(2)?);
    Err(Error::Invalid(format!("Rows in {table} point at rows that aren't in {parent} (foreign key check).")))
}

/// Past whitespace, semicolons and comments: where the next statement starts.
fn skip_blank(sql: &str, mut at: usize) -> usize {
    let bytes = sql.as_bytes();
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() || bytes[at] == b';' {
            at += 1;
        } else if sql[at..].starts_with("--") {
            at = sql[at..].find('\n').map_or(sql.len(), |n| at + n + 1);
        } else if sql[at..].starts_with("/*") {
            at = sql[at + 2..].find("*/").map_or(sql.len(), |n| at + n + 4);
        } else {
            break;
        }
    }
    at
}

fn first_keyword(statement: &str) -> String {
    let start = skip_blank(statement, 0);
    statement[start..].chars().take_while(char::is_ascii_alphabetic).collect::<String>().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_keyword_skips_comments_and_punctuation() {
        assert_eq!(first_keyword("\n-- note\n/* x */ COMMIT;"), "COMMIT");
        assert_eq!(first_keyword("begin transaction;"), "BEGIN");
        assert_eq!(first_keyword("CREATE TRIGGER t BEGIN SELECT 1; END;"), "CREATE");
    }

    #[test]
    fn versions_from_every_flavour() {
        assert_eq!(parse_version("pg_dump (PostgreSQL) 18.4 (Homebrew)\n"), Some((18, 4)));
        assert_eq!(parse_version("psql (PostgreSQL) 16.10 (Ubuntu 16.10-0ubuntu0.24.04.1)"), Some((16, 10)));
        assert_eq!(parse_version("pg_restore (PostgreSQL) 17beta2"), Some((17, 0)));
        assert_eq!(parse_version("not postgres"), None);
    }

    #[test]
    fn verbose_lines_are_progress_and_errors_are_kept() {
        assert_eq!(
            verbose_line("pg_dump", r#"pg_dump: dumping contents of table "public.track""#),
            Some(r#"dumping contents of table "public.track""#)
        );
        assert_eq!(verbose_line("pg_dump", "pg_dump: error: connection failed"), None);
        assert_eq!(verbose_line("psql", "psql:/x.sql:3: ERROR:  oops"), None);
    }

    #[test]
    fn sqlite_literals_read_back_the_same() {
        assert_eq!(sqlite_literal(ValueRef::Real(0.1)), "1e-1");
        assert_eq!(sqlite_literal(ValueRef::Real(3.0)), "3.0");
        assert_eq!(sqlite_literal(ValueRef::Real(1e300)), "1e300");
        assert_eq!(sqlite_literal(ValueRef::Real(f64::NEG_INFINITY)), "-9e999");
        assert_eq!(sqlite_literal(ValueRef::Text(b"it's")), "'it''s'");
        assert_eq!(sqlite_literal(ValueRef::Blob(&[0, 255])), "X'00FF'");
        for f in [0.1, 1.0 / 3.0, 123456.789, -2.5e-300] {
            assert_eq!(sqlite_literal(ValueRef::Real(f)).parse::<f64>().unwrap(), f);
        }
    }
}
