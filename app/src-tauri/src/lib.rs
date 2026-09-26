use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use fabio_core::{
    Count, Db, Filter, Page, PageRequest, PgTarget, QueryResult, Relation, RelationRef, SavedConnection, Store,
    TableInfo, Target,
};
use tauri::{Manager, State};

static STARTED: OnceLock<Instant> = OnceLock::new();
const KEYCHAIN_SERVICE: &str = "dev.fabio.app";

struct App {
    store: Store,
    open: Mutex<HashMap<String, Arc<Db>>>,
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
    fn db(&self, id: &str) -> Res<Arc<Db>> {
        self.open
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| "connection is not open".to_string())
    }
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
    app.open.lock().unwrap().remove(&id);
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

#[tauri::command]
fn open_sqlite_file(app: State<App>, path: PathBuf) -> Res<SavedConnection> {
    app.store.sqlite_file(&path).map_err(err)
}

#[tauri::command]
async fn connect(app: State<'_, App>, id: String) -> Res<Vec<Relation>> {
    let saved = app.store.get(&id).map_err(err)?;
    let db = Arc::new(Db::open(&with_password(saved.target, &id, None)?).await.map_err(err)?);
    let relations = db.relations().await.map_err(err)?;
    app.open.lock().unwrap().insert(id, db);
    Ok(relations)
}

#[tauri::command]
fn disconnect(app: State<App>, id: String) {
    app.open.lock().unwrap().remove(&id);
}

#[tauri::command]
async fn relations(app: State<'_, App>, id: String) -> Res<Vec<Relation>> {
    app.db(&id)?.relations().await.map_err(err)
}

#[tauri::command]
async fn describe(app: State<'_, App>, id: String, relation: RelationRef) -> Res<TableInfo> {
    app.db(&id)?.describe(&relation).await.map_err(err)
}

/// Exact counts get this long before falling back to the planner's estimate.
const COUNT_TIMEOUT: Duration = Duration::from_secs(2);

#[tauri::command]
async fn count(app: State<'_, App>, id: String, relation: RelationRef, filters: Vec<Filter>) -> Res<Count> {
    app.db(&id)?.count(&relation, &filters, COUNT_TIMEOUT).await.map_err(err)
}

#[tauri::command]
async fn page(app: State<'_, App>, id: String, request: PageRequest) -> Res<Page> {
    app.db(&id)?.page(&request).await.map_err(err)
}

#[tauri::command]
async fn run_query(app: State<'_, App>, id: String, sql: String) -> Res<QueryResult> {
    app.db(&id)?.query(&sql).await.map_err(err)
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
            let dir = app.path().app_config_dir()?;
            app.manage(App { store: Store::new(dir.join("connections.json")), open: Mutex::default() });
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
            connect,
            disconnect,
            relations,
            describe,
            count,
            page,
            run_query,
            app_ready,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
