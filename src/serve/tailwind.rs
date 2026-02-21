//! Tailwind CSS Integration
//!
//! This module handles Tailwind CSS compilation for development.

use std::path::Path;

/// Run a single Tailwind CSS compilation.
/// Returns true if compilation was successful.
pub(crate) fn compile_tailwind_css_once(folder: &Path) -> bool {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return false;
    }

    // Check for package.json with build:css script
    let package_json = folder.join("package.json");
    if !package_json.exists() {
        return false;
    }

    println!("Compiling Tailwind CSS...");

    let output = std::process::Command::new("npm")
        .arg("run")
        .arg("build:css")
        .current_dir(folder)
        .output();

    match output {
        Ok(result) => {
            if result.status.success() {
                println!("   ✓ Tailwind CSS compiled successfully");
                true
            } else {
                let stderr = String::from_utf8_lossy(&result.stderr);
                eprintln!("   ✗ Tailwind CSS compilation failed: {}", stderr);
                false
            }
        }
        Err(e) => {
            eprintln!("   ✗ Failed to run npm: {}", e);
            false
        }
    }
}
