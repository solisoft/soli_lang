//! Package (soli.toml) parsing and discovery tests.

use std::fs;

use solilang::module::{Dependency, Package};

#[test]
fn parse_minimal_package() {
    let toml = r#"
[package]
name = "myapp"
version = "1.0.0"
"#;
    let pkg = Package::parse(toml).expect("parse");
    assert_eq!(pkg.name, "myapp");
    assert_eq!(pkg.version, "1.0.0");
    // `main` is unset when not given — comes back as the default ("").
    // Tests below set it explicitly to verify the field is wired.
}

#[test]
fn parse_with_description_and_main() {
    let toml = r#"
[package]
name = "tool"
version = "0.1.0"
description = "A useful tool"
main = "src/index.sl"
"#;
    let pkg = Package::parse(toml).expect("parse");
    assert_eq!(pkg.description.as_deref(), Some("A useful tool"));
    assert_eq!(pkg.main, "src/index.sl");
}

#[test]
fn parse_dependencies_path() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
local_lib = "./libs/local"
"#;
    let pkg = Package::parse(toml).expect("parse");
    assert_eq!(pkg.dependencies.len(), 1);
    match &pkg.dependencies["local_lib"] {
        Dependency::Path(p) => assert_eq!(p, "./libs/local"),
        other => panic!("expected Path, got {:?}", other),
    }
}

#[test]
fn parse_dependencies_version() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
math = "1.2.3"
"#;
    let pkg = Package::parse(toml).expect("parse");
    match &pkg.dependencies["math"] {
        Dependency::Version(v) => assert_eq!(v, "1.2.3"),
        other => panic!("expected Version, got {:?}", other),
    }
}

#[test]
fn parse_dependencies_inline_git() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
math = { git = "https://example.com/m.git", tag = "v1.0" }
"#;
    let pkg = Package::parse(toml).expect("parse");
    match &pkg.dependencies["math"] {
        Dependency::Git {
            url,
            tag,
            branch,
            rev,
        } => {
            assert_eq!(url, "https://example.com/m.git");
            assert_eq!(tag.as_deref(), Some("v1.0"));
            assert!(branch.is_none());
            assert!(rev.is_none());
        }
        other => panic!("expected Git, got {:?}", other),
    }
}

#[test]
fn parse_dependencies_inline_branch_and_rev() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
util = { git = "https://example.com/u.git", branch = "develop" }
fixed = { git = "https://example.com/f.git", rev = "abc1234" }
"#;
    let pkg = Package::parse(toml).expect("parse");
    match &pkg.dependencies["util"] {
        Dependency::Git { branch, .. } => assert_eq!(branch.as_deref(), Some("develop")),
        other => panic!("expected Git branch, got {:?}", other),
    }
    match &pkg.dependencies["fixed"] {
        Dependency::Git { rev, .. } => assert_eq!(rev.as_deref(), Some("abc1234")),
        other => panic!("expected Git rev, got {:?}", other),
    }
}

#[test]
fn parse_dependencies_inline_path() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
local = { path = "../shared" }
"#;
    let pkg = Package::parse(toml).expect("parse");
    match &pkg.dependencies["local"] {
        Dependency::Path(p) => assert_eq!(p, "../shared"),
        other => panic!("expected Path, got {:?}", other),
    }
}

#[test]
fn parse_skips_comments_and_blanks() {
    let toml = r#"
# This is a comment

[package]
# inline comment line
name = "app"
version = "0.0.1"
"#;
    let pkg = Package::parse(toml).expect("parse");
    assert_eq!(pkg.name, "app");
}

#[test]
fn parse_rejects_unknown_section() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[bogus]
key = "value"
"#;
    let result = Package::parse(toml);
    assert!(result.is_err(), "unknown section should error");
}

#[test]
fn parse_rejects_unknown_package_field() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"
weird_field = "x"
"#;
    let result = Package::parse(toml);
    assert!(result.is_err(), "unknown package field should error");
}

#[test]
fn parse_rejects_missing_name() {
    let toml = r#"
[package]
version = "1.0.0"
"#;
    let result = Package::parse(toml);
    assert!(result.is_err(), "missing name should error");
}

#[test]
fn parse_rejects_invalid_dependency() {
    let toml = r#"
[package]
name = "app"
version = "0.0.1"

[dependencies]
bad = { foo = "bar" }
"#;
    let result = Package::parse(toml);
    assert!(result.is_err(), "invalid dependency should error");
}

#[test]
fn load_from_file_works() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("soli.toml");
    fs::write(
        &path,
        r#"
[package]
name = "from_disk"
version = "2.0.0"
"#,
    )
    .unwrap();
    let pkg = Package::load(&path).expect("load");
    assert_eq!(pkg.name, "from_disk");
    assert_eq!(pkg.version, "2.0.0");
}

#[test]
fn find_walks_up_to_root() {
    let tmp = tempfile::tempdir().unwrap();
    let nested = tmp.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        tmp.path().join("soli.toml"),
        r#"[package]
name = "found"
version = "1"
"#,
    )
    .unwrap();

    let result = Package::find(&nested);
    assert!(result.is_some(), "find should walk up the tree");
    let path = result.unwrap();
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("soli.toml"));
}

#[test]
fn find_returns_none_when_no_toml() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(Package::find(tmp.path()).is_none());
}

#[test]
fn package_new_uses_defaults() {
    let pkg = Package::new("hello");
    assert_eq!(pkg.name, "hello");
    assert!(!pkg.version.is_empty());
    assert!(pkg.dependencies.is_empty());
}
