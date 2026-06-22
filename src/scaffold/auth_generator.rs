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
    let files: [(&str, &str); 9] = [
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
        ("app/views/sessions/new.html.slv", auth::SESSIONS_NEW_VIEW),
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
    add_routes(app_path)?;

    Ok(())
}

fn ensure_directory_structure(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "app/models",
        "app/policies",
        "app/controllers",
        "app/helpers",
        "app/middleware",
        "app/views/sessions",
        "app/views/registrations",
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
    let filename = format!("{}create_users_{}.sl", timestamp, timestamp);
    let path = migrations_dir.join(&filename);
    write_file(&path, &auth::users_migration())?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

/// Append the auth routes to `config/routes.sl` (idempotent — skips if the
/// login route is already declared).
fn add_routes(app_path: &Path) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");

    if routes_file.exists() {
        let mut content = fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?;
        if content.contains("\"sessions#new\"") {
            println!("  \x1b[33mskip\x1b[0m   config/routes.sl (auth routes already present)");
            return Ok(());
        }
        content.push_str(auth::ROUTES_SNIPPET);
        fs::write(&routes_file, content)
            .map_err(|e| format!("Failed to write routes file: {}", e))?;
        println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    } else {
        write_file(&routes_file, auth::ROUTES_SNIPPET)?;
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
    println!("    1. Run the migration:   \x1b[36msoli db:migrate up\x1b[0m");
    println!("    2. Start the server:    \x1b[36msoli serve . --dev\x1b[0m");
    println!(
        "    3. Visit \x1b[36m/signup\x1b[0m to create an account, then \x1b[36m/login\x1b[0m."
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
