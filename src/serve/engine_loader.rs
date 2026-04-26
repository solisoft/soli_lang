//! Engine loader for discovering and managing mounted engines.
//!
//! Engines are mini-applications that can be mounted at specific URL paths.
//! Each engine has its own controllers, models, views, routes, and migrations.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::error::RuntimeError;
use crate::interpreter::builtins::model::EngineContextGuard;
use crate::interpreter::builtins::router::register_controller_action;
use crate::interpreter::{Interpreter, Value};
use crate::migration::{DbConfig, Migration};
use crate::serve::app_loader::{
    execute_file as interp_execute_file, sort_controllers_by_dependency,
};
use crate::serve::router::{derive_routes_from_controller, to_pascal_case_controller};
use crate::serve::FileTracker;
use crate::span::Span;

#[derive(Debug, Clone)]
pub struct Engine {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub mounted_at: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub engines: Vec<EngineMount>,
}

#[derive(Debug, Clone)]
pub struct EngineMount {
    pub name: String,
    pub mounted_at: String,
}

lazy_static::lazy_static! {
    static ref MOUNTED_ENGINES: RwLock<HashMap<String, Engine>> = RwLock::new(HashMap::new());
}

pub fn get_mounted_engine(name: &str) -> Option<Engine> {
    MOUNTED_ENGINES.read().unwrap().get(name).cloned()
}

pub fn get_all_mounted_engines() -> Vec<Engine> {
    MOUNTED_ENGINES.read().unwrap().values().cloned().collect()
}

pub fn is_engine_name(name: &str) -> bool {
    MOUNTED_ENGINES.read().unwrap().contains_key(name)
}

pub fn get_engine_for_path(path: &str) -> Option<Engine> {
    let engines = MOUNTED_ENGINES.read().unwrap();
    for engine in engines.values() {
        if path.starts_with(&engine.mounted_at) {
            return Some(engine.clone());
        }
    }
    None
}

pub fn strip_engine_path(path: &str) -> String {
    if let Some(engine) = get_engine_for_path(path) {
        path.strip_prefix(&engine.mounted_at)
            .map(|p| {
                let stripped = p.trim_start_matches('/');
                stripped.to_string()
            })
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    }
}

pub fn load_engines_config(app_path: &Path) -> Result<EngineConfig, String> {
    let config_path = app_path.join("config/engines.sl");

    if !config_path.exists() {
        return Ok(EngineConfig { engines: vec![] });
    }

    let source = fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read engines config: {}", e))?;

    let mut engines = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        // Parse: mount "name", at: "/path"
        if line.starts_with("mount ") {
            let rest = line.strip_prefix("mount ").unwrap().trim();

            // Extract the engine name (between first pair of quotes)
            let name = extract_quoted_string(rest)?;

            if !name.is_empty() {
                // Find the closing quote position (after the name)
                let closing_quote_pos = rest
                    .find(&format!("\"{name}\""))
                    .map(|pos| pos + name.len() + 1) // +1 for opening quote, name.len() for the name
                    .unwrap_or(0);
                let after_name = &rest[closing_quote_pos..];

                // Find "at:" in the remaining string
                let at_str = extract_after_at(after_name);

                if let Some(at) = at_str {
                    engines.push(EngineMount {
                        name: name.to_string(),
                        mounted_at: at.to_string(),
                    });
                }
            }
        }
    }

    Ok(EngineConfig { engines })
}

fn extract_quoted_string(s: &str) -> Result<String, String> {
    let first_quote = s.find('"').ok_or("Missing opening quote")?;
    let after_first = &s[first_quote + 1..];
    let second_quote = after_first.find('"').ok_or("Missing closing quote")?;
    Ok(after_first[..second_quote].to_string())
}

