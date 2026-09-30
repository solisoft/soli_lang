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

#[test]
fn create_oauth_writes_every_provider_service() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in [
        "github",
        "google",
        "gitlab",
        "discord",
        "linkedin",
        "microsoft",
    ] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
        let service = dir.path().join(format!("app/services/{provider}_oauth.sl"));
        assert!(service.exists(), "{provider} service missing");
    }
    // The controller finds providers by class name instead of listing them.
    let controller =
        fs::read_to_string(dir.path().join("app/controllers/oauth_controller.sl")).unwrap();
    assert!(controller.contains("oauth_service_for"), "{controller}");
    assert!(controller.contains("redirect_external"), "{controller}");
}

#[test]
fn create_oauth_accepts_provider_aliases() {
    use solilang::scaffold::oauth_generator::normalize_provider;
    assert_eq!(normalize_provider("GL").unwrap(), "gitlab");
    assert_eq!(normalize_provider("azure").unwrap(), "microsoft");
    assert_eq!(normalize_provider("li").unwrap(), "linkedin");
}

#[test]
fn emitted_provider_services_parse() {
    use solilang::lexer::Scanner;
    use solilang::parser::Parser;

    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in ["gitlab", "discord", "linkedin", "microsoft"] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
        let rel = format!("app/services/{provider}_oauth.sl");
        let source = fs::read_to_string(dir.path().join(&rel)).unwrap();
        let tokens = Scanner::new(&source).scan_tokens().unwrap();
        Parser::new(tokens)
            .parse()
            .unwrap_or_else(|e| panic!("{rel} does not parse: {e:?}"));
    }
}
