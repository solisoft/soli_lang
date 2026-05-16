//! Linter integration tests — run the linter against carefully crafted Soli
//! sources to exercise `lint/expressions.rs`, `lint/statements.rs`, and the
//! scope/style/smell/naming rules end-to-end.

use solilang::lexer::Scanner;
use solilang::lint::{LintDiagnostic, Linter};
use solilang::parser::Parser;
use std::path::Path;
use tempfile;

fn lint(source: &str) -> Vec<LintDiagnostic> {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer ok");
    let program = Parser::new(tokens).parse().expect("parser ok");
    Linter::new(source).lint(&program)
}

fn lint_with_path(source: &str, path: &Path) -> Vec<LintDiagnostic> {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer ok");
    let program = Parser::new(tokens).parse().expect("parser ok");
    Linter::new(source)
        .with_file_path(path.to_string_lossy().as_ref())
        .lint(&program)
}

fn rules(diagnostics: &[LintDiagnostic]) -> Vec<&'static str> {
    diagnostics.iter().map(|d| d.rule).collect()
}

#[test]
fn clean_code_produces_no_diagnostics() {
    let src = r#"
fn add(a: Int, b: Int) -> Int {
    return a + b;
}
let result = add(2, 3);
"#;
    let diags = lint(src);
    assert!(diags.is_empty(), "unexpected diagnostics: {:?}", diags);
}

#[test]
fn snake_case_violation_is_flagged() {
    let src = r#"
let myVar = 42;
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"naming/snake-case"),
        "expected naming/snake-case, got: {:?}",
        names
    );
}

#[test]
fn pascal_case_violation_is_flagged() {
    let src = r#"
class my_class {
    fn foo() -> Int { return 1; }
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"naming/pascal-case"),
        "expected naming/pascal-case, got: {:?}",
        names
    );
}

#[test]
fn long_line_is_flagged() {
    let mut src = String::from("let x = \"");
    src.push_str(&"a".repeat(140));
    src.push_str("\";\n");
    let diags = lint(&src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"style/line-length"),
        "expected style/line-length, got: {:?}",
        names
    );
}

#[test]
fn unreachable_code_after_return_is_flagged() {
    let src = r#"
fn foo() -> Int {
    return 1;
    let unreachable = 2;
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/unreachable-code"),
        "expected smell/unreachable-code, got: {:?}",
        names
    );
}

#[test]
fn empty_catch_is_flagged() {
    let src = r#"
fn foo() {
    try {
        let x = 1;
    } catch (e) {
    }
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/empty-catch"),
        "expected smell/empty-catch, got: {:?}",
        names
    );
}

#[test]
fn deep_nesting_is_flagged() {
    let src = r#"
fn deep() {
    if (true) {
        if (true) {
            if (true) {
                if (true) {
                    if (true) {
                        let x = 1;
                    }
                }
            }
        }
    }
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/deep-nesting"),
        "expected smell/deep-nesting, got: {:?}",
        names
    );
}

#[test]
fn duplicate_methods_in_class_flagged() {
    let src = r#"
class Foo {
    fn bar() -> Int { return 1; }
    fn bar() -> Int { return 2; }
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/duplicate-methods"),
        "expected smell/duplicate-methods, got: {:?}",
        names
    );
}

#[test]
fn undefined_local_in_function_flagged() {
    let src = r#"
fn user_fn() -> Int {
    return notdefined + 1;
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/undefined-local"),
        "expected smell/undefined-local, got: {:?}",
        names
    );
}

#[test]
fn defined_local_does_not_trigger_undefined_local() {
    let src = r#"
fn ok_fn() -> Int {
    let x = 5;
    return x + 1;
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        !names.contains(&"smell/undefined-local"),
        "false-positive on defined local: {:?}",
        names
    );
}

#[test]
fn uses_of_outer_let_dont_trigger_undefined_local() {
    let src = r#"
let global_count = 0;
fn use_global() -> Int {
    return global_count + 1;
}
"#;
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        !names.contains(&"smell/undefined-local"),
        "false-positive on top-level let: {:?}",
        names
    );
}

#[test]
fn linter_reports_span_position() {
    let src = "let badName = 1;\n";
    let diags = lint(src);
    let snake_diag = diags
        .iter()
        .find(|d| d.rule == "naming/snake-case")
        .expect("snake-case diag");
    assert_eq!(snake_diag.span.line, 1);
}

#[test]
fn diagnostics_sorted_by_position() {
    // Two violations on different lines — output must come back in line order.
    let src = "let badName = 1;\nlet anotherBad = 2;\n";
    let diags = lint(src);
    let lines: Vec<_> = diags.iter().map(|d| d.span.line).collect();
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(lines, sorted, "diagnostics not sorted by line: {:?}", lines);
}