fn extract_after_at(s: &str) -> Option<String> {
    // Look for "at:" or "at: " followed by a quoted string
    for pattern in &["at: \"", "at:\""] {
        if let Some(pos) = s.find(pattern) {
            let after_at = &s[pos + pattern.len()..];
            if let Some(end_quote) = after_at.find('"') {
                return Some(after_at[..end_quote].to_string());
            }
        }
    }
    None
}

pub fn discover_engines(app_path: &Path) -> Result<Vec<Engine>, String> {
    let engines_dir = app_path.join("engines");

    if !engines_dir.exists() {
        return Ok(vec![]);
    }

    let mut discovered = Vec::new();

    for entry in fs::read_dir(&engines_dir)
        .map_err(|e| format!("Failed to read engines directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();

        if path.is_dir() {
            let engine_file = path.join("engine.sl");
            if engine_file.exists() {
                if let Some(engine) = parse_engine_manifest(&path)? {
                    discovered.push(engine);
                }
            }
        }
    }

    Ok(discovered)
}

fn parse_engine_manifest(engine_path: &Path) -> Result<Option<Engine>, String> {
    let manifest_path = engine_path.join("engine.sl");
    let source = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read engine manifest: {}", e))?;

    let name = extract_engine_name(&source)?;
    let version = extract_engine_version(&source)?;
    let dependencies = extract_engine_dependencies(&source)?;

    let engine = Engine {
        name,
        version,
        path: engine_path.to_path_buf(),
        mounted_at: String::new(),
        dependencies,
    };

    Ok(Some(engine))
}

fn extract_engine_name(source: &str) -> Result<String, String> {
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("engine ") {
            if let Some(rest) = line.strip_prefix("engine ") {
                if let Some(name) = rest.split_whitespace().next() {
                    return Ok(name.trim_matches('"').to_string());
                }
            }
        }
    }
    Err("Engine manifest missing 'engine \"name\"' declaration".to_string())
}

fn extract_engine_version(source: &str) -> Result<String, String> {
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("version:") {
            if let Some(rest) = line.strip_prefix("version:") {
                let version = rest.trim().trim_matches(',').trim_matches('"');
                return Ok(version.to_string());
            }
        }
    }
    Ok("1.0.0".to_string())
}

fn extract_engine_dependencies(source: &str) -> Result<Vec<String>, String> {
    let mut deps = Vec::new();

    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("dependencies:") {
            if let Some(rest) = line.strip_prefix("dependencies:") {
                let rest = rest
                    .trim()
                    .trim_matches(|c| c == '[' || c == ']' || c == ' ');
                if !rest.is_empty() && rest != "[]" {
                    for dep in rest.split(',') {
                        let dep = dep.trim().trim_matches('"');
                        if !dep.is_empty() {
                            deps.push(dep.to_string());
                        }
                    }
                }
            }
        }
    }

    Ok(deps)
}

pub fn mount_engines(app_path: &Path, config: &EngineConfig) -> Result<(), String> {
    let discovered = discover_engines(app_path)?;

    let mut mounted = MOUNTED_ENGINES.write().unwrap();

    for mount in &config.engines {
        if let Some(mut engine) = discovered.iter().find(|e| e.name == mount.name).cloned() {
            engine.mounted_at = mount.mounted_at.clone();
            mounted.insert(engine.name.clone(), engine);
            println!("  Mounted engine '{}' at {}", mount.name, mount.mounted_at);
        } else {
            eprintln!(
                "  Warning: Engine '{}' not found in engines/ directory",
                mount.name
            );
        }
    }

    Ok(())
}

pub fn load_engine_controllers(
    interpreter: &mut Interpreter,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    let engines = get_all_mounted_engines();

    for engine in engines {
        load_engine_controller_directory(interpreter, &engine, file_tracker)?;
    }

    Ok(())
}

