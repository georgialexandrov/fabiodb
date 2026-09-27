use std::path::Path;

use fabiodb_core::{Layout, Link, Links, layout_path};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fabio-diagram-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn the_layout_sits_next_to_the_model() {
    assert_eq!(layout_path(Path::new("/p/db/schema.dbml")), Path::new("/p/db/schema.layout.json"));
}

#[test]
fn a_layout_is_one_table_per_line_and_reads_back() {
    let path = scratch("layout").join("schema.layout.json");
    assert_eq!(Layout::read(&path).unwrap(), Layout::default());

    let mut layout = Layout::default();
    layout.tables.insert("track".into(), [420, 80]);
    layout.tables.insert("album".into(), [40, -12]);
    layout.tables.insert("perf.big".into(), [0, 0]);
    layout.write(&path).unwrap();

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\n  \"tables\": {\n    \"album\": [40, -12],\n    \"perf.big\": [0, 0],\n    \"track\": [420, 80]\n  }\n}\n"
    );
    assert_eq!(Layout::read(&path).unwrap(), layout);
}

#[test]
fn links_are_remembered_per_workspace() {
    let links = Links::new(scratch("links").join("links.json"));
    assert_eq!(links.get("pg").unwrap(), None);

    let link = Link { dbml: "/p/schema.dbml".into(), bookmark: Some("00ff".into()) };
    links.set("pg", link.clone()).unwrap();
    links.set("pg#other", Link { dbml: "/q/other.dbml".into(), bookmark: None }).unwrap();
    assert_eq!(links.get("pg").unwrap(), Some(link));

    links.remove("pg").unwrap();
    assert_eq!(links.get("pg").unwrap(), None);
    assert!(links.get("pg#other").unwrap().is_some());
}
