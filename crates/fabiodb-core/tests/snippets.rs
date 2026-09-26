use fabiodb_core::{Snippet, Snippets};

fn temp() -> Snippets {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("fabio-snippets-{}-{n}", std::process::id())).join("snippets.json");
    let _ = std::fs::remove_file(&path);
    Snippets::new(path)
}

fn snippet(name: &str, sql: &str) -> Snippet {
    Snippet { id: String::new(), name: name.into(), sql: sql.into() }
}

#[test]
fn saves_lists_by_name_and_removes() {
    let snippets = temp();
    let b = snippets.save(snippet("slow tracks", "select 2")).unwrap();
    let a = snippets.save(snippet("Artists", "select 1")).unwrap();
    assert!(!a.id.is_empty() && a.id != b.id);

    let names: Vec<_> = snippets.list().unwrap().into_iter().map(|s| s.name).collect();
    assert_eq!(names, ["Artists", "slow tracks"]);

    snippets.remove(&a.id).unwrap();
    assert_eq!(snippets.list().unwrap(), [b]);
}

#[test]
fn saving_with_an_id_replaces() {
    let snippets = temp();
    let mut s = snippets.save(snippet("x", "select 1")).unwrap();
    s.sql = "select 42".into();
    snippets.save(s.clone()).unwrap();
    assert_eq!(snippets.list().unwrap(), [s]);
}

#[test]
fn empty_names_are_refused() {
    assert!(temp().save(snippet("  ", "select 1")).is_err());
}
