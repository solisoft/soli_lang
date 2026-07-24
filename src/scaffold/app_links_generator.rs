//! `soli generate app_links` — well-known host proof files for deep links.

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::app_links_gen;

pub struct AppLinksOptions {
    pub android_package: String,
    pub android_sha256: String,
    pub apple_app_id: String,
    pub paths: Vec<String>,
}

impl Default for AppLinksOptions {
    fn default() -> Self {
        Self {
            android_package: "net.example.app".to_string(),
            android_sha256: "00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00:00".to_string(),
            apple_app_id: "TEAMID.net.example.app".to_string(),
            paths: vec!["*".to_string()],
        }
    }
}

pub fn create_app_links(folder: &str, opts: &AppLinksOptions) -> Result<(), String> {
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

    let controllers = app_path.join("app/controllers");
    if !controllers.exists() {
        fs::create_dir_all(&controllers)
            .map_err(|e| format!("Failed to create controllers: {}", e))?;
    }

    let paths_lit = if opts.paths.is_empty() {
        r#"["*"]"#.to_string()
    } else {
        let inner = opts
            .paths
            .iter()
            .map(|p| format!("\"{}\"", p.replace('"', "")))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{}]", inner)
    };

    let body = app_links_gen::WELL_KNOWN_CONTROLLER
        .replace("{{ANDROID_PACKAGE}}", &opts.android_package)
        .replace("{{ANDROID_SHA256}}", &opts.android_sha256)
        .replace("{{APPLE_APP_ID}}", &opts.apple_app_id)
        .replace("{{APPLE_PATHS}}", &paths_lit);

    let rel = "app/controllers/well_known_controller.sl";
    let path = app_path.join(rel);
    if path.exists() {
        println!("  \x1b[33mskip\x1b[0m   {} (already exists)", rel);
    } else {
        write_file(&path, &body)?;
        println!("  \x1b[32mcreate\x1b[0m {}", rel);
    }

    add_routes(app_path)?;
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

    if content.contains(app_links_gen::ROUTES_MARKER) {
        println!("  \x1b[33mskip\x1b[0m   config/routes.sl (app_links routes already present)");
        return Ok(());
    }

    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(&app_links_gen::routes_snippet());
    write_file(&routes_file, &content)?;
    println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    Ok(())
}

pub fn print_app_links_success_message() {
    println!();
    println!("  \x1b[32mApp Links scaffold ready.\x1b[0m");
    println!();
    println!("  Set ENV (or edit the controller defaults):");
    println!("    ANDROID_PACKAGE, ANDROID_CERT_SHA256, APPLE_APP_ID");
    println!("  Serve over HTTPS with no redirect on the Apple path.");
    println!("  See \x1b[36m/docs/native/deep-links\x1b[0m");
    println!();
}