fn load_engine_controller_directory(
    interpreter: &mut Interpreter,
    engine: &Engine,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    let controllers_dir = engine.path.join("app/controllers");

    if !controllers_dir.exists() {
        return Ok(());
    }

    let mut controllers = Vec::new();

    for entry in std::fs::read_dir(&controllers_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read engine controllers directory: {}", e),
        span: Span::default(),
    })? {
        let entry = entry.map_err(|e| RuntimeError::General {
            message: format!("Failed to read directory entry: {}", e),
            span: Span::default(),
        })?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sl") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("_controller.sl") {
                    controllers.push(path);
                }
            }
        }
    }

    sort_controllers_by_dependency(&mut controllers);

    for controller_path in &controllers {
        file_tracker.track(controller_path);

        let controller_name = controller_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        let source =
            std::fs::read_to_string(controller_path).map_err(|e| RuntimeError::General {
                message: format!("Failed to read controller file: {}", e),
                span: Span::default(),
            })?;

        let routes = derive_routes_from_controller(controller_name, &source).map_err(|e| {
            RuntimeError::General {
                message: format!("Failed to derive routes: {}", e),
                span: Span::default(),
            }
        })?;

        {
            let _guard = EngineContextGuard::enter(&engine.name);
            if let Err(e) = interp_execute_file(interpreter, controller_path) {
                eprintln!("Error loading engine controller {}: {}", controller_name, e);
            }
        }

        let controller_key = controller_name.trim_end_matches("_controller");
        let class_name = to_pascal_case_controller(controller_key);
        let is_oop_controller = interpreter
            .environment
            .borrow()
            .get(&class_name)
            .map(|v| matches!(v, Value::Class(_)))
            .unwrap_or(false);

        for route in routes {
            let full_handler_name = format!("{}#{}", controller_key, route.function_name);

            if !is_oop_controller {
                if let Some(func_value) = interpreter.environment.borrow().get(&route.function_name)
                {
                    register_controller_action(
                        controller_key,
                        &route.function_name,
                        func_value.clone(),
                    );
                }
            }

            let prefixed_path = format!("{}{}", engine.mounted_at, route.path);

            crate::interpreter::builtins::server::register_route_with_handler(
                &route.method,
                &prefixed_path,
                full_handler_name,
            );
        }
    }

    Ok(())
}

pub fn load_engine_models(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
    let engines = get_all_mounted_engines();

    for engine in engines {
        load_engine_model_directory(interpreter, &engine)?;
    }

    Ok(())
}

fn load_engine_model_directory(
    interpreter: &mut Interpreter,
    engine: &Engine,
) -> Result<(), RuntimeError> {
    let models_dir = engine.path.join("app/models");

    if !models_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&models_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read engine models directory: {}", e),
        span: Span::default(),
    })? {
        let entry = entry.map_err(|e| RuntimeError::General {
            message: format!("Failed to read directory entry: {}", e),
            span: Span::default(),
        })?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sl") {
            let _guard = EngineContextGuard::enter(&engine.name);
            if let Err(e) = interp_execute_file(interpreter, &path) {
                eprintln!("Error loading engine model {}: {}", path.display(), e);
            }
        }
    }

    Ok(())
}

const ROUTES_DSL: &str = r#"
    fn get(path: Any, action: Any) { router_match("GET", path, action); }
    fn post(path: Any, action: Any) { router_match("POST", path, action); }
    fn put(path: Any, action: Any) { router_match("PUT", path, action); }
    fn delete(path: Any, action: Any) { router_match("DELETE", path, action); }
    fn patch(path: Any, action: Any) { router_match("PATCH", path, action); }

    fn namespace(name: Any, block: Any) {
        router_namespace_enter(name);
        if (block != null) { block(); }
        router_namespace_exit();
    }
"#;

