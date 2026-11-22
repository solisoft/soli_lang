//! `soli generate oauth <provider>` — OAuth *client* scaffold (sign in with GitHub, Google, GitLab, Discord, LinkedIn, Microsoft).
//!
//! Requires `soli generate auth` first (User + sessions). Idempotent: never
//! clobbers customized files; route/migration markers prevent duplicates.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::oauth;

/// Supported providers.
pub const PROVIDERS: [&str; 7] = [
    "apple",
    "github",
    "google",
    "gitlab",
    "discord",
    "linkedin",
    "microsoft",
];

pub fn normalize_provider(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "apple" => Ok("apple"),
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
        "app/views/oauth",
        "app/mailers",
        "app/views/oauth_mailer",
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
        "app/services/oauth_sso.sl",
        oauth::OAUTH_SSO_SERVICE,
    )?;
    write_if_absent(app_path, "app/mailers/oauth_mailer.sl", oauth::OAUTH_MAILER)?;
    write_if_absent(
        app_path,
        "app/views/oauth_mailer/provider_linked.html.slv",
        oauth::PROVIDER_LINKED_MAIL_VIEW,
    )?;
    write_if_absent(
        app_path,
        "app/views/oauth/reauth.html.slv",
        oauth::REAUTH_VIEW,
    )?;
    write_if_absent(
        app_path,
        "app/controllers/oauth_controller.sl",
        oauth::OAUTH_CONTROLLER,
    )?;
    write_if_absent(
        app_path,
        "app/views/oauth/identities.html.slv",
        oauth::IDENTITIES_VIEW,
    )?;

    // Per-provider service (always write the requested one; skip if present).
    let (service_file, service_source) = match provider {
        "apple" => ("apple_oauth.sl", oauth::APPLE_OAUTH_SERVICE),
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
    warn_about_predecessors(app_path);

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
    if controller_links_accounts(app_path) {
        append_routes_once(
            app_path,
            oauth::LINKING_ROUTES_MARKER,
            &oauth::linking_routes_snippet(),
            "oauth linking",
        )?;
    } else {
        println!(
            "  \x1b[33mnote\x1b[0m   app/controllers/oauth_controller.sl predates account linking and \
             Apple's posted answer; delete it and app/services/oauth_client.sl, then re-run this \
             command, to get /oauth/identities and `soli generate oauth apple`"
        );
    }
    if controller_has_reauth(app_path) {
        append_routes_once(
            app_path,
            oauth::REAUTH_ROUTES_MARKER,
            &oauth::reauth_routes_snippet(),
            "oauth reauth",
        )?;
    }
    if controller_has(app_path, "def link_provider") {
        append_routes_once(
            app_path,
            oauth::LINK_ROUTES_MARKER,
            &oauth::link_routes_snippet(),
            "oauth link",
        )?;
    }
    print_success(provider);
    Ok(())
}

/// Does the app's OAuth controller have the `/oauth/reauth` actions?
fn controller_has_reauth(app_path: &Path) -> bool {
    controller_has(app_path, "def reauthenticate")
}

/// Does the app's OAuth controller contain `needle`?
fn controller_has(app_path: &Path, needle: &str) -> bool {
    fs::read_to_string(app_path.join("app/controllers/oauth_controller.sl"))
        .map(|source| source.contains(needle))
        .unwrap_or(false)
}

