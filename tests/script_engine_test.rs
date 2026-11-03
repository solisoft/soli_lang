//! Which engine runs `soli script.sl`, and that the choice does not show.
//!
//! Scripts run on the VM unless they need the tree-walker. On a script that
//! works, both give the same output; on one that fails, the same error, at the
//! same line and column.

use std::path::Path;
use std::process::Command;

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn soli(dir: &Path, args: &[&str], log: bool) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.current_dir(dir).args(args).env_remove("SOLI_ENGINE");
    if log {
        cmd.env("SOLI_ENGINE_LOG", "1");
    } else {
        cmd.env_remove("SOLI_ENGINE_LOG");
    }
    let out = cmd.output().expect("run soli");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    }
}

fn script(source: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("s.sl"), source).expect("write");
    dir
}

#[test]
fn a_script_runs_on_the_vm_by_default() {
    let dir = script("print(1 + 1)\n");
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "2\n");
    assert!(run.stderr.contains("engine: vm"), "{}", run.stderr);

    let tree = soli(dir.path(), &["--tree", "s.sl"], true);
    assert_eq!(tree.stdout, "2\n");
    assert!(!tree.stderr.contains("engine: vm"), "{}", tree.stderr);
}

#[test]
fn reflection_sends_a_script_to_the_tree_walker() {
    // `methods` is one of the members the VM hands back at run time; a
    // script cannot be replayed halfway, so the choice is made before it
    // starts.
    let dir = script(
        "class A\n  def hi\n    \"hi\"\n  end\nend\nnames = new A().methods rescue nil\nprint(new A().hi())\n",
    );
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "hi\n", "{}", run.stderr);
    assert!(
        run.stderr
            .contains("engine: tree-walker — the script uses `methods`"),
        "{}",
        run.stderr
    );
}

#[test]
fn send_on_an_instance_keeps_a_script_on_the_vm() {
    // `obj.send("m")` called with positional arguments runs on the VM
    // (`call_method_slow_path`), so it no longer picks the tree-walker.
    let dir = script("class A\n  def hi\n    \"hi\"\n  end\nend\nprint(new A().send(\"hi\"))\n");
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "hi\n", "{}", run.stderr);
    assert!(run.stderr.contains("engine: vm"), "{}", run.stderr);
}

/// Same stdout, same stderr, same exit code on the default engine and on
/// `--tree`.
fn assert_same(source: &str) {
    let dir = script(source);
    let auto = soli(dir.path(), &["s.sl"], false);
    let tree = soli(dir.path(), &["--tree", "s.sl"], false);
    assert_eq!(auto.stdout, tree.stdout, "stdout for:\n{source}");
    assert_eq!(auto.stderr, tree.stderr, "stderr for:\n{source}");
    assert_eq!(auto.code, tree.code, "exit code for:\n{source}");
}

#[test]
fn errors_point_where_the_tree_walker_points() {
    // The column used to read 0 on the VM.
    assert_same("x = 10\nprint(x / 0)\n");
    assert_same("def f(n)\n  n * 9223372036854775807\nend\nprint(f(2))\n");
}

#[test]
fn decimal_and_array_difference_scripts_stay_on_the_vm() {
    // Decimal arithmetic, a Money amount and an array difference raised a
    // type error on the VM, which auto mode picks for these scripts: the
    // script stopped halfway with exit 70.
    let source = "price = 19.99D\ntotal = price * 3\nprint(total, total > 50, [1, 2, 3] - [2])\n\
                  print(Money.new(19.99, \"EUR\")[\"amount\"] * 3)\n";
    let dir = script(source);
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "59.97 true [1, 3]\n59.97\n", "{}", run.stderr);
    assert!(run.stderr.contains("engine: vm"), "{}", run.stderr);
    assert_same(source);
    // A zero Decimal divisor said "Cannot divide decimal by decimal".
    assert_same("def split(total, parts)\n  total / parts\nend\nprint(split(10.00D, 0.00D))\n");
}