pub fn load_engine_routes(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
    let engines = get_all_mounted_engines();

    for engine in engines {
        let routes_path = engine.path.join("config/routes.sl");

        if routes_path.exists() {
            let _guard = EngineContextGuard::enter(&engine.name);

            let tokens = crate::lexer::Scanner::new(ROUTES_DSL)
                .scan_tokens()
                .map_err(|e| RuntimeError::General {
                    message: format!("DSL Lexer error: {}", e),
                    span: Span::default(),
                })?;
            let program =
                crate::parser::Parser::new(tokens)
                    .parse()
                    .map_err(|e| RuntimeError::General {
                        message: format!("DSL Parser error: {}", e),
                        span: Span::default(),
                    })?;
            interpreter
                .interpret(&program)
                .map_err(|e| RuntimeError::General {
                    message: format!("Failed to define engine routes DSL: {}", e),
                    span: Span::default(),
                })?;

            if let Err(e) = interp_execute_file(interpreter, &routes_path) {
                eprintln!("Error loading engine routes {}: {}", engine.name, e);
            }
        }
    }

    Ok(())
}

pub fn get_engine_view_path(engine_name: &str, view_name: &str) -> Option<PathBuf> {
    let engines = get_all_mounted_engines();

    for engine in engines {
        if engine.name == engine_name {
            let view_path = engine.path.join("app/views").join(view_name);
            if view_path.exists() {
                return Some(view_path);
            }

            let erb_path = engine
                .path
                .join("app/views")
                .join(format!("{}.erb", view_name));
            if erb_path.exists() {
                return Some(erb_path);
            }
        }
    }

    None
}

/// Ensure engines are loaded and mounted from config. Needed for CLI commands
/// that run outside the server (db:migrate, db:rollback).
fn ensure_engines_mounted(app_path: &Path) -> Result<(), String> {
    if !get_all_mounted_engines().is_empty() {
        return Ok(());
    }

    let config = load_engines_config(app_path)?;
    if !config.engines.is_empty() {
        mount_engines(app_path, &config)?;
    }

    Ok(())
}

pub fn run_engine_migrations(app_path: &Path, engine_name: Option<&str>) -> Result<(), String> {
    use crate::migration::DbConfig;

    ensure_engines_mounted(app_path)?;

    let config = DbConfig::from_env(app_path);
    let engines = get_all_mounted_engines();

    for engine in engines {
        if let Some(name) = engine_name {
            if engine.name != name {
                continue;
            }
        }

        println!("\nRunning migrations for engine: {}", engine.name);

        let runner = EngineMigrationRunner {
            config: config.clone(),
            engine: &engine,
        };

        runner.migrate_up()?;
    }

    Ok(())
}

pub fn run_engine_rollback(app_path: &Path, engine_name: Option<&str>) -> Result<(), String> {
    use crate::migration::DbConfig;

    ensure_engines_mounted(app_path)?;

    let config = DbConfig::from_env(app_path);
    let engines = get_all_mounted_engines();

    let mut found = false;
    for engine in engines {
        if let Some(name) = engine_name {
            if engine.name != name {
                continue;
            }
        }

        found = true;
        println!("\nRolling back last migration for engine: {}", engine.name);

        let runner = EngineMigrationRunner {
            config: config.clone(),
            engine: &engine,
        };

        runner.migrate_down()?;
    }

    if !found {
        if let Some(name) = engine_name {
            return Err(format!("Engine '{}' not found in config/engines.sl", name));
        }
    }

    Ok(())
}

struct EngineMigrationRunner<'a> {
    config: DbConfig,
    engine: &'a Engine,
}

