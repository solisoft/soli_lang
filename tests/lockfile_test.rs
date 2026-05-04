//! Lock-file integration tests — exercise file IO + dependency-satisfaction
//! checks for branch/rev paths that the in-module unit tests skip.

use std::path::PathBuf;

use solilang::module::lockfile::{LockEntry, LockFile};
use solilang::module::Dependency;

fn entry(
    name: &str,
    url: &str,
    rev: &str,
    cache: &std::path::Path,
    ref_spec: Option<&str>,
) -> LockEntry {
    LockEntry {
        name: name.to_string(),
        url: url.to_string(),
        resolved_rev: rev.to_string(),
        cache_path: cache.to_path_buf(),
        ref_spec: ref_spec.map(|s| s.to_string()),
    }
}

#[test]
fn load_returns_empty_when_file_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let lock = LockFile::load(&tmp.path().join("missing.lock")).expect("load");
    assert!(lock.packages.is_empty());
}

#[test]
fn save_then_load_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("soli.lock");
    let cache = tmp.path().to_path_buf();

    let mut lock = LockFile::default();
    lock.packages.insert(
        "math".to_string(),
        entry(
            "math",
            "https://example.com/m.git",
            "deadbeef",
            &cache,
            Some("v1.0.0"),
        ),
    );
    lock.packages.insert(
        "util".to_string(),
        entry(
            "util",
            "https://example.com/u.git",
            "cafef00d",
            &cache,
            None,
        ),
    );
    lock.save(&path).expect("save");

    let reloaded = LockFile::load(&path).expect("reload");
    assert_eq!(reloaded.packages.len(), 2);
    assert_eq!(reloaded.packages["math"].resolved_rev, "deadbeef");
    assert_eq!(reloaded.packages["util"].ref_spec, None);
}

#[test]
fn save_output_is_sorted_for_determinism() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("soli.lock");
    let cache = tmp.path().to_path_buf();

    let mut lock = LockFile::default();
    // Insert in non-alphabetical order
    for n in ["zebra", "alpha", "mango"] {
        lock.packages.insert(
            n.to_string(),
            entry(n, &format!("u-{}", n), "rev", &cache, None),
        );
    }
    lock.save(&path).expect("save");

    let content = std::fs::read_to_string(&path).expect("read");
    let alpha_pos = content.find("alpha").unwrap();
    let mango_pos = content.find("mango").unwrap();
    let zebra_pos = content.find("zebra").unwrap();
    assert!(alpha_pos < mango_pos && mango_pos < zebra_pos, "not sorted");
}

#[test]
fn load_skips_comments_and_blank_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("soli.lock");
    std::fs::write(
        &path,
        "# header\n\nmath|url|rev|/tmp|v1\n# trailing comment\n\n",
    )
    .unwrap();
    let lock = LockFile::load(&path).expect("load");
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.packages["math"].resolved_rev, "rev");
}

#[test]
fn load_returns_err_on_malformed_line() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("soli.lock");
    std::fs::write(&path, "math|url|onlyone\n").unwrap();
    let result = LockFile::load(&path);
    assert!(result.is_err(), "malformed line should error");
}

#[test]
fn is_satisfied_branch_match() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lock = LockFile::default();
    lock.packages.insert(
        "math".to_string(),
        entry(
            "math",
            "https://example.com/m.git",
            "abc",
            tmp.path(),
            Some("main"),
        ),
    );

    let dep = Dependency::Git {
        url: "https://example.com/m.git".to_string(),
        tag: None,
        branch: Some("main".to_string()),
        rev: None,
    };
    assert!(lock.is_satisfied("math", &dep));
}

#[test]
fn is_satisfied_branch_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lock = LockFile::default();
    lock.packages.insert(
        "math".to_string(),
        entry(
            "math",
            "https://example.com/m.git",
            "abc",
            tmp.path(),
            Some("main"),
        ),
    );
    let dep = Dependency::Git {
        url: "https://example.com/m.git".to_string(),
        tag: None,
        branch: Some("dev".to_string()),
        rev: None,
    };
    assert!(!lock.is_satisfied("math", &dep));
}

#[test]
fn is_satisfied_rev_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let mut lock = LockFile::default();
    lock.packages.insert(
        "math".to_string(),
        entry(
            "math",
            "https://example.com/m.git",
            "abcdef0123456789",
            tmp.path(),
            None,
        ),
    );
    let dep = Dependency::Git {
        url: "https://example.com/m.git".to_string(),
        tag: None,
        branch: None,
        rev: Some("abcdef".to_string()),
    };
    assert!(lock.is_satisfied("math", &dep));
}

#[test]
fn is_satisfied_returns_false_when_cache_missing() {
    let mut lock = LockFile::default();
    lock.packages.insert(
        "math".to_string(),
        entry(
            "math",
            "https://example.com/m.git",
            "abc",
            &PathBuf::from("/nonexistent/path/that/should/not/exist"),
            Some("v1"),
        ),
    );
    let dep = Dependency::Git {
        url: "https://example.com/m.git".to_string(),
        tag: Some("v1".to_string()),
        branch: None,
        rev: None,
    };
    assert!(!lock.is_satisfied("math", &dep));
}

#[test]
fn is_satisfied_unknown_package_returns_false() {
    let lock = LockFile::default();
    let dep = Dependency::Git {
        url: "u".to_string(),
        tag: None,
        branch: None,
        rev: None,
    };
    assert!(!lock.is_satisfied("unknown", &dep));
}
