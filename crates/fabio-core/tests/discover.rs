//! Finding databases in a project folder: Postgres services in Compose files
//! (with .env interpolation) and SQLite files.

use std::path::{Path, PathBuf};

use fabio_core::{SslMode, Target, discover};

fn folder(files: &[(&str, &str)]) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("fabio-discover-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (name, content) in files {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    dir
}

fn pg(target: &Target) -> &fabio_core::PgTarget {
    match target {
        Target::Postgres(t) => t,
        _ => panic!("not postgres: {target:?}"),
    }
}

#[test]
fn reads_a_postgres_service_with_env_interpolation() {
    let dir = folder(&[
        (
            "compose.yaml",
            r#"
name: shop
services:
  db:
    image: postgres:17-alpine
    environment:
      POSTGRES_USER: ${DB_USER}
      POSTGRES_PASSWORD: ${DB_PASSWORD:-secret}
      POSTGRES_DB: shop
    ports:
      - "127.0.0.1:${DB_PORT:-5433}:5432"
  web:
    image: nginx
    ports: ["8080:80"]
"#,
        ),
        (".env", "DB_USER=shopper\n# comment\nDB_PORT=6000\n"),
    ]);

    let found = discover(&dir).unwrap().found;

    assert_eq!(found.len(), 1, "{found:?}");
    let t = pg(&found[0].target);
    assert_eq!((t.host.as_str(), t.port, t.user.as_str(), t.database.as_str()), ("127.0.0.1", 6000, "shopper", "shop"));
    assert_eq!(t.password.as_deref(), Some("secret"));
    assert_eq!(t.ssl, SslMode::Disable);
    assert_eq!(found[0].name, "shop · db");
    assert!(found[0].source.ends_with("compose.yaml"));
    assert_eq!(found[0].note, None);
}

#[test]
fn defaults_list_environment_env_files_and_long_ports() {
    let dir = folder(&[
        (
            "docker-compose.yml",
            r#"
services:
  postgres:
    image: postgis/postgis:16-3.4
    env_file: db.env
    environment:
      - POSTGRES_PASSWORD=pw
    ports:
      - target: 5432
        published: "5544"
"#,
        ),
        ("db.env", "POSTGRES_USER=gis\n"),
    ]);

    let found = discover(&dir).unwrap().found;

    let t = pg(&found[0].target);
    // No POSTGRES_DB: the image names the database after the user.
    assert_eq!((t.host.as_str(), t.port, t.user.as_str(), t.database.as_str()), ("localhost", 5544, "gis", "gis"));
    assert_eq!(t.password.as_deref(), Some("pw"));
    // No top-level name: the folder names the project.
    assert!(found[0].name.ends_with(" · postgres"), "{}", found[0].name);
}

#[test]
fn a_service_without_a_published_port_is_found_with_a_note() {
    let dir = folder(&[("compose.yml", "services:\n  pg:\n    image: postgres\n    expose: ['5432']\n")]);

    let found = discover(&dir).unwrap().found;

    let t = pg(&found[0].target);
    assert_eq!((t.port, t.user.as_str(), t.database.as_str()), (5432, "postgres", "postgres"));
    assert!(found[0].note.as_deref().unwrap().contains("not published"), "{:?}", found[0].note);
}

#[test]
fn finds_sqlite_files_but_not_in_dependency_folders() {
    let dir = folder(&[
        ("data/app.sqlite", "SQLite format 3\0"),
        ("notes.db", "not sqlite at all"),
        ("node_modules/pkg/test.sqlite", "SQLite format 3\0"),
    ]);

    let found = discover(&dir).unwrap().found;

    let paths: Vec<_> = found
        .iter()
        .map(|d| match &d.target {
            Target::Sqlite { path } => path.strip_prefix(&dir).unwrap().to_path_buf(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(paths, [Path::new("data/app.sqlite")]);
}

#[test]
fn a_broken_compose_file_is_reported_not_fatal() {
    let dir = folder(&[("compose.yaml", "services: [unclosed"), ("x.sqlite", "SQLite format 3\0")]);

    let discovery = discover(&dir).unwrap();

    assert_eq!(discovery.found.len(), 1);
    assert_eq!(discovery.problems.len(), 1);
    assert!(discovery.problems[0].contains("compose.yaml"), "{:?}", discovery.problems);
}