/// Files an earlier `soli generate oauth` (or `auth`) wrote that the new
/// templates cannot work with.
fn warn_about_predecessors(app_path: &Path) {
    // The shared helpers were `class OauthClient` in app/services/oauth_client.sl:
    // the provider services written now call `OauthSso`, and the old name
    // clashes with the model `soli generate oidc_provider` writes.
    let old_service = app_path.join("app/services/oauth_client.sl");
    if let Ok(source) = fs::read_to_string(&old_service) {
        if source.contains("class OauthClient") && !source.contains("< Model") {
            println!(
                "  \x1b[33mnote\x1b[0m   app/services/oauth_client.sl is the old `OauthClient` service; \
                 delete it, app/services/*_oauth.sl and app/controllers/oauth_controller.sl, then \
                 re-run this command for each provider: they now use `OauthSso` (app/services/oauth_sso.sl)"
            );
        }
    }
    // Linking used to start from `GET /auth/:provider`: a cross-site link to it
    // from a recently signed-in user's browser began attaching a provider.
    let controller = app_path.join("app/controllers/oauth_controller.sl");
    if let Ok(source) = fs::read_to_string(&controller) {
        if source.contains("def identities") && !source.contains("def link_provider") {
            println!(
                "  \x1b[33mnote\x1b[0m   app/controllers/oauth_controller.sl starts linking a provider from \
                 GET /auth/:provider, which any site can make a signed-in browser send; delete it, \
                 app/services/oauth_sso.sl and app/views/oauth/identities.html.slv, then re-run this \
                 command: linking moves to POST /auth/:provider/link"
            );
        }
    }
    // OauthSso written before SSO-only accounts and auth_time: it links a
    // provider to a `soli generate saml` account by its address.
    if let Ok(source) = fs::read_to_string(app_path.join("app/services/oauth_sso.sl")) {
        if source.contains("class OauthSso") && !source.contains("def sso_only?") {
            println!(
                "  \x1b[33mnote\x1b[0m   app/services/oauth_sso.sl predates SSO-only accounts: it links a \
                 provider to an account that must sign in through its identity provider, and records no \
                 auth_time; replace it with the current version (delete it and re-run this command)"
            );
        }
    }
    // Microsoft identities were stored under the bare `sub`; they are
    // `<tid>:<sub>` now, and the current service adopts the old rows.
    if let Ok(source) = fs::read_to_string(app_path.join("app/services/microsoft_oauth.sl")) {
        if source.contains("class MicrosoftOauth") && !source.contains("adopt_legacy_identity") {
            println!(
                "  \x1b[33mnote\x1b[0m   app/services/microsoft_oauth.sl stores identities under the bare \
                 `sub`, which is unique only within one tenant. The current version stores `<tid>:<sub>` and \
                 rewrites an old row on its owner's next sign-in (when the tenant recorded with it matches, \
                 or in a single-tenant app); delete the file and run `soli generate oauth microsoft` — \
                 see /docs/security/oauth-client"
            );
        }
    }
    // Taking over an unconfirmed account revokes everything it can be entered
    // with through `User#revoke_credentials`, which `soli generate auth` writes.
    let user_model = fs::read_to_string(app_path.join("app/models/user.sl")).unwrap_or_default();
    if !user_model.contains("def revoke_credentials") {
        println!(
            "  \x1b[33mnote\x1b[0m   app/models/user.sl has no `revoke_credentials`: OauthSso.take_over_unconfirmed \
             calls it. Add it (it clears password_digest and remember_token_digest, bumps session_version and \
             deletes the user's OauthIdentity, passkey, API token, 2FA and magic-link rows) — see \
             /docs/security/oauth-client"
        );
    }
}

/// Does the app's OAuth controller have the linking actions? One written
/// before them does not, and routes to missing actions would only be 500s.
fn controller_links_accounts(app_path: &Path) -> bool {
    fs::read_to_string(app_path.join("app/controllers/oauth_controller.sl"))
        .map(|source| source.contains("def identities"))
        .unwrap_or(false)
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
    if provider == "apple" {
        println!(
            "    2. In the Apple developer console: a Services ID with \"Sign in with Apple\","
        );
        println!("       return URL /auth/apple/callback (https only), and a key for it (.p8)");
        println!("    3. Set in .env:");
        println!("         APPLE_CLIENT_ID=…          (the Services ID)");
        println!("         APPLE_TEAM_ID=…");
        println!("         APPLE_KEY_ID=…");
        println!(
            "         APPLE_PRIVATE_KEY_FILE=…   (the .p8, inside the app; or APPLE_PRIVATE_KEY with its contents)"
        );
        println!("         APPLE_REDIRECT_URI=https://your.app/auth/apple/callback");
        println!("    4. Link from login:  <a href=\"/auth/apple\">Sign in with Apple</a>");
        println!();
        println!("  Signed-in users connect and disconnect providers at /oauth/identities.");
        println!("  Docs: /docs/security/oauth-client");
        println!();
        return;
    }
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
            "Register an Entra ID app with the optional ID-token claims email, xms_edov and verified_primary_email (Token configuration); also set MICROSOFT_TENANT to your tenant id (a GUID), or to `organizations` with MICROSOFT_ALLOWED_TENANTS (a list of tenant ids)",
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
    println!("  Signed-in users connect and disconnect providers at /oauth/identities.");
    println!("  Docs: /docs/security/oauth-client");
    println!();
}
