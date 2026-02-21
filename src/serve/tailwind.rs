//! Tailwind CSS Integration
//!
//! This module handles Tailwind CSS compilation for development.
//! Uses the standalone Tailwind CSS CLI binary, downloading it automatically if needed.

use std::path::{Path, PathBuf};

/// Platform-specific Tailwind standalone CLI download URL and binary name
fn tailwind_binary_info() -> (&'static str, &'static str) {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        (
            "https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-arm64",
            "tailwindcss-macos-arm64",
        )
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        (
            "https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-macos-x64",
            "tailwindcss-macos-x64",
        )
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        (
            "https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-x64",
            "tailwindcss-linux-x64",
        )
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        (
            "https://github.com/tailwindlabs/tailwindcss/releases/latest/download/tailwindcss-linux-arm64",
            "tailwindcss-linux-arm64",
        )
    }
    // Fallback for other platforms
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        ("", "tailwindcss")
    }
}

/// Get the path where the standalone Tailwind binary is cached.
/// Stored in ~/.soli/bin/
fn tailwind_cache_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".soli").join("bin"))
}

/// Download the standalone Tailwind CSS CLI binary.
fn download_tailwind_binary(dest: &Path) -> bool {
    let (url, _) = tailwind_binary_info();
    if url.is_empty() {
        eprintln!("   ✗ No standalone Tailwind binary available for this platform");
        return false;
    }

    println!("   Downloading Tailwind CSS standalone CLI...");

    // Create parent directory
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("   ✗ Failed to create directory: {}", e);
            return false;
        }
    }

    // Use curl (available on macOS and Linux) to follow redirects and download
    let output = std::process::Command::new("curl")
        .args(["-sL", "-o"])
        .arg(dest)
        .arg(url)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            // Make executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755));
            }
            println!("   ✓ Tailwind CSS CLI downloaded");
            true
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            eprintln!("   ✗ Download failed: {}", stderr);
            false
        }
        Err(e) => {
            eprintln!("   ✗ Failed to run curl: {}", e);
            false
        }
    }
}

/// Find or download the Tailwind CSS binary.
/// Priority: local node_modules > cached standalone > download standalone
fn find_tailwind_binary(folder: &Path) -> Option<PathBuf> {
    // 1. Check local node_modules/.bin/tailwindcss
    let local_bin = folder.join("node_modules/.bin/tailwindcss");
    if local_bin.exists() {
        return Some(local_bin);
    }

    // 2. Check cached standalone binary in ~/.soli/bin/
    let (_, binary_name) = tailwind_binary_info();
    if let Some(cache_dir) = tailwind_cache_dir() {
        let cached = cache_dir.join(binary_name);
        if cached.exists() {
            return Some(cached);
        }

        // 3. Download standalone binary
        if download_tailwind_binary(&cached) {
            return Some(cached);
        }
    }

    None
}

/// Compile all CSS files in app/assets/css/ to public/css/.
/// Each `app/assets/css/foo.css` is compiled to `public/css/foo.css`.
/// Returns true if all compilations were successful.
pub(crate) fn compile_tailwind_css_once(folder: &Path) -> bool {
    let tailwind_config = folder.join("tailwind.config.js");
    if !tailwind_config.exists() {
        return false;
    }

    let assets_dir = folder.join("app/assets/css");
    if !assets_dir.exists() {
        return false;
    }

    // Collect all .css files in app/assets/css/
    let css_files: Vec<_> = match std::fs::read_dir(&assets_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "css")
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return false,
    };

    if css_files.is_empty() {
        return false;
    }

    let output_dir = folder.join("public/css");
    let _ = std::fs::create_dir_all(&output_dir);

    println!("Compiling Tailwind CSS...");

    let tailwind_bin = match find_tailwind_binary(folder) {
        Some(bin) => bin,
        None => {
            eprintln!("   ✗ Tailwind CSS CLI not found. Run 'npm install' in your project or check your internet connection.");
            return false;
        }
    };

    let mut all_ok = true;
    for entry in &css_files {
        let input = entry.path();
        let filename = input.file_name().unwrap();
        let output = output_dir.join(filename);

        let result = std::process::Command::new(&tailwind_bin)
            .arg("-i")
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .current_dir(folder)
            .output();

        match result {
            Ok(r) if r.status.success() => {
                println!("   ✓ {}", filename.to_string_lossy());
            }
            Ok(r) => {
                let stderr = String::from_utf8_lossy(&r.stderr);
                eprintln!("   ✗ {} failed: {}", filename.to_string_lossy(), stderr);
                all_ok = false;
            }
            Err(e) => {
                eprintln!("   ✗ {} failed: {}", filename.to_string_lossy(), e);
                all_ok = false;
            }
        }
    }

    all_ok
}
