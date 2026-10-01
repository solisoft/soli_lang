//! `soli build tool.sl` makes an executable that runs the script, and a
//! script reads its own arguments with `System.argv`.

use std::path::Path;
use std::process::Command;

fn soli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_soli"))
}

fn write(dir: &Path, name: &str, source: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, source).expect("write");
}

const TOOL: &str = r#"import "./lib/math.sl"

args = System.argv
if args.length == 0
  print("usage: tool N")
else
  n = args[0].to_i()
  print("fib(#{n}) = #{fib(n)} #{args.join(",")}")
end
"#;

const MATH: &str =
    "export def fib(n: Int) -> Int\n  return n if n < 2\n\n  fib(n - 1) + fib(n - 2)\nend\n";

#[test]
fn a_script_reads_the_arguments_after_double_dash() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tool.sl", TOOL);
    write(dir.path(), "lib/math.sl", MATH);

    // A bare file name, run from its own directory: its imports resolve
    // from `.`.
    let out = soli()
        .current_dir(dir.path())
        .args(["tool.sl", "--", "10", "--flag"])
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "fib(10) = 55 10,--flag\n"
    );

    let without = soli()
        .current_dir(dir.path())
        .arg("tool.sl")
        .output()
        .expect("run");
    assert_eq!(String::from_utf8_lossy(&without.stdout), "usage: tool N\n");
}

#[test]
fn a_built_script_runs_without_its_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tool.sl", TOOL);
    write(dir.path(), "lib/math.sl", MATH);
    let exe = dir.path().join(if cfg!(windows) {
        "fibtool.exe"
    } else {
        "fibtool"
    });

    let build = soli()
        .current_dir(dir.path())
        .args(["build", "tool.sl", "-o"])
        .arg(&exe)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    // The sources are gone; the executable carries the resolved program.
    std::fs::remove_file(dir.path().join("tool.sl")).expect("rm");
    std::fs::remove_dir_all(dir.path().join("lib")).expect("rm");

    let elsewhere = tempfile::tempdir().expect("tempdir");
    let run = Command::new(&exe)
        .current_dir(elsewhere.path())
        .args(["20", "x"])
        .output()
        .expect("run built tool");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "fib(20) = 6765 20,x\n"
    );

    let bytes = std::fs::read(&exe).expect("read exe");
    let build_dir = dir.path().to_string_lossy().into_owned();
    assert!(
        !bytes
            .windows(build_dir.len())
            .any(|w| w == build_dir.as_bytes()),
        "the build directory is embedded in the executable"
    );
}

#[test]
fn a_built_script_exits_like_soli_on_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "bad.sl", "print(\"before\")\nprint(1 / 0)\n");
    let exe = dir
        .path()
        .join(if cfg!(windows) { "bad.exe" } else { "bad" });
    let build = soli()
        .current_dir(dir.path())
        .args(["build", "bad.sl", "--vm", "-o"])
        .arg(&exe)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&exe).output().expect("run");
    assert_eq!(run.status.code(), Some(70));
    assert_eq!(String::from_utf8_lossy(&run.stdout), "before\n");
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("Division by zero"),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn a_script_build_refuses_app_only_flags() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tool.sl", "print(1)\n");
    let out = soli()
        .current_dir(dir.path())
        .args(["build", "tool.sl", "--encrypt"])
        .output()
        .expect("build");
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not to a script executable"));
}

#[test]
fn arguments_after_the_script_belong_to_the_script() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "args.sl", "print(System.argv.join(\",\"))\n");
    let run = |args: &[&str]| {
        let out = soli()
            .current_dir(dir.path())
            .args(args)
            .output()
            .expect("run");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // As `python tool.py a b` does — what a `#!/usr/bin/env soli` script gets.
    assert_eq!(run(&["args.sl", "a", "b"]), "a,b\n");
    assert_eq!(run(&["args.sl", "--verbose", "x"]), "--verbose,x\n");
    // The runner's own options still work after the script…
    assert_eq!(run(&["args.sl", "--vm", "a"]), "a\n");
    // …and `--` hands even those to the script.
    assert_eq!(run(&["args.sl", "--", "--vm"]), "--vm\n");
}

#[test]
fn a_thin_script_is_small_and_runs_without_its_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "tool.sl", TOOL);
    write(dir.path(), "lib/math.sl", MATH);
    let thin = dir.path().join("tool-thin");

    let build = soli()
        .current_dir(dir.path())
        .args(["build", "tool.sl", "--thin", "-o"])
        .arg(&thin)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let bytes = std::fs::read(&thin).expect("read");
    assert!(bytes.starts_with(b"#!/usr/bin/env soli\n"));
    assert!(bytes.len() < 64 * 1024, "{} bytes", bytes.len());

    std::fs::remove_file(dir.path().join("tool.sl")).expect("rm");
    std::fs::remove_dir_all(dir.path().join("lib")).expect("rm");

    // Run through soli, as the shebang line does.
    let run = soli()
        .arg(&thin)
        .args(["20", "x"])
        .output()
        .expect("run thin");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "fib(20) = 6765 20,x\n"
    );
}

#[test]
fn a_thin_script_from_another_soli_version_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "t.sl", "print(1)\n");
    let thin = dir.path().join("t");
    let build = soli()
        .current_dir(dir.path())
        .args(["build", "t.sl", "--thin", "-o"])
        .arg(&thin)
        .output()
        .expect("build");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let ours = format!("\"soli_version\":\"{}\"", env!("CARGO_PKG_VERSION"));
    let bytes = std::fs::read(&thin).expect("read");
    let pos = bytes
        .windows(ours.len())
        .position(|w| w == ours.as_bytes())
        .expect("version in the descriptor");
    let mut other = bytes.clone();
    // Same length, another version: "0.0.1" padded to the original width.
    let fake = format!(
        "{:width$}",
        "0.0.1",
        width = env!("CARGO_PKG_VERSION").len()
    );
    let start = pos + "\"soli_version\":\"".len();
    other[start..start + fake.len()].copy_from_slice(fake.as_bytes());
    std::fs::write(&thin, other).expect("write");

    let run = soli().arg(&thin).output().expect("run");
    assert_eq!(run.status.code(), Some(70));
    let err = String::from_utf8_lossy(&run.stderr);
    assert!(
        err.contains("was built with soli") && err.contains("Rebuild it"),
        "{err}"
    );
}

#[test]
fn thin_takes_no_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(dir.path(), "t.sl", "print(1)\n");
    let out = soli()
        .current_dir(dir.path())
        .args(["build", "t.sl", "--thin", "--target", "linux-arm64"])
        .output()
        .expect("build");
    assert_eq!(out.status.code(), Some(64));
}
