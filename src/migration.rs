//! Database migration system for Soli MVC applications.
//!
//! Migrations are stored in `db/migrations/` with naming convention:
//! `YYYYMMDDHHMMSS_name.sl`
//!
//! Each migration file should contain `up()` and `down()` functions:
//!
//! ```soli
//! fn up(db: Any) -> Any {
//!     // Create collections
//!     db.create_collection("users");
//!     db.create_collection("posts");
//!
//!     // Create indexes
//!     db.create_index("users", "idx_email", ["email"], { "unique": true });
//!     db.create_index("posts", "idx_author", ["author_id"], {});
//! }
//!
//! fn down(db: Any) -> Any {
//!     db.drop_index("posts", "idx_author");
//!     db.drop_index("users", "idx_email");
//!     db.drop_collection("posts");
//!     db.drop_collection("users");
//! }
//! ```
//!
//! ## Available helpers:
//!
//! ### Collections
//! - `db.create_collection(name)` - Create a new collection
//! - `db.drop_collection(name)` - Drop a collection
//! - `db.list_collections()` - List all collections
//! - `db.collection_stats(name)` - Get collection statistics
//!
//! ### Indexes
//! - `db.create_index(collection, name, fields, options)` - Create an index
//!   - `fields`: Array of field names, e.g., `["email"]` or `["first_name", "last_name"]`
//!   - `options`: Hash with `unique` and/or `sparse` booleans
//! - `db.drop_index(collection, name)` - Drop an index
//! - `db.list_indexes(collection)` - List indexes for a collection
//!
//! ### Raw queries
//! - `db.query(sdbql)` - Execute a raw SDBQL query

use std::fs;
use std::path::{Path, PathBuf};

use crate::solidb_http::SoliDBClient;

/// Load a single .env file, setting variables that aren't already set
fn load_single_env_file(path: &Path) {
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            // Skip comments and empty lines
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                // Remove quotes if present
                let value = value.trim_matches('"').trim_matches('\'');
                // Only set if not already set in environment
                if std::env::var(key).is_err() {
                    std::env::set_var(key, value);
                }
            }
        }
    }
}

/// Load environment variables from .env files
///
/// Loading order:
/// 1. Load base `.env` file first
/// 2. If `APP_ENV` is set, load `.env.{APP_ENV}` to override values
///
/// This matches the convention used by Rails, Node.js, and other frameworks.
fn load_env_file(app_path: &Path) {
    // Load base .env first
    let env_file = app_path.join(".env");
    if env_file.exists() {
        load_single_env_file(&env_file);
    }

    // Then load environment-specific file if APP_ENV is set
    if let Ok(app_env) = std::env::var("APP_ENV") {
        let env_specific = app_path.join(format!(".env.{}", app_env));
        if env_specific.exists() {
            load_single_env_file(&env_specific);
        }
    }
}

/// Configuration for database connection
#[derive(Clone)]
pub struct DbConfig {
    pub host: String,
    pub database: String,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl DbConfig {
    pub fn new(host: &str, database: &str) -> Self {
        Self {
            host: host.to_string(),
            database: database.to_string(),
            username: None,
            password: None,
        }
    }

    pub fn with_auth(mut self, username: &str, password: &str) -> Self {
        self.username = Some(username.to_string());
        self.password = Some(password.to_string());
        self
    }

    /// Load config from .env file and environment variables
    pub fn from_env(app_path: &Path) -> Self {
        // Load .env file first (won't override existing env vars)
        load_env_file(app_path);

        let host =
            std::env::var("SOLIDB_HOST").unwrap_or_else(|_| "http://localhost:6745".to_string());
        // Strip http:// or https:// prefix for TCP connection
        let host_for_tcp = host
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .to_string();
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        let username = std::env::var("SOLIDB_USERNAME").ok();
        let password = std::env::var("SOLIDB_PASSWORD").ok();

        Self {
            host: host_for_tcp,
            database,
            username,
            password,
        }
    }
}

/// Represents a single migration file
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub path: PathBuf,
}

