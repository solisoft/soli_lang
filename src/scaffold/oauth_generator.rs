//! `soli generate oauth <provider>` — OAuth *client* scaffold (sign in with GitHub, Google, GitLab, Discord, LinkedIn, Microsoft).
//!
//! Requires `soli generate auth` first (User + sessions). Idempotent: never
//! clobbers customized files; route/migration markers prevent duplicates.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::oauth;

/// Supported providers.
pub const PROVIDERS: [&str; 6] = [
    "github",
    "google",
    "gitlab",
    "discord",
    "linkedin",
    "microsoft",
];

pub fn normalize_provider(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "github" | "gh" => Ok("github"),
        "google" | "g" => Ok("google"),
        "gitlab" | "gl" => Ok("gitlab"),
        "discord" => Ok("discord"),
        "linkedin" | "li" => Ok("linkedin"),
        "microsoft" | "ms" | "azure" | "entra" => Ok("microsoft"),
        other => Err(format!(
            "Unknown OAuth provider {other:?}. Supported: {}",
            PROVIDERS.join(", ")
        )),
    }
}

/// Generate the OAuth client scaffold into the application at `folder`.
pub fn create_oauth(folder: &str, provider: &str) -> Result<(), String> {
    let provider = normalize_provider(provider)?;
    let app_path = Path::new(folder);

    if !app_path.exists() {
        return Err(format!("Directory '{folder}' does not exist"));
    }
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{folder}' does not look like a Soli app (no app/ directory). \
             Run this inside a project created with `soli new`."
        ));
    }

    // Precondition: session auth (User model).
    if !app_path.join("app/models/user.sl").exists() {
        return Err(
            "OAuth client requires `soli generate auth` first (missing app/models/user.sl).".into(),
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

    // Shared base (once).
    write_if_absent(
        app_path,
        "app/models/oauth_identity.sl",
        oauth::OAUTH_IDENTITY_MODEL,
    )?;
    write_if_absent(
        app_path,
        "app/services/oauth_client.sl",
        oauth::OAUTH_CLIENT_SERVICE,
    )?;
    write_if_absent(
        app_path,
        "app/controllers/oauth_controller.sl",
        oauth::OAUTH_CONTROLLER,
    )?;

    // Per-provider service (always write the requested one; skip if present).
    let (service_file, service_source) = match provider {
        "github" => ("github_oauth.sl", oauth::GITHUB_OAUTH_SERVICE),
        "google" => ("google_oauth.sl", oauth::GOOGLE_OAUTH_SERVICE),
        "gitlab" => ("gitlab_oauth.sl", oauth::GITLAB_OAUTH_SERVICE),
        "discord" => ("discord_oauth.sl", oauth::DISCORD_OAUTH_SERVICE),
        "linkedin" => ("linkedin_oauth.sl", oauth::LINKEDIN_OAUTH_SERVICE),
        "microsoft" => ("microsoft_oauth.sl", oauth::MICROSOFT_OAUTH_SERVICE),
        _ => unreachable!(),
    };
    write_if_absent(
        app_path,
        &format!("app/services/{service_file}"),
        service_source,
    )?;
    warn_if_controller_is_hardcoded(app_path);

    write_migration_once(
        app_path,
        "create_oauth_identities",
        0,
        &oauth::identities_migration(),
    )?;
    append_routes_once(
        app_path,
        oauth::ROUTES_MARKER,
        &oauth::routes_snippet(),
        "oauth",
    )?;
    print_success(provider);
    Ok(())
}

/// An app generated before providers were looked up by name has a controller
/// with `if provider == "github"` chains; it will 404 on a newly added provider
/// until it is replaced.
fn warn_if_controller_is_hardcoded(app_path: &Path) {
    let controller = app_path.join("app/controllers/oauth_controller.sl");
    if let Ok(source) = fs::read_to_string(&controller) {
        if !source.contains("oauth_service_for") {
            println!(
                "  \x1b[33mnote\x1b[0m   app/controllers/oauth_controller.sl dispatches on a fixed provider list; \
                 delete it and re-run this command to get the version that finds providers by name"
            );
        }
    }
}

fn print_success(provider: &str) {
    println!();
    println!("  \x1b[32mOAuth client scaffold ready ({provider}).\x1b[0m");
    println!();
    println!("  Next:");
    println!("    1. soli db:migrate up");
    let (label, env_prefix, note) = match provider {
        "github" => ("GitHub", "GITHUB", "Create a GitHub OAuth App"),
        "google" => ("Google", "GOOGLE", "Create a Google OAuth client"),
        "gitlab" => (
            "GitLab",
            "GITLAB",
            "Create a GitLab application (scopes: openid email profile); GITLAB_BASE_URL for self-hosted",
        ),
        "discord" => ("Discord", "DISCORD", "Create a Discord application (OAuth2)"),
        "linkedin" => (
            "LinkedIn",
            "LINKEDIN",
            "Create a LinkedIn app with \"Sign In with LinkedIn using OpenID Connect\"",
        ),
        "microsoft" => (
            "Microsoft",
            "MICROSOFT",
            "Register an Entra ID app; also set MICROSOFT_TENANT to your tenant (multi-tenant endpoints are refused)",
        ),
        _ => (provider, "PROVIDER", "Create an OAuth application"),
    };
    println!("    2. {note} → redirect /auth/{provider}/callback");
    println!("    3. Set in .env:");
    println!("         {env_prefix}_CLIENT_ID=…");
    println!("         {env_prefix}_CLIENT_SECRET=…");
    println!("         {env_prefix}_REDIRECT_URI=http://localhost:3000/auth/{provider}/callback");
    println!("    4. Link from login:  <a href=\"/auth/{provider}\">Sign in with {label}</a>");
    println!();
    println!("  Re-run with another provider to add it (shared base is skipped).");
    println!("  Docs: /docs/security/oauth-client");
    println!();
}
