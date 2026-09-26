use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fabio_core::{
    AuditEntry, AuditLog, CompletionTable, Count, Db, Discovery, ExportFormat, Filter, Insights, NewAuditEntry, Page,
    PageRequest, PgTarget, Plan, QueryResult, Relation, RelationRef, ResultColumn, RowUpdate, Rows, SavedConnection,
    Snippet, Snippets, Sort, Source, Store, TableInfo, Target, format_rows,
};
use serde::Serialize;
use tauri::{Manager, State};

static STARTED: OnceLock<Instant> = OnceLock::new();
const KEYCHAIN_SERVICE: &str = "dev.fabio.app";

struct App {
    store: Store,
    snippets: Snippets,
    audit: AuditLog,
    /// One read-only connection per saved connection, for browsing.
    open: Mutex<HashMap<String, Arc<Db>>>,
    /// One connection per query tab, so tabs have their own session and write mode.
    sessions: Mutex<HashMap<String, Session>>,
}

struct Session {
    connection_id: String,
    db: Arc<Db>,
    /// Restored when the connection has to be reopened.
    writable: bool,
}

/// Said when a query tab's connection died (sleep, network, server restart).
const LOST: &str =
    "The connection was lost and has been reopened. Anything not committed was rolled back; run the statement again.";

/// A failed statement, with where it failed when the database says.
#[derive(Serialize)]
struct QueryError {
    message: String,
    position: Option<u32>,
}

type Res<T> = Result<T, String>;

fn err(e: impl ToString) -> String {
    e.to_string()
}

fn keychain(id: &str) -> Res<keyring::Entry> {
    keyring::Entry::new(KEYCHAIN_SERVICE, id).map_err(err)
}

fn saved_password(id: &str) -> Res<Option<String>> {
    match keychain(id)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(err(e)),
    }
}

/// `password`: `None` keeps what's saved, `Some("")` removes it.
fn with_password(mut target: Target, id: &str, password: Option<String>) -> Res<Target> {
    if let Target::Postgres(pg) = &mut target {
        pg.password = match password {
            Some(p) if p.is_empty() => None,
            Some(p) => Some(p),
            None if id.is_empty() => None,
            None => saved_password(id)?,
        };
    }
    Ok(target)
}

impl App {
    fn session(&self, id: &str) -> Res<(String, Arc<Db>)> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .map(|s| (s.connection_id.clone(), s.db.clone()))
            .ok_or_else(|| "query tab is closed".to_string())
    }

    /// `id` is a saved connection's id, or `id#database` for another database
    /// on the same server (⌘D): same host and credentials, its own workspace.
    async fn open_db(&self, id: &str) -> Res<Db> {
        let (connection_id, database) = match id.split_once('#') {
            Some((c, d)) => (c, Some(d)),
            None => (id, None),
        };
        let saved = self.store.get(connection_id).map_err(err)?;
        let mut target = with_password(saved.target, connection_id, None)?;
        if let (Target::Postgres(pg), Some(database)) = (&mut target, database) {
            pg.database = database.to_owned();
        }
        Db::open(&target).await.map_err(err)
    }

    fn db(&self, id: &str) -> Res<Arc<Db>> {
        self.open.lock().unwrap().get(id).cloned().ok_or_else(|| "connection is not open".to_string())
    }

    /// Runs a browse call; a connection that died (sleep, network) is reopened
    /// and the call tried once more. Browsing holds no state worth losing.
    async fn browsing<T, F>(&self, id: &str, call: impl Fn(Arc<Db>) -> F) -> Res<T>
    where
        F: std::future::Future<Output = fabio_core::Result<T>> + Send,
    {
        let mut db = self.db(id)?;
        if db.is_closed() {
            db = self.reopen(id).await?;
        }
        match call(db.clone()).await {
            Err(_) if db.is_closed() => call(self.reopen(id).await?).await.map_err(err),
            result => result.map_err(err),
        }
    }

    async fn reopen(&self, id: &str) -> Res<Arc<Db>> {
        let db = Arc::new(self.open_db(id).await?);
        self.open.lock().unwrap().insert(id.to_owned(), db.clone());
        Ok(db)
    }

    /// A query tab's connection, reopened (with its write mode) if it died.
    /// Reopening is reported, not hidden: its transaction is gone.
    async fn live_session(&self, id: &str) -> Result<(String, Arc<Db>), QueryError> {
        let (connection_id, db) = self.session(id).map_err(|message| QueryError { message, position: None })?;
        if db.is_closed() {
            self.reopen_session(id).await?;
            return Err(QueryError { message: LOST.into(), position: None });
        }
        Ok((connection_id, db))
    }

    async fn reopen_session(&self, id: &str) -> Result<(), QueryError> {
        let fail = |message: String| QueryError { message, position: None };
        let (connection_id, writable) = {
            let sessions = self.sessions.lock().unwrap();
            let s = sessions.get(id).ok_or_else(|| fail("query tab is closed".into()))?;
            (s.connection_id.clone(), s.writable)
        };
        let db = self.open_db(&connection_id).await.map_err(|e| fail(format!("The connection was lost: {e}")))?;
        if writable {
            db.set_writable(true).await.map_err(|e| fail(e.to_string()))?;
        }
        if let Some(s) = self.sessions.lock().unwrap().get_mut(id) {
            s.db = Arc::new(db);
        }
        Ok(())
    }
}

