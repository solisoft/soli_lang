//! Application scaffolding generator

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use crate::scaffold::templates::app;
use crate::scaffold::ui::{ProgressDisplay, Spinner};

/// Create directories for a new application
pub fn create_directories(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "",
        "app",
        "app/controllers",
        "app/helpers",
        "app/middleware",
        "app/models",
        "app/views",
        "app/views/home",
        "app/views/layouts",
        "config",
        "db",
        "db/migrations",
        "app/assets",
        "app/assets/css",
        "public",
        "public/css",
        "public/js",
        "public/images",
        "stdlib",
        "tests",
    ];

    for dir in dirs {
        let path = app_path.join(dir);
        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
    }

    Ok(())
}

/// Write content to a file
pub fn write_file(path: &Path, content: &str) -> Result<(), String> {
    let mut file =
        File::create(path).map_err(|e| format!("Failed to create '{}': {}", path.display(), e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write to '{}': {}", path.display(), e))?;
    Ok(())
}

/// Create the routes configuration file
pub fn create_routes_file(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join("config/routes.sl"), app::ROUTES_TEMPLATE)
}

/// Create the home controller
pub fn create_home_controller(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/controllers/home_controller.sl"),
        app::HOME_CONTROLLER_TEMPLATE,
    )
}

/// Create the application layout
pub fn create_layout(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/views/layouts/application.html.slv"),
        app::LAYOUT_TEMPLATE,
    )
}

/// Create the home index view
pub fn create_index_view(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/views/home/index.html.slv"),
        app::INDEX_VIEW_TEMPLATE,
    )
}

/// Create the CSS file (Tailwind source in app/assets/css/)
pub fn create_css_file(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/assets/css/application.css"),
        app::CSS_TEMPLATE,
    )
}

/// Create the .env file
pub fn create_env_file(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join(".env"), app::ENV_TEMPLATE)
}

/// Create the .gitignore file
pub fn create_gitignore(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join(".gitignore"), app::GITIGNORE_TEMPLATE)
}

/// Create the application helper
pub fn create_application_helper(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/helpers/application_helper.sl"),
        app::APPLICATION_HELPER_TEMPLATE,
    )
}

/// Create sample middleware files
pub fn create_sample_middleware(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/middleware/cors.sl"),
        app::CORS_MIDDLEWARE_TEMPLATE,
    )?;

    write_file(
        &app_path.join("app/middleware/auth.sl"),
        app::AUTH_MIDDLEWARE_TEMPLATE,
    )
}

/// Create Tailwind config file
pub fn create_tailwind_config(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("tailwind.config.js"),
        app::TAILWIND_CONFIG_TEMPLATE,
    )
}

/// Create package.json
pub fn create_package_json(app_path: &Path, name: &str) -> Result<(), String> {
    write_file(&app_path.join("package.json"), &app::package_json(name))
}

/// Create README.md
pub fn create_readme(app_path: &Path, name: &str) -> Result<(), String> {
    write_file(&app_path.join("README.md"), &app::readme(name))
}

/// Install npm dependencies with spinner
pub fn install_npm_dependencies(app_path: &Path) -> bool {
    // Check if npm is available
    if Command::new("npm")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        let spinner = Spinner::start("Installing npm packages...");
        spinner.stop_with_warning("npm not found - run 'npm install' manually");
        return false;
    }

    let spinner = Spinner::start("Installing npm packages...");

    let result = Command::new("npm")
        .args(["install"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => {
            spinner.stop_with_success("Dependencies installed");
            true
        }
        _ => {
            spinner.stop_with_warning("npm install failed - run manually");
            false
        }
    }
}

