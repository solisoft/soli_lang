//! Named scopes on a model class, read on the VM.
//!
//! `Model.scope_name` resolves in the tree-walker's `class_member_access`,
//! which checks the scope registry before the statics. The VM's class branch
//! had no such check: `Measure.t` fell through to "Cannot access property
//! 't' on Measure". Under `soli serve` that error sent the handler back to
//! the tree-walker — harmless before a write, a 500 after one. An action that
//! wrote an audit row and then read through a tenant scope (`Coverage.t`)
//! answered 500 on every request (grc, organisation export, 27/09/2026).
//!
//! The VM now resolves a model scope through the tree-walker's member access,
//! with the program's globals (a scope closure is user code: it may call
//! `Current` or an application helper), for a bare read, an empty-parens
//! call, and a scope that takes arguments.
//!
//! The scopes are declared from a module's `included` block, the shape apps
//! use for a shared tenant scope. A `scope(...)` written directly in a class
//! body is a separate gap: under `--vm` no class-body DSL (`validates`,
//! `belongs_to`, `scope`…) receives its class, while `soli serve` loads model
//! files on the tree-walker.

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

/// No database here: a read comes back in-band as an empty result or an error
/// string, so only reaching each `print` is asserted — the lookup is what
/// failed, not the read.
const SCOPES: &str = r#"
module Scoped
  included do
    scope("t", fn() { this.where({"tenant_id": "x"}) })
    scope("by", fn(v) { this.where({"b": v}) })
  end
end

class Thing < Model
  include Scoped
end

print("bare=" + str(Thing.t.where({"b": 2}).limit(1).all.length()))
print("parens=" + str(Thing.t().limit(1).all.length()))
print("args=" + str(Thing.by(3).limit(1).all.length()))
"#;

fn assert_every_scope_read(run: &Run) {
    for marker in ["bare=", "parens=", "args="] {
        assert!(
            run.stdout.contains(marker),
            "missing {marker}\nstdout: {}\nstderr: {}",
            run.stdout,
            run.stderr
        );
    }
    assert!(
        !run.stderr.contains("Cannot access property"),
        "stderr: {}",
        run.stderr
    );
}

#[test]
fn the_interpreter_reads_a_model_scope() {
    assert_every_scope_read(&run_script(SCOPES, false));
}

#[test]
fn the_vm_reads_a_model_scope() {
    assert_every_scope_read(&run_script(SCOPES, true));
}

/// A static method and a scope can share a class: the static still wins when
/// no scope of that name exists, and a class without scopes is untouched.
#[test]
fn the_vm_still_resolves_statics_next_to_scopes() {
    let run = run_script(
        r#"
module Scoped
  included do
    scope("t", fn() { this.where({"tenant_id": "x"}) })
  end
end

class Thing < Model
  include Scoped
  static def label() -> String
    return "static"
  end
end

print("label=" + Thing.label())
print("scoped=" + str(Thing.t.limit(1).all.length()))
"#,
        true,
    );
    assert!(
        run.stdout.contains("label=static"),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
    assert!(
        run.stdout.contains("scoped="),
        "stdout: {}\nstderr: {}",
        run.stdout,
        run.stderr
    );
}
