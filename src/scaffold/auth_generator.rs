//! `soli generate auth` — scaffold session-based authentication (User model +
//! login/signup/logout) and a Pundit-style authorization Policy layer.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::auth;

/// Generate the auth scaffold into the application at `folder`.
pub fn create_auth(folder: &str) -> Result<(), String> {
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

    ensure_directory_structure(app_path)?;

    // (relative path, contents) — written only if absent so re-running never
    // clobbers a User model or policy you've already customized.
    let files: [(&str, &str); 17] = [
        ("app/models/user.sl", auth::USER_MODEL),
        (
            "app/policies/application_policy.sl",
            auth::APPLICATION_POLICY,
        ),
        ("app/policies/user_policy.sl", auth::USER_POLICY),
        ("app/policies/CLAUDE.md", auth::POLICIES_CLAUDE),
        ("app/helpers/auth_helper.sl", auth::AUTH_HELPER),
        (
            "app/middleware/current_user.sl",
            auth::CURRENT_USER_MIDDLEWARE,
        ),
        (
            "app/controllers/sessions_controller.sl",
            auth::SESSIONS_CONTROLLER,
        ),
        (
            "app/controllers/registrations_controller.sl",
            auth::REGISTRATIONS_CONTROLLER,
        ),
        (
            "app/controllers/passwords_controller.sl",
            auth::PASSWORDS_CONTROLLER,
        ),
        (
            "app/controllers/confirmations_controller.sl",
            auth::CONFIRMATIONS_CONTROLLER,
        ),
        ("app/mailers/auth_mailer.sl", auth::AUTH_MAILER),
        ("app/views/sessions/new.html.slv", auth::SESSIONS_NEW_VIEW),
        ("app/views/passwords/new.html.slv", auth::PASSWORDS_NEW_VIEW),
        (
            "app/views/passwords/edit.html.slv",
            auth::PASSWORDS_EDIT_VIEW,
        ),
        (
            "app/views/confirmations/new.html.slv",
            auth::CONFIRMATIONS_NEW_VIEW,
        ),
        (
            "app/views/auth_mailer/reset_password.html.slv",
            auth::MAILER_RESET_PASSWORD_VIEW,
        ),
        (
            "app/views/auth_mailer/confirm_email.html.slv",
            auth::MAILER_CONFIRM_EMAIL_VIEW,
        ),
    ];

    for (rel, contents) in files {
        write_if_absent(app_path, rel, contents)?;
    }
    write_if_absent(
        app_path,
        "app/views/registrations/new.html.slv",
        auth::REGISTRATIONS_NEW_VIEW,
    )?;

    write_migration(app_path)?;
    write_token_indexes_migration(app_path)?;
    add_routes(app_path)?;

    Ok(())
}

fn ensure_directory_structure(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "app/models",
        "app/policies",
        "app/controllers",
        "app/helpers",
        "app/mailers",
        "app/middleware",
        "app/views/sessions",
        "app/views/registrations",
        "app/views/passwords",
        "app/views/confirmations",
        "app/views/auth_mailer",
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

/// Write a timestamped `create_users` migration (skips if one already exists).
fn write_migration(app_path: &Path) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    // Don't add a second users migration if one is already present.
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains("create_users") {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations (create_users migration already exists)"
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();
    let filename = format!("{}_create_users.sl", timestamp);
    let path = migrations_dir.join(&filename);
    write_file(&path, &auth::users_migration())?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

/// Write the token-index migration (skips if already present). Separate from
/// `create_users` so apps generated before the Devise-style flows pick it up
/// on a re-run.
fn write_token_indexes_migration(app_path: &Path) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("add_auth_token_indexes")
            {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations (add_auth_token_indexes migration already exists)"
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();
    // +1 keeps it ordered after a create_users migration written this second.
    let filename = format!("{}_add_auth_token_indexes.sl", timestamp + 1);
    let path = migrations_dir.join(&filename);
    write_file(&path, &auth::auth_token_indexes_migration())?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

/// Append the auth routes to `config/routes.sl` (idempotent — the base and
/// flow snippets carry their own markers, so an app generated before the
/// Devise-style flows gains just the new routes on a re-run).
fn add_routes(app_path: &Path) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");

    let mut content = if routes_file.exists() {
        fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?
    } else {
        String::new()
    };
    let existed = routes_file.exists();

    let mut changed = false;
    if !content.contains("\"sessions#new\"") {
        content.push_str(auth::ROUTES_SNIPPET);
        changed = true;
    }
    if !content.contains("\"passwords#new\"") {
        content.push_str(auth::FLOWS_ROUTES_SNIPPET);
        changed = true;
    }

    if !changed {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (auth routes already present)");
        return Ok(());
    }

    fs::write(&routes_file, content).map_err(|e| format!("Failed to write routes file: {}", e))?;
    if existed {
        println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    } else {
        println!("  \x1b[32mcreate\x1b[0m config/routes.sl");
    }
    Ok(())
}

/// Print next-steps guidance after generation.
pub fn print_auth_success_message() {
    println!();
    println!("  \x1b[32m\x1b[1mSuccess!\x1b[0m Scaffolded authentication + policies.");
    println!();
    println!("  \x1b[2mNext steps:\x1b[0m");
    println!("    1. Run the migrations:  \x1b[36msoli db:migrate up\x1b[0m");
    println!("    2. Start the server:    \x1b[36msoli serve . --dev\x1b[0m");
    println!(
        "    3. Visit \x1b[36m/signup\x1b[0m to create an account, then \x1b[36m/login\x1b[0m."
    );
    println!();
    println!("  \x1b[2mIncluded flows:\x1b[0m password reset (/password/reset), email");
    println!("  confirmation (/confirmation/resend), remember-me, account lockout.");
    println!("  Configure SMTP (SOLI_SMTP_* env vars) so the emails go out, set your");
    println!("  production URL in \x1b[36mauth_base_url\x1b[0m, and tune the knobs at the top of");
    println!(
        "  \x1b[36mapp/models/user.sl\x1b[0m (lockout threshold, token TTLs, confirmation gate)."
    );
    println!();
    println!("  \x1b[2mAuthorize in a controller:\x1b[0m");
    println!(
        "    \x1b[36mauthorize(record)\x1b[0m  # 403 unless the matching <Model>Policy allows it"
    );
    println!();
    println!("  \x1b[2mSee app/policies/CLAUDE.md for the policy conventions.\x1b[0m");
    println!();
}
