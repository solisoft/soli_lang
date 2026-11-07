//! `soli check` on part of an application (#678).
//!
//! `soli check app` read `app/` as the project root, looked for
//! `app/app/models`, and reported every auto-loaded class as undefined, while
//! `soli check .` was clean. A target inside an application now widens to the
//! application's root (`soli.toml` or `config/routes.sl`) for the names in
//! scope, and still checks only the files it was given.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "config/routes.sl", "get(\"/\", \"posts#index\")\n");
    write(
        root,
        "app/models/audit_entry.sl",
        "class AuditEntry < Model\n  static def recent\n    AuditEntry.all\n  end\nend\n",
    );
    write(
        root,
        "app/services/report_service.sl",
        "def report_title(name)\n  \"Report: #{name}\"\nend\n",
    );
    write(
        root,
        "app/controllers/posts_controller.sl",
        "class PostsController < Controller\n  # GET /\n  def index(req)\n    @entries = AuditEntry.recent\n    @title = report_title(\"x\")\n  end\nend\n",
    );
    dir
}

fn check(dir: &Path, target: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("check")
        .arg(target)
        .current_dir(dir)
        .output()
        .expect("run soli check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn checking_the_app_directory_sees_the_auto_loaded_names() {
    let dir = app();
    let (ok, text) = check(dir.path(), "app");
    assert!(ok, "{text}");
    assert!(!text.contains("Undefined"), "{text}");
    assert!(text.contains("Checked 3 file(s)"), "{text}");
}

#[test]
fn checking_one_subdirectory_or_file_checks_only_it() {
    let dir = app();
    let (ok, text) = check(dir.path(), "app/controllers");
    assert!(ok, "{text}");
    assert!(text.contains("Checked 1 file(s)"), "{text}");

    let (ok, text) = check(dir.path(), "app/controllers/posts_controller.sl");
    assert!(ok, "{text}");
    assert!(text.contains("Checked 1 file(s)"), "{text}");
}

#[test]
fn soli_toml_also_marks_the_root() {
    let dir = app();
    std::fs::remove_file(dir.path().join("config/routes.sl")).unwrap();
    std::fs::remove_dir(dir.path().join("config")).unwrap();
    write(dir.path(), "soli.toml", "");
    let (ok, text) = check(dir.path(), "app");
    assert!(ok, "{text}");
}

#[test]
fn a_loose_directory_keeps_the_narrow_view() {
    // No application around it: a name it does not declare is still undefined.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "scripts/run.sl", "print(report_title(\"x\"))\n");
    let (ok, text) = check(dir.path(), "scripts");
    assert!(!ok, "{text}");
    assert!(text.contains("report_title"), "{text}");
}

#[test]
fn jobs_and_subscribers_are_auto_loaded_too() {
    // `soli check`'s list had neither (#691); the server loads both into the
    // application's environment.
    let dir = app();
    write(
        dir.path(),
        "app/jobs/cleanup_job.sl",
        "def schedule_cleanup(days)\n  days\nend\n",
    );
    write(
        dir.path(),
        "app/subscribers/audit_subscriber.sl",
        "def audit_topic\n  \"audit\"\nend\n",
    );
    write(
        dir.path(),
        "app/services/maintenance.sl",
        "def run_maintenance\n  schedule_cleanup(7)\n  audit_topic()\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/maintenance.sl");
    assert!(ok, "{text}");
    assert!(!text.contains("Undefined"), "{text}");
}
