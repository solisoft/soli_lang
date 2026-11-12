//! Union types: `Int | String | nil` (#693, phase 4).
//!
//! Written in a parameter, a `let`, a field or a return type, a union admits a
//! value of any one of its members — in the checker, and in the return-type
//! check both engines run. Inside `|x: T|` block parameters the closing `|` is
//! not a union: there one is written in parentheses.

use std::path::Path;
use std::process::Command;

fn check(source: &str) -> Result<(), String> {
    solilang::type_check_source(source, None)
        .map(|_| ())
        .map_err(|errors| {
            errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        })
}

fn run(source: &str, engine: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("u.sl");
    std::fs::write(&file, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(engine)
        .arg("--no-type-check")
        .arg(Path::new(&file))
        .output()
        .expect("run soli");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn a_union_parameter_takes_any_of_its_members() {
    let src = "def show(id: Int | String) -> String\n  \"##{id}\"\nend\n";
    check(&format!("{src}show(1)\nshow(\"a\")\n")).unwrap();
    let err = check(&format!("{src}show(1.5)\n")).unwrap_err();
    assert!(err.contains("expected Int | String, found Float"), "{err}");
    let err = check(&format!("{src}show(nil)\n")).unwrap_err();
    assert!(err.contains("found Null"), "{err}");
}

#[test]
fn nil_in_a_union_makes_it_nullable() {
    let src = "def label(count: Int | nil) -> String\n  return \"none\" if count.nil?\n\n  \"#{count}\"\nend\n";
    check(&format!("{src}label(nil)\nlabel(3)\n")).unwrap();
    let err = check(&format!("{src}label(\"3\")\n")).unwrap_err();
    assert!(err.contains("expected Int?"), "{err}");

    // Three members and nil.
    check("def f(x: Int | String | nil)\n  x\nend\nf(nil)\nf(\"s\")\n").unwrap();
    let err = check("def f(x: Int | String | nil)\n  x\nend\nf(1.5)\n").unwrap_err();
    assert!(err.contains("expected Int | String | nil"), "{err}");
}

#[test]
fn a_union_value_goes_only_where_every_member_goes() {
    let src = "def pick(flag: Bool) -> Int | String\n  flag ? 1 : \"one\"\nend\n";
    let err = check(&format!("{src}let n: Int = pick(true)\n")).unwrap_err();
    assert!(err.contains("expected Int, found Int | String"), "{err}");
    check(&format!("{src}let v: Int | String | Float = pick(true)\n")).unwrap();
    // Read and used, a union is not narrowed: no false error on its members.
    check(&format!("{src}print(pick(true).to_s)\n")).unwrap();
}

#[test]
fn block_parameters_close_on_their_pipe() {
    check("xs = [1, 2]\nys = xs.map { |x: Int| x + 1 }\nadd = |a: Int, b: Int| { a + b }\nprint(add(1, 2))\n").unwrap();
    // A union between pipes is written in parentheses.
    check("f = |x: (Int | String)| { x.to_s }\nprint(f(1))\n").unwrap();
    // `fn(…)` parameters close on `)`, so `|` is a union there.
    check("g = fn(x: Int | String) { x.to_s }\nprint(g(\"a\"))\n").unwrap();
}

#[test]
fn a_return_type_ends_at_its_line() {
    // The body's first line starting with `|` must not be read as a member.
    check("def run(xs) -> Int\n  xs.map { |x| x * 2 }.length\nend\nprint(run([1]))\n").unwrap();
}

#[test]
fn a_type_in_parentheses_is_a_group_not_a_function_type() {
    check("def f(xs: (Int | String)[]) -> Int\n  xs.length\nend\nprint(f([1, \"a\"]))\n").unwrap();
}

#[test]
fn fields_and_lets_take_unions() {
    check("class Slot\n  value: Int | String\n\n  new(value: Int | String)\n    @value = value\n  end\nend\ns = Slot.new(1)\nlet x: Int | nil = nil\nx = 2\n").unwrap();
}

#[test]
fn both_engines_check_a_union_return() {
    let ok = "def pick(flag: Bool) -> Int | nil\n  flag ? 1 : nil\nend\nprint(pick(true))\nprint(pick(false))\n";
    let bad = "def pick(flag) -> Int | nil\n  \"no\"\nend\nprint(pick(true))\n";
    for engine in ["--vm", "--tree"] {
        let (success, text) = run(ok, engine);
        assert!(success, "{engine}: {text}");
        assert!(
            text.contains('1') && text.contains("null"),
            "{engine}: {text}"
        );
        let (success, text) = run(bad, engine);
        assert!(!success, "{engine}: {text}");
    }
}

#[test]
fn fmt_prints_a_union_back() {
    let src = "def f(x: Int | String | nil) -> (Int | String)[]\n  [x]\nend\n";
    let formatted = solilang::fmt::format_source(src).expect("formats");
    assert!(formatted.contains("x: Int | String | nil"), "{formatted}");
    assert!(formatted.contains("-> (Int | String)[]"), "{formatted}");
}