/// A statement failed; if that's because the connection died, reopen it and say so.
async fn statement_error(app: &App, id: &str, db: &Db, e: fabio_core::Error) -> QueryError {
    if db.is_closed() && app.reopen_session(id).await.is_ok() {
        return QueryError { message: format!("{e}. {LOST}"), position: None };
    }
    QueryError { message: e.to_string(), position: e.position() }
}

#[tauri::command]
fn list_connections(app: State<App>) -> Res<Vec<SavedConnection>> {
    app.store.list().map_err(err)
}

#[tauri::command]
fn has_password(id: String) -> Res<bool> {
    Ok(saved_password(&id)?.is_some())
}

#[tauri::command]
fn parse_url(url: String) -> Res<PgTarget> {
    PgTarget::from_url(&url).map_err(err)
}

#[tauri::command]
fn save_connection(app: State<App>, connection: SavedConnection, password: Option<String>) -> Res<SavedConnection> {
    let saved = app.store.save(connection).map_err(err)?;
    match password {
        Some(p) if p.is_empty() => match keychain(&saved.id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(err(e)),
        },
        Some(p) => keychain(&saved.id)?.set_password(&p).map_err(err)?,
        None => {}
    }
    Ok(saved)
}

#[tauri::command]
fn delete_connection(app: State<App>, id: String) -> Res<()> {
    // With the other databases opened on it (`id#db`).
    app.open.lock().unwrap().retain(|k, _| k != &id && !k.starts_with(&format!("{id}#")));
    app.store.remove(&id).map_err(err)?;
    match keychain(&id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(err(e)),
    }
}

/// Connects without keeping the connection; returns the server version.
#[tauri::command]
async fn test_connection(connection: SavedConnection, password: Option<String>) -> Res<String> {
    let target = with_password(connection.target, &connection.id, password)?;
    let db = Db::open(&target).await.map_err(err)?;
    let sql = match target {
        Target::Postgres(_) => "select version()",
        Target::Sqlite { .. } => "select 'SQLite ' || sqlite_version()",
    };
    let result = db.query(sql).await.map_err(err)?;
    Ok(result.rows[0][0].clone().unwrap_or_default())
}

/// Databases in a project folder (Compose services, SQLite files), to pick from.
#[tauri::command]
async fn discover_folder(path: PathBuf) -> Res<Discovery> {
    tauri::async_runtime::spawn_blocking(move || fabio_core::discover(&path)).await.map_err(err)?.map_err(err)
}

#[tauri::command]
fn open_sqlite_file(app: State<App>, path: PathBuf) -> Res<SavedConnection> {
    app.store.sqlite_file(&path).map_err(err)
}

#[tauri::command]
async fn connect(app: State<'_, App>, id: String) -> Res<Vec<Relation>> {
    let db = Arc::new(app.open_db(&id).await?);
    let relations = db.relations().await.map_err(err)?;
    app.open.lock().unwrap().insert(id, db);
    Ok(relations)
}

#[tauri::command]
fn disconnect(app: State<App>, id: String) {
    app.open.lock().unwrap().remove(&id);
}

/// Databases on the server of an open connection, for ⌘D.
#[tauri::command]
async fn databases(app: State<'_, App>, id: String) -> Res<Vec<String>> {
    app.browsing(&id, |db| async move { db.databases().await }).await
}

#[tauri::command]
async fn relations(app: State<'_, App>, id: String) -> Res<Vec<Relation>> {
    app.browsing(&id, |db| async move { db.relations().await }).await
}

