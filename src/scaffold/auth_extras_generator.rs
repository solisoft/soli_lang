//! `soli generate auth --magic-link | --api-tokens | --impersonation` — three
//! additions to the session auth scaffold. Each adds files of its own and
//! edits none: it runs the same on a fresh app and on one that customized
//! what `soli generate auth` wrote.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::auth_extras as templates;

/// The app at `folder`, which must have the auth scaffold; `dirs` exist after.
fn prepare<'a>(folder: &'a str, what: &str, dirs: &[&str]) -> Result<&'a Path, String> {
    let app_path = Path::new(folder);
    if !app_path.join("app/models/user.sl").exists() {
        return Err(format!(
            "{what} requires the auth scaffold (missing app/models/user.sl): run `soli generate auth` first."
        ));
    }
    for dir in dirs {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }
    Ok(app_path)
}

/// Sign in with a link sent by email.
pub fn create_magic_link(folder: &str) -> Result<(), String> {
    let app_path = prepare(
        folder,
        "Magic links",
        &[
            "app/models",
            "app/controllers",
            "app/mailers",
            "app/views/magic_links",
            "app/views/magic_link_mailer",
            "db/migrations",
        ],
    )?;
    for (rel, contents) in [
        ("app/models/magic_link.sl", templates::MAGIC_LINK_MODEL),
        (
            "app/controllers/magic_links_controller.sl",
            templates::MAGIC_LINKS_CONTROLLER,
        ),
        (
            "app/mailers/magic_link_mailer.sl",
            templates::MAGIC_LINK_MAILER,
        ),
        (
            "app/views/magic_links/new.html.slv",
            templates::MAGIC_LINKS_NEW_VIEW,
        ),
        (
            "app/views/magic_link_mailer/sign_in.html.slv",
            templates::MAGIC_LINK_MAIL_VIEW,
        ),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }
    write_migration_once(
        app_path,
        "create_magic_links",
        0,
        &templates::magic_links_migration(),
    )?;
    append_routes_once(
        app_path,
        templates::MAGIC_LINK_ROUTES_MARKER,
        &templates::magic_link_routes(),
        "magic link",
    )
}

/// Personal access tokens, and the middleware that authenticates them.
pub fn create_api_tokens(folder: &str) -> Result<(), String> {
    let app_path = prepare(
        folder,
        "API tokens",
        &[
            "app/models",
            "app/controllers",
            "app/middleware",
            "app/views/api_tokens",
            "db/migrations",
        ],
    )?;
    for (rel, contents) in [
        ("app/models/api_token.sl", templates::API_TOKEN_MODEL),
        (
            "app/middleware/api_token.sl",
            templates::API_TOKEN_MIDDLEWARE,
        ),
        (
            "app/controllers/api_tokens_controller.sl",
            templates::API_TOKENS_CONTROLLER,
        ),
        (
            "app/views/api_tokens/index.html.slv",
            templates::API_TOKENS_INDEX_VIEW,
        ),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }
    write_migration_once(
        app_path,
        "create_api_tokens",
        0,
        &templates::api_tokens_migration(),
    )?;
    append_routes_once(
        app_path,
        templates::API_TOKENS_ROUTES_MARKER,
        &templates::api_tokens_routes(),
        "API tokens",
    )
}

/// An administrator acting as a user.
pub fn create_impersonation(folder: &str) -> Result<(), String> {
    let app_path = prepare(
        folder,
        "Impersonation",
        &["app/controllers", "app/helpers", "app/views/impersonations"],
    )?;
    for (rel, contents) in [
        (
            "app/controllers/impersonations_controller.sl",
            templates::IMPERSONATIONS_CONTROLLER,
        ),
        (
            "app/helpers/impersonation_helper.sl",
            templates::IMPERSONATION_HELPER,
        ),
        (
            "app/views/impersonations/_banner.html.slv",
            templates::IMPERSONATION_BANNER,
        ),
    ] {
        write_if_absent(app_path, rel, contents)?;
    }
    append_routes_once(
        app_path,
        templates::IMPERSONATION_ROUTES_MARKER,
        &templates::impersonation_routes(),
        "impersonation",
    )
}

/// Next steps after the flags that were given.
pub fn print_auth_extras_success_message(magic_link: bool, api_tokens: bool, impersonation: bool) {
    println!();
    if magic_link {
        println!("  \x1b[32m\x1b[1mMagic link:\x1b[0m sign-in by email at \x1b[36m/login/link\x1b[0m. Link to it from your");
        println!("  sign-in view, and set \x1b[36mAPP_BASE_URL\x1b[0m and mail delivery so the link goes out.");
    }
    if api_tokens {
        println!("  \x1b[32m\x1b[1mAPI tokens:\x1b[0m users create them at \x1b[36m/api_tokens\x1b[0m; a request with");
        println!("  \x1b[36mAuthorization: Bearer …\x1b[0m runs as its user. Guard actions with");
        println!(
            "  \x1b[36mapi_scope?(\"write\")\x1b[0m; the scopes are in app/models/api_token.sl."
        );
    }
    if impersonation {
        println!("  \x1b[32m\x1b[1mImpersonation:\x1b[0m an admin posts to \x1b[36m/impersonate/:id\x1b[0m to act as a user.");
        println!("  Render the banner in your layout: \x1b[36m<%- partial(\"impersonations/banner\") %>\x1b[0m");
    }
    if magic_link || api_tokens {
        println!("  Run \x1b[36msoli db:migrate up\x1b[0m.");
    }
    println!("  Docs: /docs/security/authentication");
    println!();
}
