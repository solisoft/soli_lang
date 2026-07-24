//! `soli generate offline` — sync push/pull + client outbox helper.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::offline;

pub fn create_offline(folder: &str) -> Result<(), String> {
    let app_path = Path::new(folder);
    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", folder));
    }
    if !app_path.join("app").is_dir() {
        return Err(format!(
            "'{}' does not look like a Soli app (no app/ directory).",
            folder
        ));
    }

    for dir in [
        "app/models",
        "app/controllers",
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

    write_if_absent(
        app_path,
        "app/models/sync_event.sl",
        offline::SYNC_EVENT_MODEL,
    )?;
    write_if_absent(
        app_path,
        "app/controllers/sync_controller.sl",
        offline::SYNC_CONTROLLER,
    )?;
    write_if_absent(
        app_path,
        "public/js/soli_outbox.js",
        offline::CLIENT_OUTBOX_JS,
    )?;
    write_migration(app_path)?;
    add_routes(app_path)?;
    Ok(())
}

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

fn write_migration(app_path: &Path) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_string_lossy()
                .contains("create_sync_events")
            {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations (create_sync_events already exists)"
                );
                return Ok(());
            }
        }
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();
    let filename = format!("{}_create_sync_events.sl", timestamp);
    write_file(
        &migrations_dir.join(&filename),
        &offline::sync_events_migration(),
    )?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

fn add_routes(app_path: &Path) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");
    let mut content = if routes_file.exists() {
        fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?
    } else {
        String::new()
    };
    if content.contains(offline::ROUTES_MARKER) {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (offline routes already present)");
        return Ok(());
    }
    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&offline::routes_snippet());
    write_file(&routes_file, &content)?;
    println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    Ok(())
}

pub fn print_offline_success_message() {
    println!();
    println!("  \x1b[32mOffline sync scaffold ready.\x1b[0m");
    println!();
    println!("  Include the outbox helper in a layout:");
    println!("    \x1b[36m<script src=\"/js/soli_outbox.js\"></script>\x1b[0m");
    println!("  Enqueue writes:  \x1b[36msoliOutbox.enqueue({{ method, path, body }})\x1b[0m");
    println!("  Flush on online: \x1b[36mawait soliOutbox.flush()\x1b[0m");
    println!("  See \x1b[36m/docs/native/offline\x1b[0m");
    println!();
}
