//! `soli check` / `type_check_source` integration tests.

use solilang::{type_check_source, type_check_source_with_ambient};

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

/// `render` and `redirect` are registered at run time and deliberately absent
/// from the type environment: they only mean anything inside a request, and
/// `serve` does not type-check. Teaching the checker about them would make
/// `soli check` accept a controller-shaped call in a standalone script that
/// cannot run it.
///
/// This is the test `tests/builtin_registration_baseline.txt` names for that
/// decision. Before it existed the baseline cited a test that did not.
#[test]
fn render_and_redirect_stay_unknown_to_the_checker() {
    for name in ["render", "redirect"] {
        let src = format!("def index(req) {{ return {name}(\"x\", {{}}) }}");
        let errors = type_check_source(&src, None)
            .expect_err("a controller helper must not type-check outside a request");
        assert!(
            errors.iter().any(|e| e.to_string().contains(name)),
            "expected an error naming `{name}`, got: {errors:?}"
        );
    }
}

/// A helper declared in a neighbouring file is callable with no import, because
/// the server loads the auto-loaded directories into one environment. The
/// checker sees one file at a time, so it has to be told.
#[test]
fn ambient_names_make_a_neighbours_helper_resolvable() {
    let src = "def view() { return card({}, []) }\nprint(view())";

    let errors = type_check_source(src, None).expect_err("without ambient names `card` is unknown");
    assert!(
        errors.iter().any(|e| e.to_string().contains("card")),
        "expected `card` to be undefined, got: {errors:?}"
    );

    type_check_source_with_ambient(src, None, &["card".to_string()])
        .expect("with `card` declared by a neighbour it must check");
}

/// A project declaration wins over a builtin of the same name, because that is
/// what happens at run time. The scaffolded EUI catalogue defines its own
/// three-argument `input`; the one-argument builtin was rejecting every call.
#[test]
fn a_project_declaration_overrides_a_builtin_of_the_same_name() {
    let src = "def view() { return input(\"a\", \"b\", {}) }\nprint(view())";

    assert!(
        type_check_source(src, None).is_err(),
        "the one-argument builtin should reject a three-argument call"
    );

    type_check_source_with_ambient(src, None, &["input".to_string()])
        .expect("a project-declared `input` must override the builtin");
}
