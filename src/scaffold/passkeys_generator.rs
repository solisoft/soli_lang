//! `soli generate auth --passkeys` — passkey (WebAuthn) registration and
//! sign-in on top of the session auth scaffold.
//!
//! Runs after the base auth set, and on its own against an app that already
//! ran `soli generate auth`: every file is written only when absent, the
//! migration once, the routes behind their own marker. An existing sign-in
//! view is never patched — the success message prints the partial include.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::{auth, passkeys};

const SESSIONS_VIEW: &str = "app/views/sessions/new.html.slv";

/// The scaffolded sign-in view with the passkey partial and a
/// conditional-UI email field. Written instead of the plain view when
/// `--passkeys` comes with a fresh `generate auth`.
pub fn sessions_view_with_passkeys() -> String {
    auth::SESSIONS_NEW_VIEW
        .replacen(
            "type=\"email\" required autofocus",
            "type=\"email\" required autofocus autocomplete=\"username webauthn\"",
            1,
        )
        .replacen(
            "    <% if signup_open %>",
            &format!(
                "{}\n    <% if signup_open %>",
                passkeys::SIGN_IN_PARTIAL_INCLUDE.trim_end()
            ),
            1,
        )
}

/// Write the passkey-aware sign-in view when the app has none yet. Called
/// before the base auth files, so `write_if_absent` then keeps it.
pub(crate) fn write_sessions_view_if_absent(app_path: &Path) -> Result<bool, String> {
    if app_path.join(SESSIONS_VIEW).exists() {
        return Ok(false);
    }
    write_if_absent(app_path, SESSIONS_VIEW, &sessions_view_with_passkeys())?;
    Ok(true)
}

/// Generate the passkey files into an app that has the auth scaffold.
pub fn create_passkeys(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "Passkeys require the auth scaffold (missing app/models/user.sl): run `soli generate auth --passkeys`."
                .into(),
        );
    }

    for dir in [
        "app/models",
        "app/controllers",
        "app/views/passkeys",
        "public/js",
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
        (
            "app/models/passkey_credential.sl",
            passkeys::PASSKEY_CREDENTIAL_MODEL,
        ),
        (
            "app/controllers/passkeys_controller.sl",
            passkeys::PASSKEYS_CONTROLLER,
        ),
        (
            "app/controllers/passkey_sessions_controller.sl",
            passkeys::PASSKEY_SESSIONS_CONTROLLER,
        ),
        (
            "app/views/passkeys/index.html.slv",
            passkeys::PASSKEYS_INDEX_VIEW,
        ),
        (
            "app/views/passkeys/_sign_in.html.slv",
            passkeys::SIGN_IN_PARTIAL,
        ),
        ("public/js/passkeys.js", passkeys::PASSKEYS_JS),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }

    // +2 orders it after the two base auth migrations written this second.
    write_migration_once(
        app_path,
        "create_passkey_credentials",
        2,
        &passkeys::credentials_migration(),
    )?;
    append_routes_once(
        app_path,
        passkeys::ROUTES_MARKER,
        &passkeys::routes_snippet(),
        "passkeys",
    )?;
    Ok(())
}

/// Does the app's sign-in view already render the passkey partial?
fn sign_in_view_has_partial(app_path: &Path) -> bool {
    fs::read_to_string(app_path.join(SESSIONS_VIEW))
        .map(|source| source.contains("passkeys/sign_in"))
        .unwrap_or(false)
}

/// Next steps after `--passkeys`.
pub fn print_passkeys_success_message(folder: &str) {
    println!();
    println!("  \x1b[32m\x1b[1mPasskeys:\x1b[0m registration at \x1b[36m/passkeys\x1b[0m, sign-in from the login page.");
    if !sign_in_view_has_partial(Path::new(folder)) {
        println!();
        println!("  Your sign-in view was kept as it is. Add the passkey button to it:");
        println!("    \x1b[36m<%- partial(\"passkeys/sign_in\") %>\x1b[0m");
        println!("  and give the email field \x1b[36mautocomplete=\"username webauthn\"\x1b[0m for autofill.");
    }
    println!();
    println!(
        "  The relying party comes from \x1b[36mAPP_BASE_URL\x1b[0m: its host is the RP ID and"
    );
    println!("  the URL itself the only accepted origin. Override with \x1b[36mWEBAUTHN_RP_ID\x1b[0m and");
    println!("  \x1b[36mWEBAUTHN_ORIGINS\x1b[0m (comma-separated). Browsers allow passkeys on https:// and");
    println!("  on http://localhost only.");
    println!("  Docs: /docs/security/authentication#passkeys");
    println!();
}
