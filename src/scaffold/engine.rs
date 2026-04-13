//! Engine scaffolding generator
//!
//! Creates the directory structure and files for a new engine:
//! engines/<name>/
//!   engine.sl              # Manifest
//!   app/
//!     controllers/
//!     models/
//!     views/
//!   config/
//!     routes.sl
//!   db/
//!     migrations/

use std::fs;
use std::path::Path;

use crate::scaffold::ui::Spinner;

/// Create a new engine with the given name
pub fn create_engine(name: &str) -> Result<(), String> {
    let engine_path = Path::new("engines").join(name);

    if engine_path.exists() {
        return Err(format!(
            "Engine '{}' already exists at {}",
            name,
            engine_path.display()
        ));
    }

    let spinner = Spinner::start("Creating engine structure...");

    // Create directory structure
    let dirs = [
        format!("engines/{}/app/controllers", name),
        format!("engines/{}/app/models", name),
        format!("engines/{}/app/views", name),
        format!("engines/{}/app/helpers", name),
        format!("engines/{}/config", name),
        format!("engines/{}/db/migrations", name),
    ];

    for dir in &dirs {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create directory '{}': {}", dir, e))?;
    }

    // Create engine manifest
    create_engine_manifest(&engine_path, name)?;

    // Create engine routes
    create_engine_routes(&engine_path, name)?;

    // Create engine home controller
    create_engine_home_controller(&engine_path, name)?;

    // Create engine layout
    create_engine_layout(&engine_path)?;

    // Create engine index view
    create_engine_index_view(&engine_path, name)?;

    // Create placeholder migration
    create_placeholder_migration(&engine_path, name)?;

    spinner.stop_with_success(&format!("Engine '{}' created successfully", name));

    print_engine_success_message(name);

    Ok(())
}

fn create_engine_manifest(engine_path: &Path, name: &str) -> Result<(), String> {
    let manifest_content = format!(
        r#"//! Engine manifest for {name}

engine "{name}" {{
    version: "1.0.0",
    dependencies: []
}}
"#,
        name = name
    );

    let manifest_path = engine_path.join("engine.sl");
    fs::write(&manifest_path, manifest_content)
        .map_err(|e| format!("Failed to create engine manifest: {}", e))?;

    Ok(())
}

fn create_engine_routes(engine_path: &Path, name: &str) -> Result<(), String> {
    let routes_content = format!(
        r#"//! Engine routes for {name}
//! Routes are mounted at the engine's mount point

get("/", "{name}#index")
"#,
        name = name
    );

    let routes_path = engine_path.join("config/routes.sl");
    fs::write(&routes_path, routes_content)
        .map_err(|e| format!("Failed to create engine routes: {}", e))?;

    Ok(())
}

fn create_engine_home_controller(engine_path: &Path, name: &str) -> Result<(), String> {
    let controller_content = format!(
        r#"//! {name} engine home controller

class {PascalName}Controller extends Controller {{
    // GET /
    fn index(req) {{
        return render("{name}/index", {{ "engine": "{name}" }});
    }}
}}
"#,
        name = name,
        PascalName = to_pascal_case(name)
    );

    let controller_path = engine_path.join(format!("app/controllers/{}_controller.sl", name));
    fs::write(&controller_path, controller_content)
        .map_err(|e| format!("Failed to create engine controller: {}", e))?;

    Ok(())
}

fn create_engine_layout(engine_path: &Path) -> Result<(), String> {
    let layout_content = r#"<!DOCTYPE html>
<html>
<head>
    <title><%= yield("title", "Engine") %></title>
</head>
<body>
    <%= yield() %>
</body>
</html>
"#;

    let layout_path = engine_path.join("app/views/layouts/engine.html.slv");
    fs::create_dir_all(layout_path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&layout_path, layout_content)
        .map_err(|e| format!("Failed to create engine layout: {}", e))?;

    Ok(())
}

