//! `soli generate mailer` integration tests — exercise the mailer scaffold
//! generator in a tempdir so `scaffold/mailer_generator.rs` gets covered
//! without polluting the workspace.

use std::fs;
use std::path::Path;

use solilang::scaffold::create_mailer;

fn make_app_skeleton(root: &Path) {
    for sub in ["app/mailers", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
}

#[test]
fn create_mailer_writes_class_and_views() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let actions = vec!["welcome".to_string(), "reset_password".to_string()];
    let result = create_mailer(dir.path().to_str().unwrap(), "User", &actions);
    assert!(result.is_ok(), "generate mailer failed: {:?}", result.err());

    for rel in [
        "app/mailers/user_mailer.sl",
        "app/views/user_mailer/welcome.html.slv",
        "app/views/user_mailer/reset_password.html.slv",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} not created");
    }

    let class = fs::read_to_string(dir.path().join("app/mailers/user_mailer.sl")).unwrap();
    assert!(
        class.contains("class UserMailer < Mailer"),
        "missing class decl"
    );
    assert!(
        class.contains("def welcome(user)"),
        "missing welcome action"
    );
    assert!(
        class.contains("def reset_password(user)"),
        "missing reset_password action"
    );
    assert!(class.contains("this.mail("), "action should call this.mail");
}

#[test]
fn create_mailer_normalizes_name_with_suffix() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    // "OrderMailer" must not become "order_mailer_mailer".
    create_mailer(dir.path().to_str().unwrap(), "OrderMailer", &[]).expect("ok");
    assert!(dir.path().join("app/mailers/order_mailer.sl").exists());
    // No actions given -> a default `welcome` action + view.
    assert!(dir
        .path()
        .join("app/views/order_mailer/welcome.html.slv")
        .exists());
    let class = fs::read_to_string(dir.path().join("app/mailers/order_mailer.sl")).unwrap();
    assert!(class.contains("class OrderMailer < Mailer"));
}

#[test]
fn create_mailer_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let actions = vec!["welcome".to_string()];
    create_mailer(dir.path().to_str().unwrap(), "User", &actions).expect("first run");

    let mailer = dir.path().join("app/mailers/user_mailer.sl");
    fs::write(&mailer, "# customized\n").unwrap();

    create_mailer(dir.path().to_str().unwrap(), "User", &actions).expect("second run");
    assert_eq!(
        fs::read_to_string(&mailer).unwrap(),
        "# customized\n",
        "second run clobbered an existing mailer"
    );
}

#[test]
fn create_mailer_rejects_non_app_directory() {
    let dir = tempfile::tempdir().unwrap();
    let result = create_mailer(dir.path().to_str().unwrap(), "User", &[]);
    assert!(result.is_err());
    assert!(result
        .err()
        .unwrap()
        .contains("does not look like a Soli app"));
}
