//! Code frames and error codes on the command line (#694).
//!
//! `soli run`, `soli -e`, `soli check` and `soli lint --frames` print the
//! line they always printed, then a frame: the file and position, the source
//! line, an underline, the "did you mean" hint and the error code. The first
//! line is unchanged, so anything that greps it keeps working.

use std::path::Path;
use std::process::{Command, Output};

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn soli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
        .output()
        .expect("run soli")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn a_runtime_error_is_framed_the_same_on_both_engines() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.sl", "name = \"Alice\"\nprint(nme)\n");
    for engine in ["--vm", "--tree"] {
        let output = soli(dir.path(), &[engine, "--no-type-check", "a.sl"]);
        let text = stderr(&output);
        assert_eq!(output.status.code(), Some(70), "{engine}: {text}");
        assert!(
            text.starts_with("Error: Runtime error: Undefined variable 'nme'"),
            "{engine}: {text}"
        );
        assert!(text.contains(" --> a.sl:2:7\n"), "{engine}: {text}");
        assert!(text.contains("2 | print(nme)\n"), "{engine}: {text}");
        assert!(text.contains("  |       ^^^\n"), "{engine}: {text}");
        assert!(
            text.contains("= note: error[E0301] undefined-variable — `soli explain E0301`"),
            "{engine}: {text}"
        );
        // Piped, not a terminal: no colour.
        assert!(!text.contains('\x1b'), "{engine}: {text}");
    }
}

#[test]
fn the_type_checker_error_of_soli_run_carries_the_hint() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.sl", "name = \"Alice\"\nprint(nme)\n");
    let output = soli(dir.path(), &["a.sl"]);
    let text = stderr(&output);
    assert!(
        text.starts_with(
            "Error: Type error: Undefined variable 'nme' (did you mean 'name'?) at 2:7\n"
        ),
        "{text}"
    );
    assert!(text.contains("= help: did you mean 'name'?"), "{text}");
}

#[test]
fn an_eval_error_is_framed_from_the_code_given() {
    let dir = tempfile::tempdir().unwrap();
    let output = soli(
        dir.path(),
        &["--no-type-check", "-e", "x = 1\nprint(x / 0)"],
    );
    let text = stderr(&output);
    assert_eq!(output.status.code(), Some(70), "{text}");
    assert!(text.contains(" --> <eval>:2:7\n"), "{text}");
    assert!(text.contains("2 | print(x / 0)\n"), "{text}");
    assert!(text.contains("error[E0501] division-by-zero"), "{text}");
}

#[test]
fn an_error_inside_an_imported_module_frames_that_module() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "lib/helper.sl",
        "export def helper(x)\n  x + undefined_thing\nend\n",
    );
    write(
        dir.path(),
        "main.sl",
        "import \"./lib/helper.sl\"\n\nprint(1)\nhelper(2)\n",
    );
    for engine in ["--vm", "--tree"] {
        let output = soli(dir.path(), &[engine, "--no-type-check", "main.sl"]);
        let text = stderr(&output);
        assert!(
            text.contains(" --> lib/helper.sl:2:7\n"),
            "{engine}: {text}"
        );
        assert!(
            text.contains("2 |   x + undefined_thing\n"),
            "{engine}: {text}"
        );
    }
}

#[test]
fn an_error_in_the_entry_file_of_a_program_with_imports_frames_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "lib/helper.sl",
        "export def helper(x)\n  x\nend\n",
    );
    write(
        dir.path(),
        "main.sl",
        "import \"./lib/helper.sl\"\nhelper(1)\nprint(10 / 0)\n",
    );
    for engine in ["--vm", "--tree"] {
        let output = soli(dir.path(), &[engine, "--no-type-check", "main.sl"]);
        let text = stderr(&output);
        assert!(text.contains(" --> main.sl:3:7\n"), "{engine}: {text}");
        assert!(text.contains("3 | print(10 / 0)\n"), "{engine}: {text}");
    }
}

#[test]
fn a_missing_import_points_at_the_import() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "main.sl",
        "# entry\nimport \"./lib/missing.sl\"\n",
    );
    let output = soli(dir.path(), &["main.sl"]);
    let text = stderr(&output);
    assert!(
        text.starts_with("Error: Runtime error: Module resolution error: Module not found"),
        "{text}"
    );
    assert!(text.contains(" --> main.sl:2:1\n"), "{text}");
    assert!(text.contains("error[E0701] module-not-found"), "{text}");
}

