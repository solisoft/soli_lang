//! The VM must hand query-builder member access back to the tree-walker.
//!
//! `where`, `limit`, `first`, `order`, the aggregates and scope chaining are
//! implemented only in the interpreter's `query_builder_member_access`, and
//! running them needs an `Interpreter`. The VM had no arm for
//! `Value::QueryBuilder` at all, so its catch-all raised `NoSuchProperty`
//! ("Cannot access property 'limit' on QueryBuilder").
//!
//! That error class is **catchable by user code**, which is what made it a
//! production bug rather than a demotion: a handler with its own `try/catch`
//! around a model call swallowed it and reported its own failure, so the error
//! never reached the serve layer and the handler never demoted to the
//! interpreter. `EngineFallback` is deliberately not catchable
//! (`vm.rs`: `if !catchable || err.is_engine_fallback()`), so it propagates,
//! the handler demotes once, and the code runs.
//!
//! These tests pin both halves: the VM refuses with a fallback, and user
//! `try/catch` cannot intercept that refusal.

use std::path::PathBuf;
use std::process::Command;

fn soli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_soli"))
}

struct Run {
    stdout: String,
    stderr: String,
}

fn run_script(source: &str, vm: bool) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("t.sl");
    std::fs::write(&script, source).expect("write script");

    let mut cmd = Command::new(soli_binary());
    if vm {
        cmd.arg("--vm");
    }
    let out = cmd
        .arg("--no-type-check")
        .arg(&script)
        .current_dir(dir.path())
        .output()
        .expect("soli should run");

    Run {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// A model chain, the shape every app writes.
const CHAIN: &str = r#"
class Thing < Model
  static def newest() -> Any
    return Thing.where({"a": 1}).limit(1).all
  end
end
print("rows=" + str(Thing.newest().length()))
"#;

/// The tree-walker runs it. No database here, so the result is empty — what
/// matters is that it is a result and not an error.
#[test]
fn the_interpreter_runs_a_query_builder_chain() {
    let run = run_script(CHAIN, false);
    assert!(
        run.stdout.contains("rows="),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
}

/// The VM refuses, and the refusal must say it needs the interpreter — that is
/// the marker `serve` keys the demotion off. The old "Cannot access property"
/// wording was a plain runtime error and demoted nothing.
#[test]
fn the_vm_refuses_with_an_engine_fallback_not_a_property_error() {
    let run = run_script(CHAIN, true);
    let all = format!("{}{}", run.stdout, run.stderr);

    assert!(
        all.contains("requires the interpreter"),
        "the VM should ask for the interpreter: {all}"
    );
    assert!(
        !all.contains("Cannot access property"),
        "a query-builder member must not read as a missing property: {all}"
    );
}

/// The bug in one test.
///
/// A handler that wraps a model call in its own `try/catch` used to swallow the
/// VM's refusal and report its own error — the production symptom was
/// `OAuth failed: Cannot access property 'limit' on QueryBuilder`. Because the
/// error never escaped, `serve` never saw it and never demoted the handler, so
/// the route stayed broken for every request.
#[test]
fn user_code_cannot_swallow_the_vms_refusal() {
    let source = r#"
class Thing < Model
  static def newest() -> Any
    return Thing.where({"a": 1}).limit(1).all
  end
end

try {
  let rows = Thing.newest()
  print("rows=" + str(rows.length()))
} catch e {
  print("SWALLOWED: " + str(e))
}
"#;

    let run = run_script(source, true);
    let all = format!("{}{}", run.stdout, run.stderr);

    assert!(
        !all.contains("SWALLOWED"),
        "user try/catch must not intercept the engine fallback, or serve never \
         learns it has to demote the handler: {all}"
    );
}

/// `.first` has the same shape as `.limit` and took the same path.
#[test]
fn first_is_also_handed_back_rather_than_reported_as_missing() {
    let source = r#"
class Thing < Model
  static def one() -> Any
    return Thing.where({"a": 1}).first
  end
end
print("got=" + str(Thing.one()))
"#;

    let interpreted = run_script(source, false);
    assert!(
        interpreted.stdout.contains("got="),
        "the interpreter should run it: {}{}",
        interpreted.stdout,
        interpreted.stderr
    );

    let vm = run_script(source, true);
    let all = format!("{}{}", vm.stdout, vm.stderr);
    assert!(
        !all.contains("Cannot access property"),
        "`.first` must not read as a missing property either: {all}"
    );
}
