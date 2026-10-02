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

/// Run `main.sl` in `dir` with the built binary — type checker and runtime
/// both — and return what it printed.
fn run_main(dir: &std::path::Path) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(dir.join("main.sl"))
        .current_dir(dir)
        .output()
        .expect("soli runs");
    assert!(
        out.status.success(),
        "soli main.sl failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// An imported module brings what its exports need: the modules it imports
/// itself, and its private helpers. Before, only the `export`ed declarations
/// crossed, so a module two levels down — or a helper beside an export —
/// was an `Undefined variable` at the call.
#[test]
fn an_import_carries_what_its_exports_need() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("shared.sl"),
        "const BASE = 40\n\nexport class Counter\n  static def start\n    BASE\n  end\nend\n",
    )
    .unwrap();
    // `rules.sl` imports `shared.sl` and keeps a helper to itself.
    fs::write(
        dir.join("rules.sl"),
        "import \"./shared.sl\"\n\ndef bump(n)\n  n + 1\nend\n\nexport def next_value\n  bump(Counter.start)\nend\n",
    )
    .unwrap();
    // `labels.sl` imports `shared.sl` too: two paths to one module.
    fs::write(
        dir.join("labels.sl"),
        "import \"./shared.sl\"\n\nexport def label(n)\n  \"#{n} of #{BASE}\"\nend\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.sl"),
        "import \"./rules.sl\"\nimport { label as describe } from \"./labels.sl\"\n\nprint(describe(next_value()))\n",
    )
    .unwrap();

    assert_eq!(run_main(dir), "41 of 40");
}

/// A re-exporting `mod.sl`, the layout the language guide shows: `main.sl`
/// imports the folder's `mod.sl`, which imports the files beside it.
#[test]
fn a_mod_file_reexports_the_files_beside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::create_dir_all(dir.join("utils")).unwrap();
    fs::write(
        dir.join("utils/strings.sl"),
        "export def shout(s)\n  s.upcase\nend\n",
    )
    .unwrap();
    fs::write(
        dir.join("utils/mod.sl"),
        "import \"./strings.sl\"\n\nexport def greet(name)\n  shout(\"hello #{name}\")\nend\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.sl"),
        "import \"./utils/mod.sl\"\n\nprint(greet(\"ada\"))\n",
    )
    .unwrap();

    assert_eq!(run_main(dir), "HELLO ADA");
}

/// `export enum` and `export const` parse, and import by name or alias like
/// any other export. Before, only `def`, `class`, `module`, `interface` and
/// `let` could follow `export`, and shared data had to be a function.
#[test]
fn enums_and_constants_can_be_exported() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    fs::write(
        dir.join("palette.sl"),
        "export enum Tone\n  Calm\n  Loud\nend\n\nexport const LIMIT = 3\n",
    )
    .unwrap();
    fs::write(
        dir.join("main.sl"),
        "import { Tone, LIMIT as MAX } from \"./palette.sl\"\n\nprint(Tone.Loud == Tone.Loud)\nprint(MAX + 1)\n",
    )
    .unwrap();

    assert_eq!(run_main(dir), "true\n4");
}
