//! `soli generate oauth` integration tests.

use std::fs;
use std::path::Path;

use solilang::scaffold::{create_auth, create_oauth};

fn make_app_with_auth(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
    create_auth(root.to_str().unwrap()).unwrap();
}

#[test]
fn create_oauth_github_writes_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());

    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();

    for rel in [
        "app/models/oauth_identity.sl",
        "app/services/oauth_client.sl",
        "app/services/github_oauth.sl",
        "app/controllers/oauth_controller.sl",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} missing");
    }

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("/auth/:provider"));
    assert!(routes.contains("oauth#callback"));

    let migrations: Vec<String> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        migrations
            .iter()
            .any(|n| n.contains("create_oauth_identities")),
        "missing identities migration: {migrations:?}"
    );
}

#[test]
fn create_oauth_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("app")).unwrap();
    let err = create_oauth(dir.path().to_str().unwrap(), "github").unwrap_err();
    assert!(err.contains("generate auth"), "{err}");
}

#[test]
fn create_oauth_rejects_unknown_provider() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    let err = create_oauth(dir.path().to_str().unwrap(), "facebook").unwrap_err();
    assert!(err.contains("Unknown OAuth provider"), "{err}");
}

#[test]
fn create_oauth_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();
    create_oauth(dir.path().to_str().unwrap(), "google").unwrap();
    assert!(dir.path().join("app/services/google_oauth.sl").exists());
    // Shared files still present, not wiped.
    assert!(dir.path().join("app/models/oauth_identity.sl").exists());
}
