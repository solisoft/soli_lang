//! `soli runner`: a script or one line of code, run with the app loaded.

use std::fs;
use std::path::Path;
use std::process::Command;

fn app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("app/models")).unwrap();
    fs::create_dir_all(root.join("app/services")).unwrap();
    fs::create_dir_all(root.join("tmp")).unwrap();
    fs::write(
        root.join("app/models/greeting.sl"),
        "class Greeting\n  static def hello(name)\n    \"hello #{name}\"\n  end\nend\n",
    )
    .unwrap();
    fs::write(
        root.join("app/services/shouter.sl"),
        "class Shouter\n  static def shout(text)\n    text.upcase\n  end\nend\n",
    )
    .unwrap();
    fs::write(root.join(".env"), "RUNNER_GREETING=from-dotenv\n").unwrap();
    dir
}

fn runner(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("runner")
        .args(args)
        .current_dir(cwd)
        .env_remove("RUNNER_GREETING")
        .output()
        .expect("run soli runner");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn code_runs_with_models_services_and_env_loaded() {
    let dir = app();
    let (code, out, err) = runner(
        dir.path(),
        &[
            "-e",
            "print(Shouter.shout(Greeting.hello(getenv(\"RUNNER_GREETING\"))))",
        ],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "HELLO FROM-DOTENV");
}

#[test]
fn a_script_runs_from_the_app_folder_named_after_it() {
    let dir = app();
    let root = dir.path();
    fs::write(
        root.join("tmp/report.sl"),
        "print(Greeting.hello(\"script\"))\nprint(File.exists(\"app/models/greeting.sl\"))\n",
    )
    .unwrap();
    // From elsewhere: the script path as typed, the app folder as an argument.
    let outside = tempfile::tempdir().unwrap();
    let script = root.join("tmp/report.sl");
    let (code, out, err) = runner(
        outside.path(),
        &[script.to_str().unwrap(), root.to_str().unwrap()],
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out.trim(), "hello script\ntrue");
    // From the app: a path relative to it.
    let (code, out, err) = runner(root, &["tmp/report.sl"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with("hello script"), "{out}");
}

#[test]
fn an_error_exits_non_zero_and_a_folder_that_is_no_app_is_refused() {
    let dir = app();
    let (code, _, err) = runner(dir.path(), &["-e", "Greeting.nope()"]);
    assert_ne!(code, 0);
    assert!(err.contains("Error"), "{err}");

    let empty = tempfile::tempdir().unwrap();
    let (code, _, err) = runner(empty.path(), &["-e", "print(1)"]);
    assert_eq!(code, 64);
    assert!(err.contains("not a Soli app"), "{err}");

    let (code, _, err) = runner(dir.path(), &[]);
    assert_eq!(code, 64);
    assert!(err.contains("runner requires a script"), "{err}");
}
