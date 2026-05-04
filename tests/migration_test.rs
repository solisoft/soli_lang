//! Migration module tests — exercises the parts that don't need a live
//! SoliDB (filename parsing, file generation, name sanitization).

use std::path::PathBuf;

use solilang::migration::{generate_migration, Migration};

#[test]
fn from_path_parses_well_formed_filename() {
    let p = PathBuf::from("db/migrations/20260101120000_create_users.sl");
    let m = Migration::from_path(&p).expect("should parse");
    assert_eq!(m.version, "20260101120000");
    assert_eq!(m.name, "create_users");
    assert_eq!(m.full_name(), "20260101120000_create_users");
}

#[test]
fn from_path_rejects_no_underscore() {
    let p = PathBuf::from("db/migrations/20260101120000.sl");
    assert!(Migration::from_path(&p).is_none());
}

#[test]
fn from_path_rejects_non_numeric_version() {
    let p = PathBuf::from("db/migrations/oops_create_users.sl");
    assert!(Migration::from_path(&p).is_none());
}

#[test]
fn from_path_handles_multi_underscore_name() {
    let p = PathBuf::from("db/migrations/20260101120000_add_email_index.sl");
    let m = Migration::from_path(&p).expect("parse");
    assert_eq!(m.version, "20260101120000");
    // splitn(2, '_') keeps everything after first underscore intact
    assert_eq!(m.name, "add_email_index");
}

#[test]
fn generate_migration_writes_template_and_returns_path() {
    let tmp = tempfile::tempdir().unwrap();
    let path = generate_migration(tmp.path(), "create_users").expect("generate");

    assert!(path.exists(), "migration file not created");
    assert!(path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .ends_with("_create_users.sl"));

    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("fn up(db: Any)"));
    assert!(contents.contains("fn down(db: Any)"));
    assert!(contents.contains("// Migration: create_users"));
}

#[test]
fn generate_migration_sanitizes_name() {
    let tmp = tempfile::tempdir().unwrap();
    // Spaces and dashes get replaced with underscores
    let path =
        generate_migration(tmp.path(), "add my-cool index!").expect("generate with weird name");
    let stem = path.file_stem().unwrap().to_str().unwrap();
    assert!(
        stem.contains("add_my_cool_index_"),
        "name not sanitized: {}",
        stem
    );
}

#[test]
fn generate_migration_creates_db_migrations_dir_if_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // Don't pre-create db/migrations — generate_migration should mkdir -p.
    assert!(!tmp.path().join("db/migrations").exists());
    generate_migration(tmp.path(), "init").expect("generate");
    assert!(tmp.path().join("db/migrations").is_dir());
}

#[test]
fn migrations_with_consecutive_timestamps_sort_lexicographically() {
    let m1 = Migration::from_path(&PathBuf::from("db/migrations/20260101120000_a.sl")).unwrap();
    let m2 = Migration::from_path(&PathBuf::from("db/migrations/20260101120100_b.sl")).unwrap();
    let m3 = Migration::from_path(&PathBuf::from("db/migrations/20260102000000_c.sl")).unwrap();

    let mut versions = vec![&m3.version, &m1.version, &m2.version];
    versions.sort();
    assert_eq!(
        versions,
        vec![&m1.version, &m2.version, &m3.version],
        "string-sortable timestamps should produce chronological order"
    );
}
