//! `soli generate offline` — sync push/pull + client outbox helper.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
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
    write_migration_once(
        app_path,
        "create_sync_events",
        0,
        &offline::sync_events_migration(),
    )?;
    append_routes_once(
        app_path,
        offline::ROUTES_MARKER,
        &offline::routes_snippet(),
        "offline",
    )?;
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