#[test]
fn a_syntax_error_in_an_imported_module_points_into_the_module() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "lib/bad.sl", "export def broken(\n  1\n");
    write(dir.path(), "main.sl", "import \"./lib/bad.sl\"\n");
    let output = soli(dir.path(), &["main.sl"]);
    let text = stderr(&output);
    assert!(text.contains(" --> lib/bad.sl:2:3\n"), "{text}");
    assert!(text.contains("2 |   1\n"), "{text}");
    assert!(text.contains("error[E0704] module-syntax-error"), "{text}");
}

#[test]
fn soli_check_frames_every_error_and_speaks_json() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.sl",
        "name = \"Alice\"\nprint(nme)\ndef add(a: Int, b: Int) -> Int\n  a + b\nend\nadd(\"x\", 2)\nprint(totl)\n",
    );
    let output = soli(dir.path(), &["check", "a.sl"]);
    let text = stdout(&output);
    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("a.sl: Type error: Undefined variable 'nme'"),
        "{text}"
    );
    assert!(text.contains(" --> a.sl:2:7\n"), "{text}");
    assert!(text.contains(" --> a.sl:6:"), "{text}");
    assert!(text.contains(" --> a.sl:7:7\n"), "{text}");
    assert!(text.contains("3 error(s) in 1 of 1 file(s)"), "{text}");

    let output = soli(dir.path(), &["check", "--format", "json", "a.sl"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["errors"], 3);
    let first = &json["diagnostics"][0];
    assert_eq!(first["code"], "E0301");
    assert_eq!(first["file"], "a.sl");
    assert_eq!(first["line"], 2);
    assert_eq!(first["column"], 7);
    assert_eq!(first["message"], "Undefined variable 'nme'");
    assert_eq!(first["hint"], "did you mean 'name'?");
}

#[test]
fn soli_lint_keeps_its_line_and_adds_frames_and_json_on_request() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "l.sl",
        "def f\n  total = 1\n  print(totl)\nend\n",
    );
    let plain = stdout(&soli(dir.path(), &["lint", "l.sl"]));
    assert!(
        plain.contains("l.sl:3:9 - [smell/undefined-local]"),
        "{plain}"
    );
    assert!(!plain.contains(" --> "), "{plain}");

    let framed = stdout(&soli(dir.path(), &["lint", "--frames", "l.sl"]));
    assert!(
        framed.contains("l.sl:3:9 - [smell/undefined-local]"),
        "{framed}"
    );
    assert!(framed.contains(" --> l.sl:3:9\n"), "{framed}");
    assert!(
        framed.contains("`soli explain smell/undefined-local`"),
        "{framed}"
    );

    let output = soli(dir.path(), &["lint", "--format=json", "l.sl"]);
    assert_eq!(output.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON");
    assert_eq!(json["issues"], 1);
    assert_eq!(json["diagnostics"][0]["code"], "smell/undefined-local");
    assert_eq!(json["diagnostics"][0]["line"], 3);
}

#[test]
fn soli_explain_prints_codes_and_rules_and_suggests_on_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let output = soli(dir.path(), &["explain", "E0301"]);
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("# E0301: undefined variable"));

    let output = soli(dir.path(), &["explain", "undefined-variable"]);
    assert!(stdout(&output).starts_with("# E0301: undefined variable"));

    let output = soli(dir.path(), &["explain", "smell/undefined-local"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("smell/undefined-local"));

    let output = soli(dir.path(), &["explain", "E0310"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Did you mean 'E0301'?"),
        "{}",
        stderr(&output)
    );

    let output = soli(dir.path(), &["explain"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("E0701  module-not-found"));
}

#[test]
fn an_error_in_a_module_reached_through_a_block_frames_the_module() {
    // The block runs in a nested VM activation: the innermost one notes the
    // module's file, and the outer one, handing the same error on, keeps it.
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "lib/helper.sl",
        "export def helper(x)\n  x / 0\nend\n",
    );
    write(
        dir.path(),
        "main.sl",
        "import \"./lib/helper.sl\"\n[1, 2].each do |x|\n  helper(x)\nend\n",
    );
    for engine in ["--vm", "--tree"] {
        let output = soli(dir.path(), &[engine, "--no-type-check", "main.sl"]);
        let text = stderr(&output);
        assert_eq!(output.status.code(), Some(70), "{engine}: {text}");
        assert!(text.contains(" --> lib/helper.sl:2:"), "{engine}: {text}");
        assert!(text.contains("2 |   x / 0\n"), "{engine}: {text}");
    }
}

#[test]
fn json_output_stays_json_when_there_is_nothing_to_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("empty")).unwrap();
    for command in ["check", "lint"] {
        let output = soli(dir.path(), &[command, "--format", "json", "empty"]);
        assert!(output.status.success(), "{command}");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|e| panic!("{command}: {e}: {}", stdout(&output)));
        assert_eq!(json["diagnostics"], serde_json::json!([]), "{command}");
    }
}

