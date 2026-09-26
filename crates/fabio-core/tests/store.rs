use fabio_core::{PgTarget, SavedConnection, SslMode, Store, Target};

fn temp_store() -> (Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("fabio-store-{}", std::process::id()));
    let path = dir.join(format!("{:?}.json", std::thread::current().id()).replace(['(', ')'], ""));
    let _ = std::fs::remove_file(&path);
    (Store::new(&path), path)
}

fn pg(password: Option<&str>) -> SavedConnection {
    SavedConnection {
        id: String::new(),
        name: "local".into(),
        target: Target::Postgres(PgTarget {
            host: "localhost".into(),
            port: 5432,
            user: "me".into(),
            password: password.map(Into::into),
            database: "app".into(),
            ssl: SslMode::Prefer,
        }),
    }
}

#[test]
fn empty_when_file_missing() {
    let (store, _) = temp_store();
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn save_assigns_id_and_never_writes_the_password() {
    let (store, path) = temp_store();
    let saved = store.save(pg(Some("hunter2"))).unwrap();

    assert!(!saved.id.is_empty());
    assert!(!std::fs::read_to_string(path).unwrap().contains("hunter2"));
    assert_eq!(store.get(&saved.id).unwrap(), saved);
}

#[test]
fn save_with_existing_id_replaces() {
    let (store, _) = temp_store();
    let mut saved = store.save(pg(None)).unwrap();
    saved.name = "renamed".into();
    store.save(saved.clone()).unwrap();

    assert_eq!(store.list().unwrap(), [saved]);
}

#[test]
fn remove_deletes_by_id() {
    let (store, _) = temp_store();
    let a = store.save(pg(None)).unwrap();
    let b = store.save(pg(None)).unwrap();
    store.remove(&a.id).unwrap();

    assert_eq!(store.list().unwrap(), [b]);
}

#[test]
fn opening_the_same_sqlite_file_twice_reuses_the_entry() {
    let (store, _) = temp_store();
    let path = std::path::Path::new("/tmp/music.db");
    let first = store.sqlite_file(path).unwrap();
    let second = store.sqlite_file(path).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.name, "music.db");
    assert_eq!(store.list().unwrap().len(), 1);
}