fn create_engine_index_view(engine_path: &Path, name: &str) -> Result<(), String> {
    let view_content = format!(
        r#"<%% use layout("layouts/engine.html.slv"); %%>
<div class="engine-content">
    <h1>{name} Engine</h1>
    <p>Welcome to the {name} engine!</p>
</div>
"#,
        name = name
    );

    let view_path = engine_path.join(format!("app/views/{}/index.html.slv", name));
    fs::create_dir_all(view_path.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(&view_path, view_content)
        .map_err(|e| format!("Failed to create engine index view: {}", e))?;

    Ok(())
}

fn create_placeholder_migration(engine_path: &Path, name: &str) -> Result<(), String> {
    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");
    let migration_content = format!(
        r#"//! Migration: {timestamp}_engine_setup
//! Engine: {name}

fn up(db: Any) -> Any {{
    // Create your collections here
    // Example:
    // db.create_collection("{name}_items");
}}
"#,
        timestamp = timestamp,
        name = name
    );

    let migration_path = engine_path.join(format!("db/migrations/{}_engine_setup.sl", timestamp));
    fs::write(&migration_path, migration_content)
        .map_err(|e| format!("Failed to create placeholder migration: {}", e))?;

    Ok(())
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("shop"), "Shop");
        assert_eq!(to_pascal_case("user_auth"), "UserAuth");
        assert_eq!(to_pascal_case("my-engine"), "MyEngine");
        assert_eq!(to_pascal_case("a_b_c"), "ABC");
    }

    // NOTE: create_engine uses relative paths (cwd-dependent) and set_current_dir
    // is process-global, so these tests must be serialized. We use a mutex to avoid
    // races when tests run in parallel.
    use std::sync::Mutex;
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_create_engine_generates_structure() {
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        create_engine("shop").unwrap();

        let engine_path = dir.path().join("engines/shop");
        assert!(engine_path.join("engine.sl").exists());
        assert!(engine_path.join("config/routes.sl").exists());
        assert!(engine_path
            .join("app/controllers/shop_controller.sl")
            .exists());
        assert!(engine_path.join("app/views/shop/index.html.slv").exists());
        assert!(engine_path
            .join("app/views/layouts/engine.html.slv")
            .exists());
        assert!(engine_path.join("app/models").is_dir());
        assert!(engine_path.join("app/helpers").is_dir());
        assert!(engine_path.join("db/migrations").is_dir());

        // Verify manifest content
        let manifest = std::fs::read_to_string(engine_path.join("engine.sl")).unwrap();
        assert!(manifest.contains(r#"engine "shop""#));

        // Verify controller uses PascalCase
        let controller =
            std::fs::read_to_string(engine_path.join("app/controllers/shop_controller.sl"))
                .unwrap();
        assert!(controller.contains("ShopController"));

        // At least one migration file was created
        let migrations: Vec<_> = std::fs::read_dir(engine_path.join("db/migrations"))
            .unwrap()
            .collect();
        assert_eq!(migrations.len(), 1);

        std::env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn test_create_engine_fails_if_exists() {
        let _lock = CWD_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        create_engine("shop").unwrap();
        let result = create_engine("shop");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));

        std::env::set_current_dir(original_dir).unwrap();
    }
}

fn print_engine_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1mSuccess!\x1b[0m Created engine \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    println!("  \x1b[2mGenerated files:\x1b[0m");
    println!();
    println!("    \x1b[36mengines/{}/engine.sl\x1b[0m", name);
    println!("    \x1b[36mengines/{}/config/routes.sl\x1b[0m", name);
    println!(
        "    \x1b[36mengines/{}/app/controllers/{}_controller.sl\x1b[0m",
        name, name
    );
    println!(
        "    \x1b[36mengines/{}/app/views/{}/index.html.slv\x1b[0m",
        name, name
    );
    println!("    \x1b[36mengines/{}/db/migrations/...\x1b[0m", name);
    println!();
    println!("  \x1b[2mTo mount the engine, add to config/engines.sl:\x1b[0m");
    println!();
    println!("    \x1b[33mmount \"{}\", at: \"/{}\"\x1b[0m", name, name);
    println!();
}
