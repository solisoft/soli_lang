//! `soli check` / `type_check_source` integration tests.

use solilang::type_check_source;

#[test]
fn clean_source_has_no_type_errors() {
    let src = "fn add(a: Int, b: Int) -> Int { return a + b; }\nprint(add(1, 2));\n";
    assert!(type_check_source(src, None).is_ok());
}

#[test]
fn type_mismatch_is_reported() {
    let src = "let x: Int = \"nope\";\n";
    let errors = type_check_source(src, None).expect_err("should fail to type-check");
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| e.to_string().contains("expected Int")),
        "expected an Int mismatch, got: {:?}",
        errors.iter().map(|e| e.to_string()).collect::<Vec<_>>()
    );
}

#[test]
fn parse_error_surfaces_as_a_single_error() {
    let errors = type_check_source("fn broken( {\n", None).expect_err("should fail to parse");
    assert_eq!(errors.len(), 1);
}

#[test]
fn enum_declaration_and_usage_typechecks() {
    let src = r#"
enum Status { Active, Archived, Pending(reason: String) }
fn describe(s: Status) -> String {
  return match s {
    Status.Active => "a",
    Status.Archived => "x",
    Status.Pending(r) => "waiting " + r,
  }
}
print(describe(Status.Active))
"#;
    let warnings = type_check_source(src, None).expect("enum should type-check");
    assert!(
        warnings.is_empty(),
        "exhaustive match should not warn, got: {:?}",
        warnings
    );
}

#[test]
fn non_exhaustive_enum_match_warns_but_does_not_fail() {
    let src = r#"
enum Status { Active, Archived, Pending(reason: String) }
fn describe(s: Status) -> String {
  return match s {
    Status.Active => "a",
    Status.Pending(r) => "waiting " + r,
  }
}
print(describe(Status.Active))
"#;
    // Non-exhaustive is a non-blocking warning: the check succeeds (Ok) and the
    // warning names the missing variant.
    let warnings = type_check_source(src, None).expect("non-exhaustive match must not fail");
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("not exhaustive") && w.contains("Archived")),
        "expected an exhaustiveness warning naming Archived, got: {:?}",
        warnings
    );
}

#[test]
fn enum_match_with_wildcard_is_exhaustive() {
    let src = r#"
enum Status { Active, Archived, Pending(reason: String) }
fn describe(s: Status) -> String {
  return match s {
    Status.Active => "a",
    _ => "other",
  }
}
print(describe(Status.Active))
"#;
    let warnings = type_check_source(src, None).expect("should type-check");
    assert!(
        warnings.is_empty(),
        "a `_` arm makes the match exhaustive, got: {:?}",
        warnings
    );
}
