//! `soli generate saml` — SAML 2.0 single sign-on with one identity provider,
//! on top of the session auth scaffold.
//!
//! The cryptography and the protocol checks are the native `Saml` class; what
//! is generated is the application's part: where the identity provider is,
//! the request in flight, and which account an assertion maps to. Every file
//! is written only when absent, the migration once, the routes behind their
//! own marker.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::saml;

/// Generate the SAML files into an app that has the auth scaffold.
pub fn create_saml(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.exists() {
        return Err(format!("Directory '{folder}' does not exist"));
    }
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "SAML sign-in requires `soli generate auth` first (missing app/models/user.sl).".into(),
        );
    }

    for dir in [
        "app/models",
        "app/services",
        "app/controllers",
        "config",
        "db/migrations",
    ] {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }

    for (rel, contents) in [
        ("app/models/saml_identity.sl", saml::SAML_IDENTITY_MODEL),
        ("app/models/saml_assertion.sl", saml::SAML_ASSERTION_MODEL),
        ("app/services/saml_sso.sl", saml::SAML_SSO_SERVICE),
        ("app/controllers/saml_controller.sl", saml::SAML_CONTROLLER),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }
    write_migration_once(
        app_path,
        "create_saml_identities",
        0,
        &saml::identities_migration(),
    )?;
    write_migration_once(
        app_path,
        "create_saml_assertions",
        1,
        &saml::assertions_migration(),
    )?;
    append_routes_once(
        app_path,
        saml::ROUTES_MARKER,
        &saml::routes_snippet(),
        "saml",
    )?;
    Ok(())
}

/// Next steps after `soli generate saml`.
pub fn print_saml_success_message() {
    println!();
    println!("  \x1b[32m\x1b[1mSAML single sign-on:\x1b[0m sign-in starts at \x1b[36m/saml/login\x1b[0m.");
    println!();
    println!("  1. \x1b[36msoli db:migrate up\x1b[0m");
    println!(
        "  2. Give the identity provider \x1b[36m/saml/metadata\x1b[0m (entity id and ACS URL),"
    );
    println!("     with \x1b[36mAPP_BASE_URL\x1b[0m set to the https:// origin it reaches you at.");
    println!(
        "  3. Point the app at the identity provider: \x1b[36mSAML_IDP_METADATA_FILE\x1b[0m, or"
    );
    println!("     \x1b[36mSAML_IDP_ENTITY_ID\x1b[0m + \x1b[36mSAML_IDP_SSO_URL\x1b[0m + \x1b[36mSAML_IDP_CERTIFICATE\x1b[0m.");
    println!("  4. Set \x1b[36mSOLI_SESSION_SECRET\x1b[0m (32+ characters): the request in flight");
    println!("     travels in an encrypted cookie.");
    println!("  5. Add the link to your sign-in view:");
    println!("       \x1b[36m<a href=\"/saml/login\">Sign in with SSO</a>\x1b[0m");
    println!();
    println!("  A first sign-in creates the account (a member, SSO only: no password),");
    println!("  unless \x1b[36mAUTH_SIGNUP=closed\x1b[0m; \x1b[36mSAML_PROVISION=on|off\x1b[0m overrides it.");
    println!("  An existing account with the same email is never linked by default;");
    println!("  \x1b[36mSAML_LINK_EXISTING=on\x1b[0m links it (never an admin's) — only if the");
    println!("  identity provider's users cannot choose the email it asserts.");
    println!("  Docs: /docs/security/saml");
    println!();
}
