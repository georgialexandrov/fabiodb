use std::sync::OnceLock;
use std::time::Instant;

use fabio_core::{QueryResult, Target};

static STARTED: OnceLock<Instant> = OnceLock::new();

#[tauri::command]
async fn run_query(target: Target, sql: String) -> Result<QueryResult, String> {
    fabio_core::run(&target, &sql).await.map_err(|e| e.to_string())
}

/// Called by the frontend after its first paint. With `FABIO_EXIT_ON_READY=1`
/// the app prints the startup time and quits — that's how `bench/startup.sh`
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
        .invoke_handler(tauri::generate_handler![run_query, app_ready])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
