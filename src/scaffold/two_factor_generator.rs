//! `soli generate auth --two-factor` — an authenticator app (TOTP) as a
//! second factor on top of the session auth scaffold.
//!
//! Everything it adds is new: a model, a middleware that gates the session, a
//! controller, four views, a migration and routes. No file of the auth
//! scaffold is edited, so it runs the same on a fresh app and on one that ran
//! `soli generate auth` long ago and customized what it wrote.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::two_factor;

/// Generate the two-factor files into an app that has the auth scaffold.
pub fn create_two_factor(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "Two-factor requires the auth scaffold (missing app/models/user.sl): run `soli generate auth --two-factor`."
                .into(),
        );
    }

    for dir in [
        "app/models",
        "app/controllers",
        "app/middleware",
        "app/views/two_factor",
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
        ("app/models/two_factor.sl", two_factor::TWO_FACTOR_MODEL),
        (
            "app/middleware/two_factor.sl",
            two_factor::TWO_FACTOR_MIDDLEWARE,
        ),
        (
            "app/controllers/two_factor_controller.sl",
            two_factor::TWO_FACTOR_CONTROLLER,
        ),
        (
            "app/views/two_factor/index.html.slv",
            two_factor::INDEX_VIEW,
        ),
        (
            "app/views/two_factor/setup.html.slv",
            two_factor::SETUP_VIEW,
        ),
        (
            "app/views/two_factor/backup_codes.html.slv",
            two_factor::BACKUP_CODES_VIEW,
        ),
        (
            "app/views/two_factor/challenge.html.slv",
            two_factor::CHALLENGE_VIEW,
        ),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }

    // +3 orders it after the auth and passkey migrations written this second.
    write_migration_once(
        app_path,
        "create_two_factors",
        3,
        &two_factor::two_factors_migration(),
    )?;
    append_routes_once(
        app_path,
        two_factor::ROUTES_MARKER,
        &two_factor::routes_snippet(),
        "two-factor",
    )?;
    Ok(())
}

/// Does the app's passkey sign-in mark the session as verified? A scaffold
/// written before `--two-factor` existed does not.
fn passkey_sign_in_skips_the_challenge(app_path: &Path) -> Option<bool> {
    fs::read_to_string(app_path.join("app/controllers/passkey_sessions_controller.sl"))
        .ok()
        .map(|source| source.contains("session_set(\"two_factor\""))
}

/// Next steps after `--two-factor`.
pub fn print_two_factor_success_message(folder: &str) {
    println!();
    println!("  \x1b[32m\x1b[1mTwo-factor:\x1b[0m users turn it on at \x1b[36m/two_factor\x1b[0m; sign-in then asks for a code.");
    println!();
    println!("  Run \x1b[36msoli db:migrate up\x1b[0m, and set \x1b[36mSOLI_ENCRYPTION_KEY\x1b[0m (the secret is stored");
    println!("  encrypted): \x1b[36msoli -e 'print(Crypto.random_hex(32))'\x1b[0m. \x1b[36mTWO_FACTOR_ISSUER\x1b[0m is the");
    println!("  name authenticator apps show.");
    println!();
    println!("  The gate is app/middleware/two_factor.sl: it covers password, password");
    println!("  reset, remember-me and OAuth sign-ins. Link to /two_factor from your");
    println!("  account page.");
    if passkey_sign_in_skips_the_challenge(Path::new(folder)) == Some(false) {
        println!();
        println!("  Your passkey sign-in was kept as it is, so it is followed by the code");
        println!("  challenge. A passkey already is two factors; to skip the challenge, add");
        println!("  to `create` in app/controllers/passkey_sessions_controller.sl, after");
        println!("  session_set(\"user_id\", …):");
        println!("    \x1b[36msession_set(\"two_factor\", \"ok:#{{user._key}}\") if result[\"user_verified\"]\x1b[0m");
    }
    println!("  Docs: /docs/security/authentication#two-factor");
    println!();
}
