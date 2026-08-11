//! `soli generate oauth <provider>` — OAuth *client* scaffold (sign in with GitHub/Google).
//!
//! Requires `soli generate auth` first (User + sessions). Idempotent: never
//! clobbers customized files; route/migration markers prevent duplicates.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::oauth;

/// Supported providers for v1.
pub fn normalize_provider(raw: &str) -> Result<&'static str, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "github" | "gh" => Ok("github"),
        "google" | "g" => Ok("google"),
        other => Err(format!(
            "Unknown OAuth provider {other:?}. Supported: github, google"
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
    match provider {
        "github" => {
            write_if_absent(
                app_path,
                "app/services/github_oauth.sl",
                oauth::GITHUB_OAUTH_SERVICE,
            )?;
        }
        "google" => {
            write_if_absent(
                app_path,
                "app/services/google_oauth.sl",
                oauth::GOOGLE_OAUTH_SERVICE,
            )?;
        }
        _ => unreachable!(),
    }

    write_migration(app_path)?;
    add_routes(app_path)?;
    print_success(provider);
    Ok(())
}

fn write_if_absent(app_path: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let path = app_path.join(rel);
    if path.exists() {
        println!("  \x1b[33mskip\x1b[0m   {rel} (already exists)");
        return Ok(());
    }
    write_file(&path, contents)?;
    println!("  \x1b[32mcreate\x1b[0m {rel}");
    Ok(())
}

fn write_migration(app_path: &Path) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("create_oauth_identities")
            {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations (create_oauth_identities already exists)"
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {e}"))?
        .as_secs();
    let filename = format!("{timestamp}_create_oauth_identities.sl");
    let path = migrations_dir.join(&filename);
    write_file(&path, &oauth::identities_migration())?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{filename}");
    Ok(())
}

fn add_routes(app_path: &Path) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");
    let mut content = if routes_file.exists() {
        fs::read_to_string(&routes_file).map_err(|e| format!("Failed to read routes file: {e}"))?
    } else {
        String::new()
    };

    if content.contains(oauth::ROUTES_MARKER) {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (oauth routes already present)");
        return Ok(());
    }

    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&oauth::routes_snippet());
    write_file(&routes_file, &content)?;
    println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    Ok(())
}

fn print_success(provider: &str) {
    println!();
    println!("  \x1b[32mOAuth client scaffold ready ({provider}).\x1b[0m");
    println!();
    println!("  Next:");
    println!("    1. soli db:migrate up");
    match provider {
        "github" => {
            println!("    2. Create a GitHub OAuth App → callback /auth/github/callback");
            println!("    3. Set in .env:");
            println!("         GITHUB_CLIENT_ID=…");
            println!("         GITHUB_CLIENT_SECRET=…");
            println!("         GITHUB_REDIRECT_URI=http://localhost:3000/auth/github/callback");
            println!("    4. Link from login:  <a href=\"/auth/github\">Sign in with GitHub</a>");
        }
        "google" => {
            println!("    2. Create a Google OAuth client → redirect /auth/google/callback");
            println!("    3. Set in .env:");
            println!("         GOOGLE_CLIENT_ID=…");
            println!("         GOOGLE_CLIENT_SECRET=…");
            println!("         GOOGLE_REDIRECT_URI=http://localhost:3000/auth/google/callback");
            println!("    4. Link from login:  <a href=\"/auth/google\">Sign in with Google</a>");
        }
        _ => {}
    }
    println!();
    println!("  Re-run with the other provider to add it (shared base is skipped).");
    println!("  Docs: /docs/security/oauth-client");
    println!();
}
