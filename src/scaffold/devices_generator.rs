//! `soli generate devices` — Device model, register endpoint, prune helper.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
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
                .contains("create_devices")
            {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations (create_devices migration already exists)"
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();
    let filename = format!("{}_create_devices.sl", timestamp);
    let path = migrations_dir.join(&filename);
    write_file(&path, &devices::devices_migration())?;
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

    if content.contains(devices::ROUTES_MARKER) {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (devices routes already present)");
        return Ok(());
    }

    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&devices::routes_snippet());
    write_file(&routes_file, &content)?;
    println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
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
