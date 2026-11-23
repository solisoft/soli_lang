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

    // A `Device` model of the app's own (a fleet, a sensor) is not the one
    // this controller and migration are written against: `Device.register`
    // is not there, and the migration would index its collection.
    let model_path = app_path.join("app/models/device.sl");
    if let Ok(existing) = fs::read_to_string(&model_path) {
        if !existing.contains("push_targets_for") {
            return Err(
                "app/models/device.sl is not the model `soli generate devices` writes: \
                 rename the app's own Device model, or write the push devices by hand."
                    .to_string(),
            );
        }
    }
    write_if_absent(app_path, "app/models/device.sl", devices::DEVICE_MODEL)?;
    name_the_devices_model(app_path)?;
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

/// A model written by an earlier `soli generate devices` does not name
/// itself (`PUSH_DEVICES_MODEL`), and the auth scaffold's password reset
/// then leaves its rows alone. Add the constant to it — to the generated
/// model only: a file that is not `class Device < Model` is the app's own.
fn name_the_devices_model(app_path: &Path) -> Result<(), String> {
    let path = app_path.join("app/models/device.sl");
    let Ok(model) = fs::read_to_string(&path) else {
        return Ok(());
    };
    if model.contains("PUSH_DEVICES_MODEL") || !model.contains("push_targets_for") {
        return Ok(());
    }
    // Before the class header itself — the line that begins with it — not
    // the first place those words appear: a comment may quote them.
    let mut named = String::with_capacity(model.len() + 48);
    let mut done = false;
    for line in model.split_inclusive('\n') {
        if !done && line.starts_with("class Device < Model") {
            named.push_str("const PUSH_DEVICES_MODEL = \"Device\"\n\n");
            done = true;
        }
        named.push_str(line);
    }
    if !done {
        return Ok(());
    }
    fs::write(&path, named).map_err(|e| format!("Failed to update '{}': {}", path.display(), e))
}
