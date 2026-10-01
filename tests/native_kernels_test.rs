//! Native kernels change speed, never behaviour.
//!
//! Each script in `tests/fixtures/_native/` is run on both engines, with
//! kernels and without (`SOLI_NATIVE=0`), and the four runs must agree in
//! pairs: same stdout, same stderr, same exit code, error messages included.
//! The tree-walker and the VM are *not* compared with each other — they
//! already differ in places (error spans, a missing return value), and a
//! kernel must reproduce whichever engine called it.
//!
//! A comparison passes trivially when nothing was compiled, so each fixture
//! names the kernels it expects on a `# kernels:` line, and the test checks
//! `SOLI_NATIVE_LOG=1` reported exactly those.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_native");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "sl"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no fixtures in {}", dir.display());
    files
}

struct Outcome {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn run(path: &Path, vm: bool, native: bool, log: bool) -> Outcome {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.arg(path);
    cmd.arg(if vm { "--vm" } else { "--tree" });
    if path.file_stem().is_some_and(|s| s == "no_type_check") {
        cmd.arg("--no-type-check");
    }
    cmd.env("SOLI_NATIVE", if native { "1" } else { "0" });
    if log {
        cmd.env("SOLI_NATIVE_LOG", "1");
    } else {
        cmd.env_remove("SOLI_NATIVE_LOG");
    }
    let output = cmd.output().expect("run soli");
    Outcome {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code(),
    }
}

fn expected_kernels(path: &Path) -> Vec<String> {
    let source = std::fs::read_to_string(path).expect("read fixture");
    let line = source
        .lines()
        .find_map(|l| l.strip_prefix("# kernels:"))
        .unwrap_or_else(|| panic!("{} has no `# kernels:` line", path.display()));
    let mut names: Vec<String> = line.split(',').map(|n| n.trim().to_string()).collect();
    names.sort();
    names
}

fn compiled_kernels(stderr: &str) -> Vec<String> {
    let mut names: Vec<String> = stderr
        .lines()
        .filter(|l| l.contains(" compiled (line "))
        .filter_map(|l| l.strip_prefix("native: "))
        .filter_map(|l| l.split('(').next())
        .map(str::to_string)
        .collect();
    names.sort();
    names
}

#[test]
fn kernels_do_not_change_what_a_script_does() {
    let mut failures = Vec::new();
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for vm in [false, true] {
            let engine = if vm { "vm" } else { "tree" };
            let off = run(&path, vm, false, false);
            let on = run(&path, vm, true, false);
            if off.stdout != on.stdout {
                failures.push(format!(
                    "{name} [{engine}] stdout differs\n  without kernels: {:?}\n  with kernels:    {:?}",
                    off.stdout, on.stdout
                ));
            }
            if off.stderr != on.stderr {
                failures.push(format!(
                    "{name} [{engine}] stderr differs\n  without kernels: {:?}\n  with kernels:    {:?}",
                    off.stderr, on.stderr
                ));
            }
            if off.code != on.code {
                failures.push(format!(
                    "{name} [{engine}] exit code {:?} without kernels, {:?} with",
                    off.code, on.code
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
#[cfg_attr(not(feature = "native"), ignore = "built without the `native` feature")]
fn each_fixture_compiles_the_kernels_it_names() {
    let mut failures = Vec::new();
    for path in fixtures() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let want = expected_kernels(&path);
        for vm in [false, true] {
            let logged = run(&path, vm, true, true);
            let got = compiled_kernels(&logged.stderr);
            if got != want {
                failures.push(format!(
                    "{name} [{}]: expected kernels {want:?}, compiled {got:?}\n{}",
                    if vm { "vm" } else { "tree" },
                    logged.stderr
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
#[cfg_attr(not(feature = "native"), ignore = "built without the `native` feature")]
fn the_log_says_why_a_function_was_refused() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_native/binding.sl");
    let logged = run(&path, false, true, true);
    assert!(
        logged
            .stderr
            .contains("native: shout (line 26) refused — calls `print` (line 27)"),
        "{}",
        logged.stderr
    );
    assert!(
        logged
            .stderr
            .contains("native: again (line 31) refused — defined more than once"),
        "{}",
        logged.stderr
    );
    // The call made before `later` existed declined, and said so once.
    assert!(
        logged.stderr.contains(
            "native: early declined (a function it calls is not defined, or was redefined)"
        ),
        "{}",
        logged.stderr
    );
}