impl Migration {
    /// Parse migration info from filename
    /// Expected format: YYYYMMDDHHMMSS_name.sl
    pub fn from_path(path: &Path) -> Option<Self> {
        let filename = path.file_stem()?.to_str()?;
        let parts: Vec<&str> = filename.splitn(2, '_').collect();

        if parts.len() != 2 {
            return None;
        }

        let version = parts[0].to_string();
        let name = parts[1].to_string();

        // Validate version is numeric
        if !version.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        Some(Self {
            version,
            name,
            path: path.to_path_buf(),
        })
    }

    /// Full migration name for display
    pub fn full_name(&self) -> String {
        format!("{}_{}", self.version, self.name)
    }
}

/// Migration runner that handles up/down/status operations
pub struct MigrationRunner {
    config: DbConfig,
    migrations_path: PathBuf,
}

impl MigrationRunner {
    pub fn new(config: DbConfig, app_path: &Path) -> Self {
        Self {
            config,
            migrations_path: app_path.join("db/migrations"),
        }
    }

    /// Get all migration files sorted by version
    pub fn get_migrations(&self) -> Result<Vec<Migration>, String> {
        if !self.migrations_path.exists() {
            return Ok(vec![]);
        }

        let mut migrations: Vec<Migration> = fs::read_dir(&self.migrations_path)
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

    /// Get list of applied migrations from database
    pub fn get_applied_migrations(&self) -> Result<Vec<String>, String> {
        let config = self.config.clone();

        let mut client =
            SoliDBClient::connect(&config.host).map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&config.database);

        // Create _migrations collection if it doesn't exist
        let collections = client
            .list_collections()
            .map_err(|e| format!("Failed to list collections: {}", e))?;
        if !collections
            .iter()
            .any(|c| c.get("name").and_then(|n| n.as_str()) == Some("_migrations"))
        {
            client
                .create_collection("_migrations")
                .map_err(|e| format!("Failed to create _migrations collection: {}", e))?;
        }

        // Query applied migrations (SDBQL/AQL syntax)
        let query = "FOR m IN _migrations SORT m.version ASC RETURN m";
        let results = client.query(query, None).unwrap_or_else(|_| vec![]);

        let mut versions = Vec::new();
        for item in results {
            if let Some(version) = item.get("version").and_then(|v| v.as_str()) {
                versions.push(version.to_string());
            }
        }

        Ok(versions)
    }

    /// Record a migration as applied
    fn record_migration(&self, migration: &Migration) -> Result<(), String> {
        let config = self.config.clone();
        let version = migration.version.clone();
        let name = migration.name.clone();

        let mut client =
            SoliDBClient::connect(&config.host).map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&config.database);

        // Create _migrations collection if it doesn't exist
        let collections = client
            .list_collections()
            .map_err(|e| format!("Failed to list collections: {}", e))?;
        if !collections
            .iter()
            .any(|c| c.get("name").and_then(|n| n.as_str()) == Some("_migrations"))
        {
            client
                .create_collection("_migrations")
                .map_err(|e| format!("Failed to create _migrations collection: {}", e))?;
        }

        // Get the next batch number (SDBQL/AQL syntax)
        let batch_query =
            "FOR m IN _migrations COLLECT AGGREGATE max_batch = MAX(m.batch) RETURN { max_batch }";
        let batch_result = client.query(batch_query, None).unwrap_or_else(|_| vec![]);

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
            .insert("_migrations", Some(&version), doc)
            .map_err(|e| format!("Failed to record migration: {}", e))?;