impl<'a> EngineMigrationRunner<'a> {
    fn get_migrations(&self) -> Result<Vec<Migration>, String> {
        let migrations_path = self.engine.path.join("db/migrations");

        if !migrations_path.exists() {
            return Ok(vec![]);
        }

        let mut migrations: Vec<Migration> = fs::read_dir(&migrations_path)
            .map_err(|e| format!("Failed to read migrations directory: {}", e))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .map(|ext| ext == "sl")
                    .unwrap_or(false)
            })
            .filter_map(|entry| Migration::from_path(&entry.path()))
            .collect();

        migrations.sort_by(|a, b| a.version.cmp(&b.version));

        Ok(migrations)
    }

    fn get_applied_migrations(&self) -> Result<Vec<String>, String> {
        use crate::solidb_http::SoliDBClient;

        let engine_name = &self.engine.name;

        let mut client = SoliDBClient::connect(&self.config.host)
            .map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&self.config.database);

        let collection_name = format!("{}_migrations", engine_name);

        let collections = client
            .list_collections()
            .map_err(|e| format!("Failed to list collections: {}", e))?;
        if !collections
            .iter()
            .any(|c| c.get("name").and_then(|n| n.as_str()) == Some(&collection_name))
        {
            client
                .create_collection(&collection_name, None)
                .map_err(|e| format!("Failed to create migrations collection: {}", e))?;
        }

        let query = format!("FOR m IN {} SORT m.version ASC RETURN m", collection_name);
        let results = client.query(&query, None).unwrap_or_else(|_| vec![]);

        let mut versions = Vec::new();
        for item in results {
            if let Some(version) = item.get("version").and_then(|v| v.as_str()) {
                versions.push(version.to_string());
            }
        }

        Ok(versions)
    }

    fn record_migration(&self, migration: &Migration) -> Result<(), String> {
        use crate::solidb_http::SoliDBClient;

        let engine_name = &self.engine.name;
        let collection_name = format!("{}_migrations", engine_name);
        let version = migration.version.clone();
        let name = migration.name.clone();

        let mut client = SoliDBClient::connect(&self.config.host)
            .map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&self.config.database);

        let batch_query = format!(
            "FOR m IN {} COLLECT AGGREGATE max_batch = MAX(m.batch) RETURN {{ max_batch }}",
            collection_name
        );
        let batch_result = client.query(&batch_query, None).unwrap_or_else(|_| vec![]);

        let batch: i64 = batch_result
            .first()
            .and_then(|r| r.get("max_batch"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            + 1;

        let doc = serde_json::json!({
            "version": version,
            "name": name,
            "batch": batch,
            "executed_at": chrono::Utc::now().to_rfc3339()
        });

        client
            .insert(&collection_name, Some(&migration.version), doc)
            .map_err(|e| format!("Failed to record migration: {}", e))?;

        Ok(())
    }

    fn execute_migration(&self, migration: &Migration, direction: &str) -> Result<(), String> {
        let engine_name = &self.engine.name;
        let source = fs::read_to_string(&migration.path)
            .map_err(|e| format!("Failed to read migration file: {}", e))?;

        let host = &self.config.host;
        let database = &self.config.database;

        let soli_code = format!(
            r##"
{source}

let _db_host = "{host}";
let _db_name = "{database}";
let _db = Solidb(_db_host, _db_name);

fn _engine_prefix(name: String) -> String {{
    if name.starts_with("{eng}_") || name.starts_with("_") {{
        return name;
    }}
    return "{eng}_" + name;
}}

class MigrationDb {{
    fn query(sdbql: String) -> Any {{
        return solidb_query(_db_host, _db_name, sdbql);
    }}

    fn create_collection(name: String) -> Any {{
        return solidb_create_collection(_db, _engine_prefix(name));
    }}

    fn drop_collection(name: String) -> Any {{
        return solidb_drop_collection(_db, _engine_prefix(name));
    }}

    fn list_collections() -> Any {{
        return solidb_list_collections(_db);
    }}

    fn collection_stats(name: String) -> Any {{
        return solidb_collection_stats(_db, _engine_prefix(name));
    }}

    fn create_index(collection: String, name: String, fields: Any, options: Any) -> Any {{
        return solidb_create_index(_db, _engine_prefix(collection), name, fields, options);
    }}

    fn drop_index(collection: String, name: String) -> Any {{
        return solidb_drop_index(_db, _engine_prefix(collection), name);
    }}

    fn list_indexes(collection: String) -> Any {{
        return solidb_list_indexes(_db, _engine_prefix(collection));
    }}
}}

let db = MigrationDb();

{direction}(db);
"##,
            source = source,
            host = host,
            database = database,
            eng = engine_name,
            direction = direction
        );

        crate::run_with_options(&soli_code, false)
            .map_err(|e| format!("Migration {} failed: {}", direction, e))?;

        Ok(())
    }

    fn remove_migration_record(&self, migration: &Migration) -> Result<(), String> {
        use crate::solidb_http::SoliDBClient;

        let engine_name = &self.engine.name;
        let collection_name = format!("{}_migrations", engine_name);
        let version = migration.version.clone();

        let mut client = SoliDBClient::connect(&self.config.host)
            .map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&self.config.database);

        client
            .delete(&collection_name, &version)
            .map_err(|e| format!("Failed to remove migration record: {}", e))?;

        Ok(())
    }

    fn migrate_up(&self) -> Result<(), String> {
        let migrations = self.get_migrations()?;
        let applied = self.get_applied_migrations()?;

        let pending: Vec<&Migration> = migrations
            .iter()
            .filter(|m| !applied.contains(&m.version))
            .collect();

        let pending_count = pending.len();
        if pending_count == 0 {
            println!("  No pending migrations for engine '{}'", self.engine.name);
            return Ok(());
        }

        for migration in pending {
            println!("  \x1b[33mMigrating\x1b[0m {}", migration.full_name());
            self.execute_migration(migration, "up")?;
            self.record_migration(migration)?;
            println!("  \x1b[32m   Applied\x1b[0m {}", migration.full_name());
        }

        println!(
            "  Applied {} migration(s) for engine '{}'",
            pending_count, self.engine.name
        );
        Ok(())
    }

    fn migrate_down(&self) -> Result<(), String> {
        let migrations = self.get_migrations()?;
        let applied = self.get_applied_migrations()?;

        if applied.is_empty() {
            println!(
                "  No migrations to rollback for engine '{}'",
                self.engine.name
            );
            return Ok(());
        }

        let last_version = applied.last().unwrap();
        let migration = migrations
            .iter()
            .find(|m| &m.version == last_version)
            .ok_or_else(|| format!("Migration {} not found in files", last_version))?;

        println!("  \x1b[33mRolling back\x1b[0m {}", migration.full_name());

        self.execute_migration(migration, "down")?;
        self.remove_migration_record(migration)?;

        println!("  \x1b[32m   Reverted\x1b[0m {}", migration.full_name());

        Ok(())
    }
}