/// #694: `soli test` draws a failing test's code frame under its line.
#[test]
fn soli_test_frames_a_failing_assertion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tests")).unwrap();
    std::fs::write(dir.path().join(".env.test"), "SOLIDB_DATABASE=frames\n").unwrap();
    std::fs::write(
        dir.path().join("tests/math_spec.sl"),
        "describe(\"math\") do\n  test(\"adds\") do\n    total = 1 + 1\n    assert_eq(total, 3)\n  end\n\n  test(\"typo\") do\n    x = 2\n    assert_eq(y, 2)\n  end\nend\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["test", "tests", "--no-coverage"])
        .current_dir(dir.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("run soli test");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success());
    assert!(text.contains("--> tests/math_spec.sl:4:5"), "{text}");
    assert!(text.contains("4 |     assert_eq(total, 3)"), "{text}");
    assert!(text.contains("--> tests/math_spec.sl:9:15"), "{text}");
    assert!(text.contains("did you mean 'x'?"), "{text}");
    assert!(text.contains("error[E0301]"), "{text}");
}

/// #694, phase 6: a misspelt local, parameter, top-level variable or an
/// enclosing function's local gets the same "did you mean" on the VM as on
/// the tree-walker — the compiler keeps the names bytecode slots lack.
#[test]
fn the_vm_suggests_locals_like_the_tree_walker() {
    let cases = [
        ("def f\n  total = 2\n  print(totl)\nend\nf()\n", "'total'"),
        ("def k(price)\n  print(prise)\nend\nk(1)\n", "'price'"),
        ("items = [1]\nprint(itmes)\n", "'items'"),
        (
            "def g\n  count = 1\n  h = fn() { print(cuont) }\n  h()\nend\ng()\n",
            "'count'",
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    for (source, expected) in cases {
        let file = dir.path().join("typo.sl");
        std::fs::write(&file, source).unwrap();
        let first_line = |engine: &str| {
            let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
                .args([engine, "--no-type-check"])
                .arg(&file)
                .env("NO_COLOR", "1")
                .output()
                .expect("run soli");
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let vm = first_line("--vm");
        assert!(
            vm.contains(&format!("did you mean {expected}?")),
            "{source}: {vm}"
        );
        assert_eq!(vm, first_line("--tree"), "{source}");
    }
}

/// #694, phase 4: a wrong argument count points at the `def` too, and an
/// unknown member at the class — when they are in the file checked; a
/// declaration from another file of the application gets no label (its span
/// would be read against the wrong file).
#[test]
fn a_frame_points_at_the_declaration_too() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("labels.sl");
    std::fs::write(
        &file,
        "class Invoice\n  def total -> Int\n    1\n  end\nend\n\ndef greet(name, greeting = \"Hi\")\n  \"#{greeting} #{name}\"\nend\n\ngreet(\"a\", \"b\", \"c\")\nInvoice.new.totl\n",
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("check")
        .arg(&file)
        .env("NO_COLOR", "1")
        .output()
        .expect("run soli check");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains(":::"), "{text}");
    assert!(
        text.contains(" 7 | def greet(name, greeting = \"Hi\")"),
        "{text}"
    );
    assert!(text.contains("--- declared here"), "{text}");
    assert!(text.contains(" 1 | class Invoice"), "{text}");
    assert!(text.contains("`Invoice` is declared here"), "{text}");

    // `soli run` checks first, and draws the same frame.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(&file)
        .env("NO_COLOR", "1")
        .output()
        .expect("run soli");
    let text = String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("--- declared here"), "{text}");
}