        Ok(())
    }

    /// Remove a migration record
    fn remove_migration_record(&self, migration: &Migration) -> Result<(), String> {
        let config = self.config.clone();
        let version = migration.version.clone();

        let mut client =
            SoliDBClient::connect(&config.host).map_err(|e| format!("Failed to connect: {}", e))?;

        if let (Some(username), Some(password)) = (&config.username, &config.password) {
            client = client.with_basic_auth(username, password);
        }
        client.set_database(&config.database);

        client
            .delete("_migrations", &version)
            .map_err(|e| format!("Failed to remove migration record: {}", e))?;

        Ok(())
    }

    /// Execute a migration's up() or down() function
    fn execute_migration(&self, migration: &Migration, direction: &str) -> Result<(), String> {
        // Read migration file
        let source = fs::read_to_string(&migration.path)
            .map_err(|e| format!("Failed to read migration file: {}", e))?;

        // Create interpreter with db connection
        let config = self.config.clone();

        // Execute the migration using the interpreter
        let full_source = format!(
            r#"
{}

// Create db connection helper with collection and index management
let _db_host = "{}";
let _db_name = "{}";
let _db = Solidb(_db_host, _db_name);

class MigrationDb {{
    // Run a raw SDBQL query
    fn query(sdbql: String) -> Any {{
        return solidb_query(_db_host, _db_name, sdbql);
    }}

    // Collection management
    fn create_collection(name: String) -> Any {{
        return solidb_create_collection(_db, name);
    }}

    fn drop_collection(name: String) -> Any {{
        return solidb_drop_collection(_db, name);
    }}

    fn list_collections() -> Any {{
        return solidb_list_collections(_db);
    }}

    fn collection_stats(name: String) -> Any {{
        return solidb_collection_stats(_db, name);
    }}

    // Index management
    fn create_index(collection: String, name: String, fields: Any, options: Any) -> Any {{
        return solidb_create_index(_db, collection, name, fields, options);
    }}

    fn drop_index(collection: String, name: String) -> Any {{
        return solidb_drop_index(_db, collection, name);
    }}

    fn list_indexes(collection: String) -> Any {{
        return solidb_list_indexes(_db, collection);
    }}
}}

let db = MigrationDb();

// Run the migration
{}(db);
"#,
            source, config.host, config.database, direction
        );

        // Run using tree-walk interpreter
        crate::run_with_options(&full_source, false)
            .map_err(|e| format!("Migration {} failed: {}", direction, e))?;

        Ok(())
    }

    /// Run all pending migrations
    pub fn migrate_up(&self) -> Result<MigrationResult, String> {
        let migrations = self.get_migrations()?;
        let applied = self.get_applied_migrations()?;

        let pending: Vec<&Migration> = migrations
            .iter()
            .filter(|m| !applied.contains(&m.version))
            .collect();

        if pending.is_empty() {
            return Ok(MigrationResult {
                applied: vec![],
                message: "No pending migrations".to_string(),
            });
        }

        let mut applied_migrations = Vec::new();

        for migration in pending {
            println!("  \x1b[33mMigrating\x1b[0m {}", migration.full_name());

            self.execute_migration(migration, "up")?;
            self.record_migration(migration)?;

            println!("  \x1b[32m   Applied\x1b[0m {}", migration.full_name());

            applied_migrations.push(migration.full_name());
        }

        Ok(MigrationResult {
            message: format!("Applied {} migration(s)", applied_migrations.len()),
            applied: applied_migrations,
        })
    }

    /// Rollback the last migration
    pub fn migrate_down(&self) -> Result<MigrationResult, String> {
        let migrations = self.get_migrations()?;
        let applied = self.get_applied_migrations()?;

        if applied.is_empty() {
            return Ok(MigrationResult {
                applied: vec![],
                message: "No migrations to rollback".to_string(),
            });
        }

        // Get the last applied migration
        let last_version = applied.last().unwrap();
        let migration = migrations
            .iter()
            .find(|m| &m.version == last_version)
            .ok_or_else(|| format!("Migration {} not found in files", last_version))?;

        println!("  \x1b[33mRolling back\x1b[0m {}", migration.full_name());

        self.execute_migration(migration, "down")?;
        self.remove_migration_record(migration)?;

        println!("  \x1b[32m   Reverted\x1b[0m {}", migration.full_name());

        Ok(MigrationResult {
            message: format!("Rolled back {}", migration.full_name()),
            applied: vec![migration.full_name()],
        })
    }

    /// Show migration status
    pub fn status(&self) -> Result<MigrationStatus, String> {
        let migrations = self.get_migrations()?;
        let applied = self.get_applied_migrations()?;

        let statuses: Vec<MigrationStatusEntry> = migrations
            .iter()
            .map(|m| MigrationStatusEntry {
                version: m.version.clone(),
                name: m.name.clone(),
                applied: applied.contains(&m.version),
            })
            .collect();

        let pending_count = statuses.iter().filter(|s| !s.applied).count();
        let applied_count = statuses.iter().filter(|s| s.applied).count();

        Ok(MigrationStatus {
            entries: statuses,
            pending_count,
            applied_count,
        })
    }
}

