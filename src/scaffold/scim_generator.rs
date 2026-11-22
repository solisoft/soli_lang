//! `soli generate scim` — a SCIM 2.0 server, so an organisation's directory
//! (Okta, Microsoft Entra ID) creates, updates and deactivates the app's
//! users. A controller, a middleware that keeps deactivated accounts out, a
//! migration and routes; no file of the auth scaffold is edited.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::scim;

/// Generate the SCIM files into an app that has the auth scaffold.
pub fn create_scim(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "SCIM requires `soli generate auth` first (missing app/models/user.sl).".into(),
        );
    }
    for dir in [
        "app/controllers",
        "app/middleware",
        "app/models",
        "config",
        "db/migrations",
    ] {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }
    write_if_absent(
        app_path,
        "app/controllers/scim_controller.sl",
        scim::SCIM_CONTROLLER,
    )?;
    write_if_absent(
        app_path,
        "app/middleware/scim_active.sl",
        scim::SCIM_ACTIVE_MIDDLEWARE,
    )?;
    write_if_absent(
        app_path,
        "app/controllers/scim_groups_controller.sl",
        scim::SCIM_GROUPS_CONTROLLER,
    )?;
    write_if_absent(app_path, "app/models/scim_group.sl", scim::SCIM_GROUP_MODEL)?;
    write_migration_once(
        app_path,
        "add_scim_user_indexes",
        0,
        &scim::users_migration(),
    )?;
    append_routes_once(
        app_path,
        scim::ROUTES_MARKER,
        &scim::routes_snippet(),
        "scim",
    )
}

/// Next steps after `soli generate scim`.
pub fn print_scim_success_message() {
    println!();
    println!("  \x1b[32m\x1b[1mSCIM 2.0:\x1b[0m the directory provisions users at \x1b[36m/scim/v2\x1b[0m.");
    println!();
    println!("  1. \x1b[36msoli db:migrate up\x1b[0m");
    println!("  2. Give the directory a token of its own, 32+ characters:");
    println!("       \x1b[36mSCIM_TOKEN=…\x1b[0m   or   \x1b[36mSCIM_TOKENS=okta=…,entra=…\x1b[0m");
    println!("  3. In the directory: base URL \x1b[36mAPP_BASE_URL/scim/v2\x1b[0m, bearer token");
    println!("     authentication, `userName` mapped to the email.");
    println!();
    println!("  A directory reaches the accounts it provisioned, never an admin or a local");
    println!("  account. \x1b[36mSCIM_ADOPT_EXISTING=on\x1b[0m lets it adopt one (its credentials are revoked).");
    println!("  A deactivated user is signed out and kept out by app/middleware/scim_active.sl.");
    println!("  Groups: read them with \x1b[36mScimGroup.names_for(user)\x1b[0m, or map them to roles in");
    println!("  \x1b[36mSCIM_GROUP_ROLES\x1b[0m (app/models/scim_group.sl). A directory never grants `admin`.");
    println!("  Docs: /docs/security/scim");
    println!();
}