#[test]
fn redundant_model_import_in_controller_flagged() {
    let src = r#"
import "../models/user.sl";
fn index(req: Any) -> Any {
    return {};
}
"#;
    // Pretend the source is a controller file so the rule applies.
    let tokens = Scanner::new(src).scan_tokens().unwrap();
    let program = Parser::new(tokens).parse().unwrap();
    let diags = Linter::new(src)
        .with_file_path("app/controllers/users_controller.sl")
        .lint(&program);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"style/redundant-model-import"),
        "expected style/redundant-model-import, got: {:?}",
        names
    );
}

/// Build a minimal MVC project layout in a temp directory and return the root.
///
/// Layout:
///   <root>/app/controllers/<controller_file>
///   <root>/app/services/stripe_service.sl   (defines `charge_card`)
///   <root>/app/jobs/mailer_job.sl           (defines `send_welcome_email`)
///   <root>/app/helpers/format_helper.sl     (defines `format_price`)
fn make_mvc_tree(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let root = tmp.path().to_path_buf();
    for dir in &[
        "app/controllers",
        "app/services",
        "app/jobs",
        "app/helpers",
        "app/models",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(
        root.join("app/services/stripe_service.sl"),
        "fn charge_card(amount: Int) -> Hash { return {}; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/jobs/mailer_job.sl"),
        "let send_welcome_email = fn(user) { print(user); };\n",
    )
    .unwrap();
    std::fs::write(
        root.join("app/helpers/format_helper.sl"),
        "fn format_price(cents: Int) -> String { return str(cents); }\n",
    )
    .unwrap();
    root
}

#[test]
fn controller_calling_service_fn_no_undefined_local() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = make_mvc_tree(&tmp);

    let src = r#"
fn index(req: Any) -> Any {
    let result = charge_card(100);
    return render("index", result);
}
"#;
    let controller_path = root.join("app/controllers/payments_controller.sl");
    std::fs::write(&controller_path, src).unwrap();

    let diags = lint_with_path(src, &controller_path);
    let names: Vec<_> = rules(&diags);
    assert!(
        !names.contains(&"smell/undefined-local"),
        "false positive: charge_card is defined in app/services/ and must be visible: {:?}",
        names
    );
}

#[test]
fn controller_referencing_job_let_no_undefined_local() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = make_mvc_tree(&tmp);

    let src = r#"
fn create(req: Any) -> Any {
    send_welcome_email(req["json"]["user"]);
    return redirect("/");
}
"#;
    let controller_path = root.join("app/controllers/users_controller.sl");
    std::fs::write(&controller_path, src).unwrap();

    let diags = lint_with_path(src, &controller_path);
    let names: Vec<_> = rules(&diags);
    assert!(
        !names.contains(&"smell/undefined-local"),
        "false positive: send_welcome_email is a top-level let in app/jobs/ and must be visible: {:?}",
        names
    );
}

#[test]
fn model_referencing_helper_fn_no_undefined_local() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = make_mvc_tree(&tmp);

    let src = r#"
fn display_price(cents: Int) -> String {
    return format_price(cents);
}
"#;
    let model_path = root.join("app/models/product.sl");
    std::fs::write(&model_path, src).unwrap();

    let diags = lint_with_path(src, &model_path);
    let names: Vec<_> = rules(&diags);
    assert!(
        !names.contains(&"smell/undefined-local"),
        "false positive: format_price is defined in app/helpers/ and must be visible from app/models/: {:?}",
        names
    );
}

#[test]
fn standalone_script_outside_app_tree_still_flags_undefined() {
    let src = r#"
fn broken() -> Int {
    return ghost_variable + 1;
}
"#;
    // No file path → no auto-load scanning → undefined reference must still be flagged.
    let diags = lint(src);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/undefined-local"),
        "expected smell/undefined-local for truly undefined name in standalone script: {:?}",
        names
    );
}

#[test]
fn standalone_script_with_path_outside_app_still_flags_undefined() {
    let tmp = tempfile::TempDir::new().unwrap();
    let script_path = tmp.path().join("my_script.sl");
    let src = r#"
fn broken() -> Int {
    return ghost_variable + 1;
}
"#;
    std::fs::write(&script_path, src).unwrap();

    let diags = lint_with_path(src, &script_path);
    let names: Vec<_> = rules(&diags);
    assert!(
        names.contains(&"smell/undefined-local"),
        "standalone script must still flag truly undefined names even with a file path: {:?}",
        names
    );
}