/// Result of a migration operation
pub struct MigrationResult {
    pub message: String,
    pub applied: Vec<String>,
}

/// Status of all migrations
pub struct MigrationStatus {
    pub entries: Vec<MigrationStatusEntry>,
    pub pending_count: usize,
    pub applied_count: usize,
}

/// Status of a single migration
pub struct MigrationStatusEntry {
    pub version: String,
    pub name: String,
    pub applied: bool,
}

/// Generate a new migration file
pub fn generate_migration(app_path: &Path, name: &str) -> Result<PathBuf, String> {
    let migrations_path = app_path.join("db/migrations");

    // Create migrations directory if it doesn't exist
    fs::create_dir_all(&migrations_path)
        .map_err(|e| format!("Failed to create migrations directory: {}", e))?;

    // Generate timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");

    // Sanitize name
    let safe_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    let filename = format!("{}_{}.sl", timestamp, safe_name);
    let filepath = migrations_path.join(&filename);

    // Generate migration template
    let template = format!(
        r#"// Migration: {}
// Created: {}

fn up(db: Any) -> Any {{
    // Collection helpers:
    //   db.create_collection("users")
    //   db.drop_collection("users")
    //   db.list_collections()
    //   db.collection_stats("users")
    //
    // Index helpers:
    //   db.create_index("users", "idx_email", ["email"], {{ "unique": true }})
    //   db.create_index("users", "idx_name", ["first_name", "last_name"], {{ "sparse": true }})
    //   db.drop_index("users", "idx_email")
    //   db.list_indexes("users")
    //
    // Raw SDBQL queries:
    //   db.query("FOR doc IN users RETURN doc")
    //   db.query("INSERT {{ name: 'value' }} INTO users")
}}

fn down(db: Any) -> Any {{
    // Rollback the changes made in up()
}}
"#,
        name,
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    fs::write(&filepath, template).map_err(|e| format!("Failed to write migration file: {}", e))?;

    Ok(filepath)
}

/// Print migration status in a nice format
pub fn print_status(status: &MigrationStatus) {
    println!();
    println!("  \x1b[1mDatabase Migrations\x1b[0m");
    println!();

    if status.entries.is_empty() {
        println!("  No migrations found.");
        println!();
        println!("  Create one with: \x1b[36msoli db:migrate generate <name>\x1b[0m");
        println!();
        return;
    }

    println!("  {:14}  {:30}  {:10}", "Version", "Name", "Status");
    println!("  {:-<14}  {:-<30}  {:-<10}", "", "", "");

    for entry in &status.entries {
        let status_str = if entry.applied {
            "\x1b[32m   up   \x1b[0m"
        } else {
            "\x1b[33m  down  \x1b[0m"
        };

        println!("  {:14}  {:30}  {}", entry.version, entry.name, status_str);
    }

    println!();
    println!(
        "  \x1b[32m{}\x1b[0m applied, \x1b[33m{}\x1b[0m pending",
        status.applied_count, status.pending_count
    );
    println!();
}