#[tauri::command]
async fn describe(app: State<'_, App>, id: String, relation: RelationRef) -> Res<TableInfo> {
    let relation = &relation;
    app.browsing(&id, |db| async move { db.describe(relation).await }).await
}

/// Exact counts get this long before falling back to the planner's estimate.
const COUNT_TIMEOUT: Duration = Duration::from_secs(2);

#[tauri::command]
async fn count(app: State<'_, App>, id: String, relation: RelationRef, filters: Vec<Filter>) -> Res<Count> {
    let relation = &relation;
    let filters = &filters;
    app.browsing(&id, |db| async move { db.count(relation, filters, COUNT_TIMEOUT).await }).await
}

#[tauri::command]
async fn page(app: State<'_, App>, id: String, request: PageRequest) -> Res<Page> {
    let request = &request;
    app.browsing(&id, |db| async move { db.page(request).await }).await
}

/// Streams a whole table (under the grid's sort and filters) to a file, on its
/// own connection so browsing carries on meanwhile. Returns the row count.
#[tauri::command]
async fn export_table(
    app: State<'_, App>,
    id: String,
    relation: RelationRef,
    sort: Option<Sort>,
    filters: Vec<Filter>,
    format: ExportFormat,
    path: PathBuf,
) -> Res<u64> {
    let db = app.open_db(&id).await?;
    db.export_table(&relation, sort.as_ref(), &filters, format, &path).await.map_err(err)
}

/// Saves rows already on screen (a query result) to a file.
#[tauri::command]
fn export_rows(
    columns: Vec<ResultColumn>,
    rows: Rows,
    format: ExportFormat,
    table: Option<RelationRef>,
    path: PathBuf,
) -> Res<()> {
    let text = format_rows(format, &columns, &rows, table.as_ref()).map_err(err)?;
    std::fs::write(&path, text).map_err(err)
}

/// Rows on screen as text, for the clipboard.
#[tauri::command]
fn copy_rows(columns: Vec<ResultColumn>, rows: Rows, format: ExportFormat, table: Option<RelationRef>) -> Res<String> {
    format_rows(format, &columns, &rows, table.as_ref()).map_err(err)
}

/// The UPDATEs a save would run, for the user to read first.
#[tauri::command]
async fn preview_updates(
    app: State<'_, App>,
    id: String,
    relation: RelationRef,
    updates: Vec<RowUpdate>,
) -> Res<Vec<String>> {
    let relation = &relation;
    let updates = &updates;
    app.browsing(&id, |db| async move { db.update_statements(relation, updates).await }).await
}

/// Saves cell edits in one transaction on a writable connection of its own
/// (browsing stays read-only), and records them like any typed statement.
#[tauri::command]
async fn apply_updates(app: State<'_, App>, id: String, relation: RelationRef, updates: Vec<RowUpdate>) -> Res<u64> {
    let db = app.open_db(&id).await?;
    let sql = db.update_statements(&relation, &updates).await.map_err(err)?.join("\n");
    let started = Instant::now();
    let applied = match db.set_writable(true).await {
        Ok(()) => db.apply_updates(&relation, &updates).await,
        Err(e) => Err(e),
    };
    let entry = NewAuditEntry {
        connection_id: id,
        source: Source::Human,
        sql,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        rows: applied.as_ref().ok().copied(),
        error: applied.as_ref().err().map(ToString::to_string),
    };
    if let Err(e) = app.audit.record(&entry) {
        eprintln!("audit log: {e}");
    }
    applied.map_err(err)
}

#[tauri::command]
async fn completion_schema(app: State<'_, App>, id: String) -> Res<Vec<CompletionTable>> {
    app.browsing(&id, |db| async move { db.completion_schema().await }).await
}

/// Opens a query tab's own connection; returns the session id.
#[tauri::command]
async fn open_session(app: State<'_, App>, connection_id: String) -> Res<String> {
    let db = Arc::new(app.open_db(&connection_id).await?);
    let id = format!("{connection_id}:{:x}", Instant::now().duration_since(*STARTED.get().unwrap()).as_nanos());
    app.sessions.lock().unwrap().insert(id.clone(), Session { connection_id, db, writable: false });
    Ok(id)
}

#[tauri::command]
fn close_session(app: State<App>, id: String) {
    app.sessions.lock().unwrap().remove(&id);
}

#[tauri::command]
async fn set_write_mode(app: State<'_, App>, id: String, writable: bool) -> Res<()> {
    let db = app.live_session(&id).await.map_err(|e| e.message)?.1;
    db.set_writable(writable).await.map_err(err)?;
    if let Some(s) = app.sessions.lock().unwrap().get_mut(&id) {
        s.writable = writable;
    }
    Ok(())
}