pub fn reset_engine_context() {
    crate::interpreter::builtins::model::set_model_engine_context(None);
    let mut mounted = MOUNTED_ENGINES.write().unwrap();
    mounted.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_quoted_string() {
        assert_eq!(extract_quoted_string(r#""shop""#).unwrap(), "shop");
        assert_eq!(
            extract_quoted_string(r#""my-engine", at: "/path""#).unwrap(),
            "my-engine"
        );
        assert!(extract_quoted_string("no quotes").is_err());
        assert!(extract_quoted_string(r#""unclosed"#).is_err());
    }

    #[test]
    fn test_extract_after_at() {
        assert_eq!(
            extract_after_at(r#"", at: "/shop""#),
            Some("/shop".to_string())
        );
        assert_eq!(
            extract_after_at(r#"", at:"/admin""#),
            Some("/admin".to_string())
        );
        assert_eq!(extract_after_at(r#"", no_at_here"#), None);
    }

    #[test]
    fn test_load_engines_config_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_engines_config(dir.path()).unwrap();
        assert!(config.engines.is_empty());
    }

    #[test]
    fn test_load_engines_config_parses_mounts() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("engines.sl"),
            r#"
// Engine configuration
mount "shop", at: "/shop"
mount "admin", at: "/admin"
"#,
        )
        .unwrap();

        let config = load_engines_config(dir.path()).unwrap();
        assert_eq!(config.engines.len(), 2);
        assert_eq!(config.engines[0].name, "shop");
        assert_eq!(config.engines[0].mounted_at, "/shop");
        assert_eq!(config.engines[1].name, "admin");
        assert_eq!(config.engines[1].mounted_at, "/admin");
    }

    #[test]
    fn test_load_engines_config_ignores_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(
            config_dir.join("engines.sl"),
            r#"
// This is a comment

mount "blog", at: "/blog"

// Another comment
"#,
        )
        .unwrap();

        let config = load_engines_config(dir.path()).unwrap();
        assert_eq!(config.engines.len(), 1);
        assert_eq!(config.engines[0].name, "blog");
    }

    #[test]
    fn test_discover_engines_no_dir() {
        let dir = tempfile::tempdir().unwrap();
        let engines = discover_engines(dir.path()).unwrap();
        assert!(engines.is_empty());
    }

    #[test]
    fn test_discover_engines_finds_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let engine_dir = dir.path().join("engines/shop");
        fs::create_dir_all(&engine_dir).unwrap();
        fs::write(
            engine_dir.join("engine.sl"),
            r#"engine "shop" {
    version: "2.0.0",
    dependencies: []
}
"#,
        )
        .unwrap();

        let engines = discover_engines(dir.path()).unwrap();
        assert_eq!(engines.len(), 1);
        assert_eq!(engines[0].name, "shop");
        assert_eq!(engines[0].version, "2.0.0");
        assert!(engines[0].dependencies.is_empty());
    }

    #[test]
    fn test_discover_engines_skips_dirs_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let engine_dir = dir.path().join("engines/no_manifest");
        fs::create_dir_all(&engine_dir).unwrap();
        // No engine.sl file

        let engines = discover_engines(dir.path()).unwrap();
        assert!(engines.is_empty());
    }

    #[test]
    fn test_extract_engine_name() {
        let source = r#"engine "shop" {
    version: "1.0.0"
}"#;
        assert_eq!(extract_engine_name(source).unwrap(), "shop");
    }

    #[test]
    fn test_extract_engine_version() {
        let source = r#"engine "shop" {
    version: "3.2.1",
    dependencies: []
}"#;
        assert_eq!(extract_engine_version(source).unwrap(), "3.2.1");
    }

    #[test]
    fn test_extract_engine_version_defaults() {
        let source = r#"engine "shop" {}"#;
        assert_eq!(extract_engine_version(source).unwrap(), "1.0.0");
    }

    #[test]
    fn test_extract_engine_dependencies() {
        let source = r#"engine "shop" {
    version: "1.0.0",
    dependencies: ["auth", "billing"]
}"#;
        let deps = extract_engine_dependencies(source).unwrap();
        assert_eq!(deps, vec!["auth", "billing"]);
    }

    #[test]
    fn test_extract_engine_dependencies_empty() {
        let source = r#"engine "shop" {
    dependencies: []
}"#;
        let deps = extract_engine_dependencies(source).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn test_mount_engines_registers_and_strips_path() {
        // Clean state
        reset_engine_context();

        let dir = tempfile::tempdir().unwrap();
        let engine_dir = dir.path().join("engines/shop");
        fs::create_dir_all(&engine_dir).unwrap();
        fs::write(
            engine_dir.join("engine.sl"),
            r#"engine "shop" {
    version: "1.0.0",
    dependencies: []
}"#,
        )
        .unwrap();

        let config = EngineConfig {
            engines: vec![EngineMount {
                name: "shop".to_string(),
                mounted_at: "/shop".to_string(),
            }],
        };

        mount_engines(dir.path(), &config).unwrap();

        let engine = get_mounted_engine("shop");
        assert!(engine.is_some());
        let engine = engine.unwrap();
        assert_eq!(engine.mounted_at, "/shop");

        // get_engine_for_path
        let found = get_engine_for_path("/shop/products");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "shop");

        assert!(get_engine_for_path("/other").is_none());

        // strip_engine_path
        assert_eq!(strip_engine_path("/shop/products"), "products");
        assert_eq!(strip_engine_path("/other"), "/other");

        // Cleanup
        reset_engine_context();
    }
}