#[test]
fn an_invalid_decimal_literal_raises_as_on_the_tree_walker() {
    // `%n[abcD]` reads as the decimal literal `abc.00`, and an integer part
    // past Decimal's range does not parse either. The VM loaded a silent 0
    // for both. The compiler now refuses them, so the tree-walker runs the
    // script and raises "Invalid decimal literal" where one is evaluated.
    let source = "x = %n[abcD] rescue \"invalid\"\nprint(x)\n";
    assert_same(source);
    let dir = script(source);
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "invalid\n", "{}", run.stderr);
    assert!(
        run.stderr
            .contains("the VM cannot compile it (Invalid decimal literal: abc.00"),
        "{}",
        run.stderr
    );
    assert_same("price = 99999999999999999999999999999999.5D\nprint(price)\n");
}

#[test]
fn a_declared_return_type_is_checked_on_the_vm_too() {
    let source = "def f(n: Int) -> Int\n  if n > 0\n    n\n  end\nend\nprint(f(1))\nprint(f(-1))\n";
    assert_same(source);
    let dir = script(source);
    let run = soli(dir.path(), &["s.sl"], false);
    assert!(
        run.stderr
            .contains("function 'f' expected to return Int, got null at 1:1"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_non_primitive_return_type_is_checked_on_the_vm_too() {
    // `-> Hash` and a class name took the allocating path; they now check
    // against the type lowered at compile time, with the same error.
    for (source, message) in [
        (
            "class Pt\nend\ndef f -> Pt\n  \"x\"\nend\nprint(f())\n",
            "function 'f' expected to return Pt, got string at 3:1",
        ),
        (
            "def f -> Hash\n  [1]\nend\nprint(f())\n",
            "function 'f' expected to return Hash, got array at 1:1",
        ),
        (
            "def f -> Function\n  return fn(x) { x }\nend\nprint(f()(7))\n",
            "",
        ),
    ] {
        assert_same(source);
        let dir = script(source);
        let run = soli(dir.path(), &["s.sl"], false);
        if message.is_empty() {
            assert_eq!(run.stdout, "7\n", "{}", run.stderr);
        } else {
            assert!(run.stderr.contains(message), "{}", run.stderr);
        }
    }
}

#[test]
fn the_depth_limit_reads_the_same() {
    assert_same("def down(n)\n  return 0 if n == 0\n\n  down(n - 1)\nend\nprint(down(100000))\n");
}

#[test]
fn the_native_fixtures_agree_across_engines() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_native");
    for entry in std::fs::read_dir(&dir).expect("fixtures").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "sl") {
            continue;
        }
        let file = path.to_string_lossy().into_owned();
        let mut args = vec![file.as_str()];
        if path.file_stem().is_some_and(|s| s == "no_type_check") {
            args.push("--no-type-check");
        }
        let auto = soli(&dir, &args, false);
        let mut tree_args = vec!["--tree"];
        tree_args.extend(&args);
        let tree = soli(&dir, &tree_args, false);
        assert_eq!(auto.stdout, tree.stdout, "{file}");
        assert_eq!(auto.stderr, tree.stderr, "{file}");
        assert_eq!(auto.code, tree.code, "{file}");
    }
}

#[test]
fn a_function_read_without_parentheses_keeps_the_tree_walker() {
    // The tree-walker calls `helper` here; the VM would hand over the
    // function. Scripts were written against the tree-walker, so it keeps
    // running there.
    let source = "def helper\n  \"called\"\nend\nx = helper\nprint(x)\n";
    assert_same(source);
    let dir = script(source);
    let run = soli(dir.path(), &["s.sl"], true);
    assert_eq!(run.stdout, "called\n");
    assert!(
        run.stderr
            .contains("engine: tree-walker — the script calls `helper` without parentheses"),
        "{}",
        run.stderr
    );

    // With the parentheses it is an ordinary call, and the VM runs it.
    let dir = script("def helper\n  \"called\"\nend\nprint(helper())\n");
    let run = soli(dir.path(), &["s.sl"], true);
    assert!(run.stderr.contains("engine: vm"), "{}", run.stderr);
}
