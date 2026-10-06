//! `soli generate devices` — Device model, register endpoint, prune helper.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::{append_routes_once, write_if_absent, write_migration_once};
use crate::scaffold::templates::devices;

/// Generate the devices / push-target scaffold into `folder`.
pub fn create_devices(folder: &str) -> Result<(), String> {
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

    for dir in [
        "app/models",
        "app/controllers",
        "app/helpers",
        "config",
        "db/migrations",
    ] {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }

    write_if_absent(app_path, "app/models/device.sl", devices::DEVICE_MODEL)?;
    write_if_absent(
        app_path,
        "app/controllers/devices_controller.sl",
        devices::DEVICES_CONTROLLER,
    )?;
    write_if_absent(
        app_path,
        "app/helpers/devices_helper.sl",
        devices::DEVICES_HELPER,
    )?;
    write_migration_once(app_path, "create_devices", 0, &devices::devices_migration())?;
    append_routes_once(
        app_path,
        devices::ROUTES_MARKER,
        &devices::routes_snippet(),
        "devices",
    )?;
    Ok(())
}

pub fn print_devices_success_message() {
    println!();
    println!("  \x1b[32mDevices scaffold ready.\x1b[0m");
    println!();
    println!("  Next:");
    println!("    1. Run migrations:  \x1b[36msoli db:migrate up\x1b[0m");
    println!("    2. In the authenticated layout: \x1b[36m<%- csrf_meta_tag() %>\x1b[0m + native_channel");
    println!("    3. After login, register a token:");
    println!(
        "       \x1b[2msoli.nativeBridge.registerDevice({{ platform, token|subscription }})\x1b[0m"
    );
    println!("       Shells POST /devices with session cookie (skip_csrf + Origin).");
    println!("    4. Notify with \x1b[36mdeliver_to_user(...)\x1b[0m or Push.deliver + prune");
    println!();
    println!("  See \x1b[36m/docs/native/devices\x1b[0m");
    println!();
}
