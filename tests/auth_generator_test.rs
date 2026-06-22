//! `soli generate auth` integration tests — exercise the auth scaffold
//! generator in a tempdir so `scaffold/auth_generator.rs` and the embedded
//! templates get covered without polluting the workspace.

use std::fs;
use std::path::Path;

use solilang::scaffold::create_auth;

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

#[test]
fn create_auth_writes_all_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let result = create_auth(dir.path().to_str().unwrap());
    assert!(result.is_ok(), "generate auth failed: {:?}", result.err());

    for rel in [
        "app/models/user.sl",
        "app/policies/application_policy.sl",
        "app/policies/user_policy.sl",
        "app/helpers/auth_helper.sl",
        "app/middleware/current_user.sl",
        "app/controllers/sessions_controller.sl",
        "app/controllers/registrations_controller.sl",
        "app/views/sessions/new.html.slv",
        "app/views/registrations/new.html.slv",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} not created");
    }

    // A timestamped users migration is generated.
    let has_migration = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().contains("create_users"));
    assert!(has_migration, "no create_users migration generated");
}

#[test]
fn create_auth_appends_routes() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_auth(dir.path().to_str().unwrap()).expect("generate auth ok");

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).expect("read routes");
    assert!(routes.contains("\"sessions#new\""), "login route missing");
    assert!(
        routes.contains("\"registrations#create\""),
        "signup route missing"
    );
}

#[test]
fn create_auth_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_auth(dir.path().to_str().unwrap()).expect("first run ok");
    // Customize a file; a second run must not clobber it.
    let user_model = dir.path().join("app/models/user.sl");
    fs::write(&user_model, "# my custom user\n").unwrap();

    create_auth(dir.path().to_str().unwrap()).expect("second run ok");

    assert_eq!(
        fs::read_to_string(&user_model).unwrap(),
        "# my custom user\n",
        "second run clobbered an existing file"
    );

    // Routes were not duplicated.
    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert_eq!(
        routes.matches("\"sessions#new\"").count(),
        1,
        "auth routes appended twice"
    );

    // Only one users migration exists.
    let migrations = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("create_users"))
        .count();
    assert_eq!(migrations, 1, "duplicate users migration generated");
}

#[test]
fn create_auth_rejects_non_app_directory() {
    let dir = tempfile::tempdir().unwrap();
    // No app/ directory.
    let result = create_auth(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert!(
        result
            .err()
            .unwrap()
            .contains("does not look like a Soli app"),
        "expected app-structure error"
    );
}
