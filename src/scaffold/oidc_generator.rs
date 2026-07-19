//! `soli generate oidc_provider` — scaffold an OpenID Connect provider
//! (Authorization Code + PKCE, the OAuth 2.1 profile).

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::oidc;

/// Generate the OIDC provider scaffold into the application at `folder`.
pub fn create_oidc_provider(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);

    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", folder));
    }
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{}' does not look like a Soli app (no app/ directory). Run this inside a project created with `soli new`.",
            folder
        ));
    }

    // The provider signs tokens *about* a signed-in user, so it needs somewhere
    // to get one. Failing here beats emitting controllers that reference an
    // undefined `User` and only break when a request arrives.
    if !app_path.join("app/models/user.sl").exists()
        || !app_path.join("app/middleware/current_user.sl").exists()
    {
        return Err(
            "`soli generate oidc_provider` builds on the auth scaffold (User model + \
             current_user middleware).\n       Run `soli generate auth` first."
                .to_string(),
        );
    }

    ensure_directory_structure(app_path)?;

    // (relative path, contents) — written only if absent, so re-running never
    // clobbers a client model or consent screen you've customized.
    let files: [(&str, &str); 14] = [
        ("app/services/oidc_config.sl", oidc::OIDC_CONFIG),
        ("app/services/oidc_helper.sl", oidc::OIDC_HELPER),
        ("app/models/oauth_client.sl", oidc::OAUTH_CLIENT_MODEL),
        (
            "app/models/oauth_authorization_code.sl",
            oidc::OAUTH_AUTHORIZATION_CODE_MODEL,
        ),
        (
            "app/models/oauth_refresh_token.sl",
            oidc::OAUTH_REFRESH_TOKEN_MODEL,
        ),
        ("app/models/oauth_consent.sl", oidc::OAUTH_CONSENT_MODEL),
        (
            "app/models/oauth_revocation.sl",
            oidc::OAUTH_REVOCATION_MODEL,
        ),
        (
            "app/controllers/oidc_discovery_controller.sl",
            oidc::OIDC_DISCOVERY_CONTROLLER,
        ),
        (
            "app/controllers/oauth_authorizations_controller.sl",
            oidc::OAUTH_AUTHORIZATIONS_CONTROLLER,
        ),
        (
            "app/controllers/oauth_tokens_controller.sl",
            oidc::OAUTH_TOKENS_CONTROLLER,
        ),
        (
            "app/controllers/oauth_userinfo_controller.sl",
            oidc::OAUTH_USERINFO_CONTROLLER,
        ),
        (
            "app/controllers/oauth_sessions_controller.sl",
            oidc::OAUTH_SESSIONS_CONTROLLER,
        ),
        (
            "app/views/oauth_authorizations/new.html.slv",
            oidc::AUTHORIZATIONS_NEW_VIEW,
        ),
        (
            "app/views/oauth_authorizations/error.html.slv",
            oidc::AUTHORIZATIONS_ERROR_VIEW,
        ),
    ];

    for (rel, contents) in files {
        write_if_absent(app_path, rel, contents)?;
    }

    write_migration(
        app_path,
        "add_oauth_indexes",
        0,
        oidc::oauth_indexes_migration,
    )?;
    add_routes(app_path)?;

    Ok(())
}

fn ensure_directory_structure(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "app/models",
        "app/controllers",
        "app/services",
        "app/views/oauth_authorizations",
        "config",
        "db/migrations",
    ];
    for dir in dirs {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }
    Ok(())
}

/// Write `rel` under `app_path` unless it already exists (then warn + skip).
fn write_if_absent(app_path: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let path = app_path.join(rel);
    if path.exists() {
        println!("  \x1b[33mskip\x1b[0m   {} (already exists)", rel);
        return Ok(());
    }
    write_file(&path, contents)?;
    println!("  \x1b[32mcreate\x1b[0m {}", rel);
    Ok(())
}

/// Write a timestamped migration, skipping if one with the same name exists.
///
/// `offset` keeps migrations written in the same second ordered.
fn write_migration(
    app_path: &Path,
    name: &str,
    offset: u64,
    body: fn() -> String,
) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains(name) {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations ({} migration already exists)",
                    name
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs()
        + offset;
    let filename = format!("{}_{}.sl", timestamp, name);
    let path = migrations_dir.join(&filename);
    write_file(&path, &body())?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

/// Append the OIDC routes to `config/routes.sl` (idempotent).
fn add_routes(app_path: &Path) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");

    let mut content = if routes_file.exists() {
        fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?
    } else {
        String::new()
    };
    let existed = routes_file.exists();

    if content.contains("\"oauth_tokens#create\"") {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (OIDC routes already present)");
        return Ok(());
    }

    content.push_str(oidc::ROUTES_SNIPPET);
    fs::write(&routes_file, content).map_err(|e| format!("Failed to write routes file: {}", e))?;
    if existed {
        println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    } else {
        println!("  \x1b[32mcreate\x1b[0m config/routes.sl");
    }
    Ok(())
}

/// Print next-steps guidance after generation.
pub fn print_oidc_success_message() {
    println!();
    println!("  \x1b[32m\x1b[1mSuccess!\x1b[0m Scaffolded an OpenID Connect provider.");
    println!();
    println!("  \x1b[2m1. Generate a signing key pair\x1b[0m (never commit these):");
    println!(
        "     \x1b[36mopenssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out oidc.pem\x1b[0m"
    );
    println!("     \x1b[36mopenssl rsa -in oidc.pem -pubout -out oidc.pub.pem\x1b[0m");
    println!();
    println!("  \x1b[2m2. Point the app at them\x1b[0m (.env):");
    println!("     \x1b[36mSOLI_OIDC_ISSUER=https://id.example.com\x1b[0m");
    println!("     \x1b[36mSOLI_OIDC_PRIVATE_KEY=\"$(cat oidc.pem)\"\x1b[0m");
    println!("     \x1b[36mSOLI_OIDC_PUBLIC_KEY=\"$(cat oidc.pub.pem)\"\x1b[0m");
    println!();
    println!("  \x1b[2m3. Run the migrations:\x1b[0m  \x1b[36msoli db:migrate up\x1b[0m");
    println!();
    println!("  \x1b[2m4. Register a relying party\x1b[0m (prints the secret once):");
    println!("     \x1b[36mOauthClient.register(\"My App\", [\"https://app.example/callback\"], {{}})\x1b[0m");
    println!();
    println!("  \x1b[1mTwo edits to app/controllers/sessions_controller.sl\x1b[0m — the generator");
    println!("  does not touch files it did not create, so make them yourself:");
    println!();
    println!("    \x1b[2m# in `create`, next to session_regenerate():\x1b[0m");
    println!("    \x1b[32msession_set(\"auth_time\", DateTime.utc().to_unix())\x1b[0m");
    println!();
    println!("    \x1b[2m# and replace the final `return redirect(\"/\")` with:\x1b[0m");
    println!("    \x1b[32mdestination = session_get(\"oidc_return_to\") ?? \"/\"\x1b[0m");
    println!("    \x1b[32msession_delete(\"oidc_return_to\")\x1b[0m");
    println!("    \x1b[32mreturn redirect(destination)\x1b[0m");
    println!();
    println!("  Without the first, `auth_time` falls back to the authorization instant;");
    println!("  without the second, signing in mid-flow drops the user on the home page");
    println!("  instead of completing the authorization.");
    println!();
    println!(
        "  \x1b[2mSee /docs/security/oidc-provider for the full flow and key rotation.\x1b[0m"
    );
    println!();
}
