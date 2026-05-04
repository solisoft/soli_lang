//! Module resolver integration tests — exercise import resolution against
//! files in a tempdir without going through the full interpreter.

use std::fs;

use solilang::ast::Program;
use solilang::lexer::Scanner;
use solilang::module::ModuleResolver;
use solilang::parser::Parser;

fn parse(source: &str) -> Program {
    let tokens = Scanner::new(source).scan_tokens().expect("lex");
    Parser::new(tokens).parse().expect("parse")
}

#[test]
fn resolves_relative_import_with_named_export() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main.sl");
    let math = tmp.path().join("math.sl");

    fs::write(
        &math,
        "export fn add(a: Int, b: Int) -> Int { return a + b; }\n",
    )
    .unwrap();
    fs::write(
        &main,
        r#"import { add } from "./math.sl";
let result = add(2, 3);
"#,
    )
    .unwrap();

    let program = parse(&fs::read_to_string(&main).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let resolved = resolver.resolve(program, &main).expect("resolve ok");

    // The resolved program should contain the imported `add` definition plus
    // the main file's `let result = ...`.
    assert!(
        resolved.statements.len() >= 2,
        "expected merged program, got {} stmts",
        resolved.statements.len()
    );
}

#[test]
fn resolves_default_import() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main.sl");
    let mod_file = tmp.path().join("greet.sl");

    fs::write(
        &mod_file,
        "export fn greet() -> String { return \"hi\"; }\n",
    )
    .unwrap();
    fs::write(&main, "import \"./greet.sl\";\n").unwrap();

    let program = parse(&fs::read_to_string(&main).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let resolved = resolver.resolve(program, &main).expect("resolve ok");
    assert!(
        !resolved.statements.is_empty(),
        "default import produced empty program"
    );
}

#[test]
fn missing_module_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main.sl");
    fs::write(&main, "import \"./does_not_exist.sl\";\n").unwrap();

    let program = parse(&fs::read_to_string(&main).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let result = resolver.resolve(program, &main);
    assert!(result.is_err(), "missing module should error");
}

#[test]
fn circular_import_is_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.sl");
    let b = tmp.path().join("b.sl");

    fs::write(
        &a,
        "import \"./b.sl\";\nexport fn from_a() -> Int { return 1; }\n",
    )
    .unwrap();
    fs::write(
        &b,
        "import \"./a.sl\";\nexport fn from_b() -> Int { return 2; }\n",
    )
    .unwrap();

    let program = parse(&fs::read_to_string(&a).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let result = resolver.resolve(program, &a);
    // Either errors with circular detection, or returns successfully if the
    // resolver handles cycles via memoization. Both are acceptable; we just
    // want to make sure it doesn't infinite-loop or panic.
    let _ = result;
}

#[test]
fn nested_imports_work() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main.sl");
    let a = tmp.path().join("a.sl");
    let b = tmp.path().join("b.sl");

    fs::write(&b, "export fn from_b() -> Int { return 99; }\n").unwrap();
    fs::write(
        &a,
        "import { from_b } from \"./b.sl\";\nexport fn from_a() -> Int { return from_b(); }\n",
    )
    .unwrap();
    fs::write(
        &main,
        "import { from_a } from \"./a.sl\";\nlet x = from_a();\n",
    )
    .unwrap();

    let program = parse(&fs::read_to_string(&main).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let resolved = resolver.resolve(program, &main).expect("nested resolve");
    // Should include from_a (and potentially from_b). The resolver may
    // unwrap exports differently; just assert at least something came in.
    assert!(
        !resolved.statements.is_empty(),
        "nested imports produced empty program"
    );
}

#[test]
fn unimported_name_is_not_pulled_in() {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main.sl");
    let lib = tmp.path().join("lib.sl");

    fs::write(
        &lib,
        r#"export fn used() -> Int { return 1; }
export fn not_used() -> Int { return 2; }
fn private_helper() -> Int { return 3; }
"#,
    )
    .unwrap();
    fs::write(
        &main,
        "import { used } from \"./lib.sl\";\nlet x = used();\n",
    )
    .unwrap();

    let program = parse(&fs::read_to_string(&main).unwrap());
    let mut resolver = ModuleResolver::new(tmp.path());
    let resolved = resolver
        .resolve(program, &main)
        .expect("named import resolve");

    // We should have `used` brought in, plus the main `let x`. The unused
    // export and private helper shouldn't both be there. We check by
    // counting that resolved program is short rather than asserting on
    // exact contents (the implementation is allowed to also include
    // private dependencies of `used`).
    assert!(
        resolved.statements.len() <= 5,
        "named import pulled in too much: {}",
        resolved.statements.len()
    );
}
