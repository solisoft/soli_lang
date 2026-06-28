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