/// Build Tailwind CSS with spinner
pub fn build_tailwind_css(app_path: &Path) {
    let spinner = Spinner::start("Compiling Tailwind CSS...");

    let result = Command::new("npm")
        .args(["run", "build:css"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => {
            spinner.stop_with_success("Tailwind CSS compiled");
        }
        _ => {
            spinner.stop_with_warning("CSS build failed - run 'npm run build:css' manually");
        }
    }
}

/// Initialize git repository with spinner
pub fn init_git_repo(app_path: &Path) {
    // Check if git is available
    if Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        let spinner = Spinner::start("Initializing git repository...");
        spinner.stop_with_warning("git not found - run 'git init' manually");
        return;
    }

    let spinner = Spinner::start("Initializing git repository...");

    // Run git init
    let init_result = Command::new("git")
        .args(["init"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if init_result.is_err() || !init_result.unwrap().success() {
        spinner.stop_with_warning("git init failed");
        return;
    }

    // Stage all files
    let add_result = Command::new("git")
        .args(["add", "."])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if add_result.is_err() || !add_result.unwrap().success() {
        spinner.stop_with_warning("git init succeeded but staging failed");
        return;
    }

    // Create initial commit
    let commit_result = Command::new("git")
        .args(["commit", "-m", "Initial commit from soli new"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match commit_result {
        Ok(status) if status.success() => {
            spinner.stop_with_success("Git repository initialized");
        }
        _ => {
            spinner.stop_with_warning("git init succeeded but commit failed");
        }
    }
}

/// Replace app_name placeholder in all files within a directory
pub fn replace_placeholders(app_path: &Path, name: &str) -> Result<(), String> {
    use walkdir::WalkDir;

    let walker = WalkDir::new(app_path).follow_links(false);

    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        if path.is_file() {
            // Skip hidden files and git directories
            if let Some(file_name) = path.file_name() {
                if file_name.to_string_lossy().starts_with('.') {
                    continue;
                }
            }

            // Skip binary files (based on extension)
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if ext_str == "png"
                    || ext_str == "jpg"
                    || ext_str == "jpeg"
                    || ext_str == "gif"
                    || ext_str == "ico"
                    || ext_str == "woff"
                    || ext_str == "woff2"
                    || ext_str == "ttf"
                    || ext_str == "eot"
                    || ext_str == "svg"
                {
                    continue;
                }
            }

            // Read, replace, and write back
            let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
            let new_content = content.replace("app_name", name);
            if content != new_content {
                fs::write(path, new_content).map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

/// Create from template archive
pub fn create_from_template(name: &str, app_path: &Path, template_url: &str) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().map_err(|e| e.to_string())?;
    let archive_path = temp_dir.path().join("template.tar.gz");

    println!();
    println!("  Downloading template from {}...", template_url);

    // Download the archive
    let response = ureq::get(template_url)
        .set("Accept", "application/vnd.github.v3.raw")
        .call()
        .map_err(|e| format!("Failed to download template: {}", e))?;

    let mut file = File::create(&archive_path).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader();
    io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;

    // Extract the archive
    let file = File::open(&archive_path).map_err(|e| e.to_string())?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // Create app directory
    fs::create_dir_all(app_path).map_err(|e| e.to_string())?;

    // Extract files
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();

        // Skip root-level files that shouldn't be in the extracted content
        if path.iter().count() == 0 {
            continue;
        }

        let mut out_path = app_path.to_path_buf();
        for component in path.iter() {
            out_path.push(component);
        }

        if entry.header().entry_type() == tar::EntryType::Directory {
            fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
            io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
        }
    }

    // Replace placeholders in all files
    replace_placeholders(app_path, name)?;

    // Initialize git repository
    println!("  Initializing git repository...");
    if Command::new("git")
        .args(["init"])
        .current_dir(app_path)
        .output()
        .is_ok()
        && Command::new("git")
            .args(["add", "."])
            .current_dir(app_path)
            .output()
            .is_ok()
        && Command::new("git")
            .args(["commit", "-m", "Initial commit from Soli template"])
            .current_dir(app_path)
            .output()
            .is_ok()
    {
        println!("  ✓ Git repository initialized");
    }

    println!("  ✓ Template extracted");
    println!();

    print_success_message(name);
    Ok(())
}

/// Print success message after creating an app
pub fn print_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1m✓ Success!\x1b[0m Created \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    println!("  \x1b[2m┌─────────────────────────────────────────┐\x1b[0m");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[1mGet started:\x1b[0m                        \x1b[2m│\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m                                       \x1b[2m│\x1b[0m");
    println!(
        "  \x1b[2m│\x1b[0m    \x1b[36mcd {}\x1b[0m{}  \x1b[2m│\x1b[0m",
        name,
        " ".repeat(30_usize.saturating_sub(name.len()))
    );
    println!(
        "  \x1b[2m│\x1b[0m    \x1b[36msoli serve . --dev\x1b[0m                 \x1b[2m│\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m                                       \x1b[2m│\x1b[0m");
    println!(
        "  \x1b[2m│\x1b[0m  Then open \x1b[4mhttp://localhost:3000\x1b[0m      \x1b[2m│\x1b[0m"
    );
    println!("  \x1b[2m└─────────────────────────────────────────┘\x1b[0m");
    println!();
}

/// Create a new Soli MVC application
pub fn create_app(name: &str, template: Option<&str>) -> Result<(), String> {
    let app_path = Path::new(name);

    if app_path.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    // Display header
    ProgressDisplay::header(name);

    if let Some(template_url) = template {
        // Use custom template from git archive
        return create_from_template(name, app_path, template_url);
    }

    // Use default template generation
    let mut progress = ProgressDisplay::new(7);

    // Step 1: Create directory structure
    progress.step("Creating directory structure...");
    create_directories(app_path)?;
    ProgressDisplay::done();

    // Step 2: Generate configuration files
    progress.step("Generating configuration files...");
    create_routes_file(app_path)?;
    create_env_file(app_path)?;
    create_gitignore(app_path)?;
    create_tailwind_config(app_path)?;
    create_package_json(app_path, name)?;
    ProgressDisplay::done();

    // Step 3: Create MVC components
    progress.step("Creating MVC components...");
    create_home_controller(app_path)?;
    create_layout(app_path)?;
    create_index_view(app_path)?;
    create_application_helper(app_path)?;
    create_sample_middleware(app_path)?;
    ProgressDisplay::done();

    // Step 4: Create assets
    progress.step("Setting up assets...");
    create_css_file(app_path)?;
    create_readme(app_path, name)?;
    ProgressDisplay::done();

    // Step 5: Install dependencies (npm)
    progress.step("Installing dependencies...");
    io::stdout().flush().unwrap();
    println!(); // Move to next line for spinner

    let npm_available = install_npm_dependencies(app_path);

    // Step 6: Build CSS
    if npm_available {
        progress.step("Building Tailwind CSS...");
        io::stdout().flush().unwrap();
        println!(); // Move to next line for spinner
        build_tailwind_css(app_path);
    } else {
        progress.step("Building Tailwind CSS...");
        ProgressDisplay::skip("npm not available");
    }

    // Step 7: Initialize git repository
    progress.step("Initializing git repository...");
    io::stdout().flush().unwrap();
    println!(); // Move to next line for spinner
    init_git_repo(app_path);

    // Print created files summary
    println!();
    println!("  \x1b[2m─────────────────────────────────────────\x1b[0m");
    println!();
    ProgressDisplay::info("\x1b[1mProject structure:\x1b[0m");
    println!("  \x1b[2m│\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[36m{}/\x1b[0m", name);
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m app/");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m controllers/    \x1b[2m# Request handlers\x1b[0m"
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m helpers/        \x1b[2m# View helpers\x1b[0m"
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m middleware/     \x1b[2m# Request filters\x1b[0m"
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m models/         \x1b[2m# Data models\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m  \x1b[2m│   └──\x1b[0m views/          \x1b[2m# Templates\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m config/");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m│   └──\x1b[0m routes.sl     \x1b[2m# URL routing\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m db/migrations/      \x1b[2m# Database migrations\x1b[0m");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m public/             \x1b[2m# Static assets\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m  \x1b[2m└──\x1b[0m tests/              \x1b[2m# Test files\x1b[0m");
    println!("  \x1b[2m│\x1b[0m");

    Ok(())
}