#[tauri::command]
async fn cancel(app: State<'_, App>, id: String) -> Res<()> {
    app.session(&id)?.1.canceller().cancel().await.map_err(err)
}

/// Runs a statement in a query tab and records it in the audit log.
#[tauri::command]
async fn run_statement(app: State<'_, App>, id: String, sql: String) -> Result<QueryResult, QueryError> {
    let (connection_id, db) = app.live_session(&id).await?;
    let started = Instant::now();
    let result = db.query(&sql).await;
    let entry = NewAuditEntry {
        connection_id,
        source: Source::Human,
        sql,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        rows: result.as_ref().ok().map(|r| r.rows.len() as u64),
        error: result.as_ref().err().map(ToString::to_string),
    };
    if let Err(e) = app.audit.record(&entry) {
        eprintln!("audit log: {e}");
    }
    match result {
        Ok(r) => Ok(r),
        Err(e) => Err(statement_error(&app, &id, &db, e).await),
    }
}

/// Explains a statement in a query tab. With `analyze` it really runs (rolled
/// back), so it is logged like any other statement.
#[tauri::command]
async fn explain(app: State<'_, App>, id: String, sql: String, analyze: bool) -> Result<Plan, QueryError> {
    let (connection_id, db) = app.live_session(&id).await?;
    let started = Instant::now();
    let plan = db.explain(&sql, analyze).await;
    let entry = NewAuditEntry {
        connection_id,
        source: Source::Human,
        sql: format!("EXPLAIN{} {sql}", if analyze { " ANALYZE" } else { "" }),
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        rows: None,
        error: plan.as_ref().err().map(ToString::to_string),
    };
    if let Err(e) = app.audit.record(&entry) {
        eprintln!("audit log: {e}");
    }
    match plan {
        Ok(p) => Ok(p),
        Err(e) => Err(statement_error(&app, &id, &db, e).await),
    }
}

#[tauri::command]
async fn insights(app: State<'_, App>, id: String) -> Res<Insights> {
    app.browsing(&id, |db| async move { db.insights().await }).await
}

#[tauri::command]
fn list_snippets(app: State<App>) -> Res<Vec<Snippet>> {
    app.snippets.list().map_err(err)
}

#[tauri::command]
fn save_snippet(app: State<App>, snippet: Snippet) -> Res<Snippet> {
    app.snippets.save(snippet).map_err(err)
}

#[tauri::command]
fn delete_snippet(app: State<App>, id: String) -> Res<()> {
    app.snippets.remove(&id).map_err(err)
}

#[tauri::command]
fn history(app: State<App>, connection_id: String, limit: u32) -> Res<Vec<AuditEntry>> {
    app.audit.recent(Some(&connection_id), limit).map_err(err)
}

/// Agent statements newer than `after`, newest first. The agent panel polls this;
/// the MCP server writes them from its own process.
#[tauri::command]
fn agent_activity(app: State<App>, after: i64) -> Res<Vec<AuditEntry>> {
    app.audit.agent_since(after, 500).map_err(err)
}

/// Called by the frontend after its first paint. With `FABIO_EXIT_ON_READY=1`
/// the app prints the startup time and quits — that's how `bench/startup.py`
/// measures cold start.
#[tauri::command]
fn app_ready(app: tauri::AppHandle) {
    let elapsed = STARTED.get().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
    if std::env::var_os("FABIO_EXIT_ON_READY").is_some() {
        println!("ready_ms={elapsed:.0}");
        app.exit(0);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    STARTED.get_or_init(Instant::now);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config = app.path().app_config_dir()?;
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            app.manage(App {
                store: Store::new(config.join("connections.json")),
                snippets: Snippets::new(config.join("snippets.json")),
                audit: AuditLog::open(data.join("audit.sqlite"))?,
                open: Mutex::default(),
                sessions: Mutex::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_connections,
            has_password,
            parse_url,
            save_connection,
            delete_connection,
            test_connection,
            open_sqlite_file,
            discover_folder,
            connect,
            disconnect,
            relations,
            databases,
            describe,
            count,
            page,
            completion_schema,
            export_table,
            export_rows,
            copy_rows,
            preview_updates,
            apply_updates,
            open_session,
            close_session,
            set_write_mode,
            cancel,
            run_statement,
            explain,
            insights,
            history,
            list_snippets,
            save_snippet,
            delete_snippet,
            agent_activity,
            app_ready,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
