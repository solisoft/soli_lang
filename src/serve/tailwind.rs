//! Tailwind CSS Integration
//!
//! This module handles Tailwind CSS compilation and watch mode for development.

use std::path::Path;

/// Spawn Tailwind in watch mode as a background process.
/// Returns the child process handle if successful.
pub(crate) fn spawn_tailwind_watch(folder: &Path) -> Option<std::process::Child> {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return None;
    }

    let package_json = folder.join("package.json");
    if !package_json.exists() {
        return None;
    }

    println!("Starting Tailwind CSS in watch mode...");

    // Use npx tailwindcss directly for watch mode
    match std::process::Command::new("npx")
        .args([
            "tailwindcss",
            "-i",
            "./app/assets/css/application.css",
            "-o",
            "./public/css/application.css",
            "--watch",
        ])
        .current_dir(folder)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => {
            println!(
                "   ✓ Tailwind CSS watch process started (PID: {})",
                child.id()
            );
            Some(child)
        }
        Err(e) => {
            eprintln!("   ✗ Failed to start Tailwind watch: {}", e);
            // Fall back to single compilation
            compile_tailwind_css_once(folder);
            None
        }
    }
}

/// Run a single Tailwind CSS compilation (fallback/initial).
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

/// Touch the input CSS file to trigger Tailwind's watch mode.
pub(crate) fn trigger_tailwind_rebuild(folder: &Path) {
    let input_css = folder.join("app/assets/css/application.css");
    if input_css.exists() {
        // Touch file by reading and rewriting (works cross-platform)
        if let Ok(content) = std::fs::read(&input_css) {
            let _ = std::fs::write(&input_css, content);
        }
    }
}
