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
//! - `db.create_collection(name, type?)` - Create a collection. `type` is optional;
//!   pass `"blob"`, `"columnar"`, `"timeseries"`, etc. The string is forwarded
//!   verbatim to SolidB. Default is a regular document collection.
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

use std::collections::HashMap;
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
                    // TODO: Audit that the environment access only happens in single-threaded code.
                    unsafe { std::env::set_var(key, value) };
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
        // SEC-027: pass the URL through to `SoliDBClient::connect` with
        // its scheme intact. The previous strip + reconnect made TLS
        // impossible — `connect` would re-add `http://` regardless of
        // the operator's https:// configuration.
        let database = std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "default".to_string());
        let username = std::env::var("SOLIDB_USERNAME").ok();
        let password = std::env::var("SOLIDB_PASSWORD").ok();

        Self {
            host,
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
    /// Connection declared inside the file with `connection "name"`. A
    /// migration knows which database it belongs to, so `soli db:migrate up`
    /// needs no `--connection` flag to place it correctly.
    pub connection: Option<String>,
    /// Set when the file's `connection` declaration is malformed (a duplicate,
    /// or one that is not the first non-comment statement). Discovery fails
    /// the run rather than guessing a target.
    pub connection_error: Option<String>,
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

        // Read the declaration now: the runner must know the target before it
        // can look up which migrations that database has already applied.
        let (connection, connection_error) = match fs::read_to_string(path) {
            Ok(src) => match scan_connection(&src) {
                Ok(name) => (name, None),
                Err(e) => (None, Some(e)),
            },
            Err(_) => (None, None),
        };

        Some(Self {
            version,
            name,
            path: path.to_path_buf(),
            connection,
            connection_error,
        })
    }

    /// Full migration name for display
    pub fn full_name(&self) -> String {
        format!("{}_{}", self.version, self.name)
    }
}

fn is_comment_line(line: &str) -> bool {
    line.starts_with('#') || line.starts_with("//")
}

/// Parse `connection "name"` / `connection("name")` on a single trimmed line.
fn parse_connection_line(line: &str) -> Option<String> {
    let rest = line.strip_prefix("connection")?;
    // `connection "x"` and `connection("x")` both read naturally.
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('(').unwrap_or(rest).trim_start();
    let rest = rest.strip_prefix('"')?;
    let (name, _) = rest.split_once('"')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Find a top-level `connection "name"` declaration in a migration's source.
///
/// Only the first non-comment statement is considered. A line inside a
/// `"""…"""` / `[[…]]` string, or after `def up`, therefore cannot pick the
/// target. A second declaration is an error, not a silent first-wins.
pub fn scan_connection(source: &str) -> Result<Option<String>, String> {
    let mut found: Option<String> = None;
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || is_comment_line(line) {
            continue;
        }
        if let Some(name) = parse_connection_line(line) {
            if found.is_some() {
                return Err(
                    "a migration may declare connection only once, as its first \
                     non-comment statement"
                        .into(),
                );
            }
            found = Some(name);
            continue;
        }
        // First real statement that is not `connection`: stop. A later
        // `connection "warehouse"` inside a string or after `def` is ignored.
        break;
    }
    Ok(found)
}

/// Comment out the `connection "name"` line before the file is executed.
///
/// The declaration is metadata for the runner, not a statement: at top level it
/// would call the model DSL's `connection` builtin with the wrong arity. The
/// line is replaced rather than removed so error messages keep their line
/// numbers. Only the first-statement declaration is stripped — the same one
/// [`scan_connection`] accepted.
pub fn strip_connection_declaration(source: &str) -> String {
    let mut out = Vec::with_capacity(source.lines().count());
    let mut seen_statement = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if !seen_statement && (trimmed.is_empty() || is_comment_line(trimmed)) {
            out.push(line.to_string());
            continue;
        }
        if !seen_statement && parse_connection_line(trimmed).is_some() {
            seen_statement = true;
            out.push(format!("# {line}   (target read by the migration runner)"));
            continue;
        }
        seen_statement = true;
        out.push(line.to_string());
    }
    out.join("\n")
}

/// Migration runner that handles up/down/status operations
pub struct MigrationRunner {
    config: DbConfig,
    migrations_path: PathBuf,
    /// `--connection NAME` from the CLI: restricts the run to that database.
    connection_filter: Option<String>,
}

impl MigrationRunner {
    pub fn new(config: DbConfig, app_path: &Path) -> Self {
        Self {
            config,
            migrations_path: app_path.join("db/migrations"),
            connection_filter: None,
        }
    }

    /// Restrict the run to one named connection (`db:migrate up -c legacy`).
    pub fn with_connection_filter(mut self, name: Option<&str>) -> Self {
        self.connection_filter = name.map(str::to_string);
        self
    }

    /// Which connection a migration runs on: its own declaration first, then
    /// the CLI flag, then the default connection.
    fn target_of(&self, migration: &Migration) -> Option<String> {
        migration
            .connection
            .clone()
            .or_else(|| self.connection_filter.clone())
    }

    /// True when `--connection` was given and this migration belongs elsewhere.
    fn filtered_out(&self, migration: &Migration) -> bool {
        match (&self.connection_filter, &migration.connection) {
            (Some(filter), Some(declared)) => filter != declared,
            _ => false,
        }
    }

    /// Run `f` against `target`, or the ambient connection when there is none.
    fn run_on<T>(
        &self,
        target: Option<&str>,
        f: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        match target {
            Some(name) => crate::db::with_connection(name, f),
            None => f(),
        }
    }

    /// Applied versions for `target`, bootstrapping its `_migrations` table.
    /// Cached because several migrations usually share one connection.
    fn applied_for<'a>(
        &self,
        cache: &'a mut HashMap<Option<String>, Vec<String>>,
        target: Option<String>,
    ) -> Result<&'a Vec<String>, String> {
        if !cache.contains_key(&target) {
            let applied = self.run_on(target.as_deref(), || {
                self.ensure_database()?;
                self.get_applied_migrations()
            })?;
            cache.insert(target.clone(), applied);
        }
        Ok(cache.get(&target).expect("just inserted"))
    }

    /// Ensure the selected adapter is ready (SoliDB default, or SQL pool).
    fn ensure_adapter(&self) -> Result<(), String> {
        crate::db::ensure_runtime_ready().map_err(|e| e.message())
    }

    /// Ensure the configured database exists, creating it if absent. Lets
    /// `db:migrate up` (and `down`/`status`) bootstrap a brand-new database
    /// instead of failing with a 404 when no one has created it yet — the
    /// database-level analogue of the `_migrations` collection bootstrap.
    fn ensure_database(&self) -> Result<(), String> {
        self.ensure_adapter()?;
        if crate::db::is_sql() {
            // DATABASE_URL already names the target database; ensure version table.
            return crate::db::sql::ensure_migrations_table();
        }
        let mut client = SoliDBClient::connect(&self.config.host)
            .map_err(|e| format!("Failed to connect: {}", e))?;
        if let (Some(username), Some(password)) = (&self.config.username, &self.config.password) {
            client = client.with_basic_auth(username, password);
        }
        let databases = client
            .list_databases()
            .map_err(|e| format!("Failed to list databases: {}", e))?;
        if !databases.iter().any(|d| d == &self.config.database) {
            client.create_database(&self.config.database).map_err(|e| {
                format!(
                    "Failed to create database '{}': {}",
                    self.config.database, e
                )
            })?;
            println!("  \x1b[32mCreated database\x1b[0m {}", self.config.database);
        }
        Ok(())
    }

    /// Get all migration files sorted by version
    pub fn get_migrations(&self) -> Result<Vec<Migration>, String> {
        if !self.migrations_path.exists() {
            return Ok(vec![]);
        }

        let mut migrations: Vec<Migration> = Vec::new();
        for entry in fs::read_dir(&self.migrations_path)
            .map_err(|e| format!("Failed to read migrations directory: {}", e))?
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if entry.path().extension().and_then(|e| e.to_str()) != Some("sl") {
                continue;
            }
            let Some(migration) = Migration::from_path(&entry.path()) else {
                continue;
            };
            if let Some(err) = &migration.connection_error {
                return Err(format!("{}: {err}", migration.path.display()));
            }
            migrations.push(migration);
        }

        migrations.sort_by(|a, b| a.version.cmp(&b.version));

        Ok(migrations)
    }

    /// Get list of applied migrations from database
    pub fn get_applied_migrations(&self) -> Result<Vec<String>, String> {
        if crate::db::is_sql() {
            let applied = crate::db::sql::list_applied_migrations()?;
            return Ok(applied.into_iter().map(|(v, _)| v).collect());
        }
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
                .create_collection("_migrations", None)
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
        if crate::db::is_sql() {
            return crate::db::sql::record_migration(&migration.version, &migration.name);
        }
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
                .create_collection("_migrations", None)
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
        if crate::db::is_sql() {
            return crate::db::sql::remove_migration(&migration.version);
        }
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
        // Read migration file. The connection declaration was already read at
        // discovery time; neutralize it so it is not executed as a statement.
        let source = fs::read_to_string(&migration.path)
            .map_err(|e| format!("Failed to read migration file: {}", e))?;
        let source = strip_connection_declaration(&source);

        if crate::db::is_sql() {
            return self.execute_migration_sql(&source, direction);
        }

        // Create interpreter with db connection
        let config = self.config.clone();

        // Surface the configured credentials to the migration script.
        // Without this the inline Solidb instance below has no auth, so
        // every operation that touches an authenticated endpoint (i.e.
        // every real DB call) gets a 401 from SoliDB. Prior to the
        // exec_db_sync error-propagation fix these 401s were silently
        // converted into Value::String("Error: HTTP 401 ..."), letting
        // the migration runner stamp "Applied" while the DB had no
        // actual change applied.
        let auth_username = config.username.as_deref().unwrap_or("");
        let auth_password = config.password.as_deref().unwrap_or("");
        let auth_snippet = if !auth_username.is_empty() {
            format!("_db.auth({:?}, {:?});\n", auth_username, auth_password)
        } else {
            String::new()
        };

        // Execute the migration using the interpreter
        let full_source = format!(
            r#"
{}

// Create db connection helper with collection and index management
let _db_host = "{}";
let _db_name = "{}";
let _db = Solidb(_db_host, _db_name);
{}
class MigrationDb {{
    // Run a raw SDBQL query, optionally with bind variables
    fn query(sdbql: String, bind_vars: Any = null) -> Any {{
        if bind_vars == null {{
            return _db.query(sdbql);
        }}
        return _db.query(sdbql, bind_vars);
    }}

    // Collection management. `collection_type` is optional — pass a
    // SolidB-recognized type ("edge", "timeseries", "blob") to create a
    // typed collection. NOTE: "columnar" is NOT a document-collection type —
    // passing it here used to silently create a mislabeled document
    // collection; use create_columnar(name, columns) instead.
    fn create_collection(name: String, collection_type: String = "") -> Any {{
        if (collection_type == "columnar") {{
            throw "columnar stores are not document collections - use db.create_columnar(name, columns)";
        }}
        if (collection_type == "") {{
            return solidb_create_collection(_db, name);
        }}
        return solidb_create_collection(_db, name, collection_type);
    }}

    // Columnar store management. `columns` is an array of
    // {{ "name": ..., "type": ..., "nullable": bool?, "indexed": bool? }}
    // hashes; options accepts {{ "compression": "lz4"|"none" }}.
    fn create_columnar(name: String, columns: Any, options: Any = null) -> Any {{
        if (options == null) {{
            return solidb_create_columnar(_db, name, columns);
        }}
        return solidb_create_columnar(_db, name, columns, options);
    }}

    fn drop_columnar(name: String) -> Any {{
        return solidb_drop_columnar(_db, name);
    }}

    fn drop_collection(name: String) -> Any {{
        return solidb_drop_collection(_db, name);
    }}

    // Timeseries retention: delete documents older than the RFC3339 cutoff.
    // Returns the number of deleted documents.
    fn prune_collection(name: String, older_than: String) -> Any {{
        return solidb_prune_collection(_db, name, older_than);
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

    // Vector index management
    fn create_vector_index(collection: String, name: String, field: String, dimension: Int, options: Any = "cosine") -> Any {{
        return solidb_create_vector_index(_db, collection, name, field, dimension, options);
    }}

    fn drop_vector_index(collection: String, name: String) -> Any {{
        return solidb_drop_vector_index(_db, collection, name);
    }}
}}

let db = MigrationDb();

// Run the migration
{}(db);
"#,
            source, config.host, config.database, auth_snippet, direction
        );

        // Run using tree-walk interpreter
        crate::run_with_options(&full_source, false)
            .map_err(|e| format!("Migration {} failed: {}", direction, e))?;

        Ok(())
    }

    /// SQL migration runner (Postgres / MySQL / SQLite): create_table / drop_table.
    fn execute_migration_sql(&self, source: &str, direction: &str) -> Result<(), String> {
        let full_source = format!(
            r#"
{source}

class MigrationDb {{
    # No column hash -> a document table (_key + doc). With one -> real columns.
    fn create_table(name: String, columns: Any = null) -> Any {{
        if (columns == null) {{
            return __soli_sql_create_table(name);
        }}
        return __soli_sql_create_columns(name, columns);
    }}

    fn drop_table(name: String) -> Any {{
        return __soli_sql_drop_table(name);
    }}

    fn add_column(table: String, name: String, column_type: Any) -> Any {{
        return __soli_sql_add_column(table, name, column_type);
    }}

    fn drop_column(table: String, name: String) -> Any {{
        return __soli_sql_drop_column(table, name);
    }}

    # `from` is a reserved word in Soli, hence the explicit parameter names.
    fn rename_column(table: String, old_name: String, new_name: String) -> Any {{
        return __soli_sql_rename_column(table, old_name, new_name);
    }}

    fn rename_table(old_name: String, new_name: String) -> Any {{
        return __soli_sql_rename_table(old_name, new_name);
    }}

    fn add_index(table: String, columns: Any, options: Any = null) -> Any {{
        return __soli_sql_add_index(table, columns, options);
    }}

    fn drop_index(table: String, name: String) -> Any {{
        return __soli_sql_drop_index(table, name);
    }}

    # SoliDB-shaped index call, so a shared migration keeps working on SQL.
    fn create_index(table: String, name: String, fields: Any, options: Any = null) -> Any {{
        let opts = {{ "name": name }};
        if (options != null && options["unique"] == true) {{
            opts["unique"] = true;
        }}
        return __soli_sql_add_index(table, fields, opts);
    }}

    # Anything the portable DSL cannot express. Engine-specific by definition.
    fn execute(sql: String) -> Any {{
        return __soli_sql_execute(sql);
    }}

    # Document-collection alias — same as create_table on SQL adapters.
    fn create_collection(name: String, collection_type: String = "") -> Any {{
        if (collection_type != "" && collection_type != "document") {{
            throw "SQL migrations only support document tables (create_table / create_collection without type). Typed collections (edge/timeseries/blob) are SoliDB-only.";
        }}
        return __soli_sql_create_table(name);
    }}

    fn drop_collection(name: String) -> Any {{
        return __soli_sql_drop_table(name);
    }}

    fn query(sdbql: String, bind_vars: Any = null) -> Any {{
        throw "Raw SDBQL db.query is SoliDB-only. On SQL adapters use create_table / drop_table.";
    }}

    fn create_columnar(name: String, columns: Any, options: Any = null) -> Any {{
        throw "create_columnar is SoliDB-only.";
    }}

    fn create_vector_index(collection: String, name: String, field: String, dimension: Int, options: Any = "cosine") -> Any {{
        throw "create_vector_index is SoliDB-only.";
    }}
}}

let db = MigrationDb();
{direction}(db);
"#
        );

        crate::run_migration_source(&full_source)
            .map_err(|e| format!("Migration {direction} failed: {e}"))?;
        Ok(())
    }

    /// Run all pending migrations
    pub fn migrate_up(&self) -> Result<MigrationResult, String> {
        let migrations = self.get_migrations()?;
        let mut cache: HashMap<Option<String>, Vec<String>> = HashMap::new();
        let mut applied_migrations = Vec::new();
        let mut skipped = 0usize;

        for migration in &migrations {
            if self.filtered_out(migration) {
                skipped += 1;
                continue;
            }
            let target = self.target_of(migration);
            if self
                .applied_for(&mut cache, target.clone())?
                .contains(&migration.version)
            {
                continue;
            }

            match &migration.connection {
                Some(name) => println!(
                    "  \x1b[33mMigrating\x1b[0m {} \x1b[90m[{}]\x1b[0m",
                    migration.full_name(),
                    name
                ),
                None => println!("  \x1b[33mMigrating\x1b[0m {}", migration.full_name()),
            }

            self.run_on(target.as_deref(), || {
                self.execute_migration(migration, "up")?;
                self.record_migration(migration)
            })?;
            // The version table for this connection just changed.
            if let Some(applied) = cache.get_mut(&target) {
                applied.push(migration.version.clone());
            }

            println!("  \x1b[32m   Applied\x1b[0m {}", migration.full_name());
            applied_migrations.push(migration.full_name());
        }

        if applied_migrations.is_empty() {
            return Ok(MigrationResult {
                applied: vec![],
                message: skip_note("No pending migrations", skipped),
            });
        }

        Ok(MigrationResult {
            message: skip_note(
                &format!("Applied {} migration(s)", applied_migrations.len()),
                skipped,
            ),
            applied: applied_migrations,
        })
    }

    /// Rollback the last migration
    pub fn migrate_down(&self) -> Result<MigrationResult, String> {
        let migrations = self.get_migrations()?;
        let mut cache: HashMap<Option<String>, Vec<String>> = HashMap::new();

        // The newest applied migration wins, checked against the version table
        // of whichever connection each one targets.
        let mut newest: Option<&Migration> = None;
        for migration in &migrations {
            if self.filtered_out(migration) {
                continue;
            }
            let target = self.target_of(migration);
            if !self
                .applied_for(&mut cache, target)?
                .contains(&migration.version)
            {
                continue;
            }
            if newest.is_none_or(|current| migration.version > current.version) {
                newest = Some(migration);
            }
        }

        let Some(migration) = newest else {
            return Ok(MigrationResult {
                applied: vec![],
                message: "No migrations to rollback".to_string(),
            });
        };

        match &migration.connection {
            Some(name) => println!(
                "  \x1b[33mRolling back\x1b[0m {} \x1b[90m[{}]\x1b[0m",
                migration.full_name(),
                name
            ),
            None => println!("  \x1b[33mRolling back\x1b[0m {}", migration.full_name()),
        }

        let target = self.target_of(migration);
        self.run_on(target.as_deref(), || {
            self.execute_migration(migration, "down")?;
            self.remove_migration_record(migration)
        })?;

        println!("  \x1b[32m   Reverted\x1b[0m {}", migration.full_name());

        Ok(MigrationResult {
            message: format!("Rolled back {}", migration.full_name()),
            applied: vec![migration.full_name()],
        })
    }

    /// Show migration status
    pub fn status(&self) -> Result<MigrationStatus, String> {
        let migrations = self.get_migrations()?;
        let mut cache: HashMap<Option<String>, Vec<String>> = HashMap::new();

        let mut statuses: Vec<MigrationStatusEntry> = Vec::with_capacity(migrations.len());
        for m in &migrations {
            if self.filtered_out(m) {
                continue;
            }
            let target = self.target_of(m);
            let applied = self.applied_for(&mut cache, target)?.contains(&m.version);
            statuses.push(MigrationStatusEntry {
                version: m.version.clone(),
                name: m.name.clone(),
                applied,
                connection: m.connection.clone(),
            });
        }

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
    /// Connection declared in the file, when it declares one.
    pub connection: Option<String>,
}

/// Append "(N skipped …)" when `--connection` held migrations back.
fn skip_note(message: &str, skipped: usize) -> String {
    if skipped == 0 {
        return message.to_string();
    }
    format!("{message} ({skipped} skipped — declared another connection)")
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
        r#"# Migration: {}
# Created: {}
#
# Runs on the default connection. To target another one, uncomment this —
# then `soli db:migrate up` places it correctly with no --connection flag:
#
# connection "legacy"

fn up(db: Any) -> Any {{
    # SoliDB collection helpers:
    #   db.create_collection("users")
    #   db.drop_collection("users")
    #   db.create_index("users", "idx_email", ["email"], {{ "unique": true }})
    #   db.query("FOR doc IN users RETURN doc")
    #
    # SQL adapters (postgres / mysql / sqlite) — a document table:
    #   db.create_table("users")
    #
    # SQL adapters — a table with real columns, portable across all three:
    #   db.create_table("users", {{
    #     "id":         "pk",
    #     "email":      {{ "type": "string", "limit": 255, "null": false }},
    #     "age":        "integer",
    #     "balance":    "decimal(10,2)",
    #     "active":     {{ "type": "boolean", "default": true }},
    #     "settings":   "json",
    #     "team_id":    {{ "type": "bigint", "references": "teams" }},
    #     "timestamps": true
    #   }})
    #   db.add_index("users", ["email"], {{ "unique": true }})
    #   db.add_column("users", "nickname", "string")
    #   db.rename_column("users", "nickname", "handle")
    #   db.drop_column("users", "handle")
    #   db.execute("…")   # engine-specific escape hatch
}}

fn down(db: Any) -> Any {{
    # Rollback the changes made in up()
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

    // Only show the connection column when a migration actually declares one,
    // so single-database apps see the same table as before.
    let show_connection = status.entries.iter().any(|e| e.connection.is_some());

    if show_connection {
        println!(
            "  {:14}  {:30}  {:12}  {:10}",
            "Version", "Name", "Connection", "Status"
        );
        println!("  {:-<14}  {:-<30}  {:-<12}  {:-<10}", "", "", "", "");
    } else {
        println!("  {:14}  {:30}  {:10}", "Version", "Name", "Status");
        println!("  {:-<14}  {:-<30}  {:-<10}", "", "", "");
    }

    for entry in &status.entries {
        let status_str = if entry.applied {
            "\x1b[32m   up   \x1b[0m"
        } else {
            "\x1b[33m  down  \x1b[0m"
        };

        if show_connection {
            println!(
                "  {:14}  {:30}  {:12}  {}",
                entry.version,
                entry.name,
                entry.connection.as_deref().unwrap_or("(default)"),
                status_str
            );
        } else {
            println!("  {:14}  {:30}  {}", entry.version, entry.name, status_str);
        }
    }

    println!();
    println!(
        "  \x1b[32m{}\x1b[0m applied, \x1b[33m{}\x1b[0m pending",
        status.applied_count, status.pending_count
    );
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_a_declared_connection() {
        assert_eq!(
            scan_connection("connection \"legacy\"\n\ndef up(db)\nend\n")
                .unwrap()
                .as_deref(),
            Some("legacy")
        );
        // Both spellings read naturally.
        assert_eq!(
            scan_connection("connection(\"warehouse\")")
                .unwrap()
                .as_deref(),
            Some("warehouse")
        );
        // Leading comments and blank lines do not hide it.
        assert_eq!(
            scan_connection("# Migration: create orders\n\nconnection \"analytics\"\n")
                .unwrap()
                .as_deref(),
            Some("analytics")
        );
    }

    #[test]
    fn a_file_without_a_declaration_targets_the_default() {
        assert_eq!(
            scan_connection("def up(db)\n  db.create_table(\"posts\")\nend").unwrap(),
            None
        );
        // A commented-out declaration must not count.
        assert_eq!(
            scan_connection("# connection \"legacy\"\ndef up(db)\nend").unwrap(),
            None
        );
        assert_eq!(scan_connection("// connection \"legacy\"").unwrap(), None);
        // Neither does a mention inside other code.
        assert_eq!(
            scan_connection("def up(db)\n  print(\"connection \\\"legacy\\\"\")\nend").unwrap(),
            None
        );
        assert_eq!(scan_connection("connection \"\"").unwrap(), None);
    }

    #[test]
    fn connection_inside_a_string_or_after_def_does_not_count() {
        // The first non-comment statement is the triple-quoted string, so the
        // `connection` line inside it is not a declaration.
        let source = "\"\"\"\nTo run against warehouse:\nconnection \"warehouse\"\n\"\"\"\n\ndef up(db)\nend\n";
        assert_eq!(scan_connection(source).unwrap(), None);
        // After `def` is too late.
        assert_eq!(
            scan_connection("def up(db)\nconnection \"legacy\"\nend").unwrap(),
            None
        );
    }

    #[test]
    fn a_second_connection_declaration_is_an_error() {
        let err = scan_connection("connection \"legacy\"\nconnection \"warehouse\"\n").unwrap_err();
        assert!(err.contains("only once"), "{err}");
    }

    #[test]
    fn the_declaration_is_neutralized_before_execution() {
        let source = "connection \"analytics\"\n\ndef up(db)\n  db.create_table(\"events\")\nend\n";
        let stripped = strip_connection_declaration(source);
        // Commented out, so it is not executed as a call…
        assert!(stripped.lines().next().unwrap().starts_with("# connection"));
        // …with the line count preserved, so error line numbers still match.
        assert_eq!(stripped.lines().count(), source.trim_end().lines().count());
        assert!(stripped.contains("db.create_table(\"events\")"));
        // A file without a declaration is untouched.
        let plain = "def up(db)\nend";
        assert_eq!(strip_connection_declaration(plain), plain);
        // A connection line inside a string is not the declaration — leave it.
        let quoted = "\"\"\"\nconnection \"warehouse\"\n\"\"\"\ndef up(db)\nend";
        assert_eq!(strip_connection_declaration(quoted), quoted);
    }

    #[test]
    fn the_cli_flag_is_a_filter_not_an_override() {
        let migration = |connection: Option<&str>| Migration {
            version: "20260812000001".into(),
            name: "create_orders".into(),
            path: PathBuf::from("db/migrations/20260812000001_create_orders.sl"),
            connection: connection.map(str::to_string),
            connection_error: None,
        };
        let runner = |filter: Option<&str>| {
            MigrationRunner::new(
                DbConfig::new("http://localhost:6745", "test"),
                Path::new("."),
            )
            .with_connection_filter(filter)
        };

        // No flag: the file's own declaration decides, else the default.
        assert_eq!(
            runner(None)
                .target_of(&migration(Some("legacy")))
                .as_deref(),
            Some("legacy")
        );
        assert_eq!(runner(None).target_of(&migration(None)), None);

        // With a flag: undeclared migrations follow it…
        assert_eq!(
            runner(Some("legacy"))
                .target_of(&migration(None))
                .as_deref(),
            Some("legacy")
        );
        // …a declared migration keeps its own target…
        assert_eq!(
            runner(Some("legacy"))
                .target_of(&migration(Some("legacy")))
                .as_deref(),
            Some("legacy")
        );
        // …and one belonging to another database is held back rather than
        // being run against the wrong schema.
        assert!(runner(Some("legacy")).filtered_out(&migration(Some("warehouse"))));
        assert!(!runner(Some("legacy")).filtered_out(&migration(Some("legacy"))));
        assert!(!runner(None).filtered_out(&migration(Some("warehouse"))));
    }

    #[test]
    fn skip_note_only_appears_when_something_was_skipped() {
        assert_eq!(
            skip_note("Applied 2 migration(s)", 0),
            "Applied 2 migration(s)"
        );
        assert!(skip_note("No pending migrations", 3).contains("3 skipped"));
    }

    /// Private SQLite file(s) + registry override. Same lock as the adapter
    /// tests so we never race another module's override.
    fn with_sqlite_app(names: &[&str], f: impl FnOnce(&Path)) {
        use crate::db::registry::{
            clear_registry_override, registry_test_lock, set_registry_for_tests,
            ConnectionRegistry, ConnectionSpec,
        };
        use crate::db::Adapter;
        use std::collections::HashMap;

        let _lock = registry_test_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let root = std::env::temp_dir().join(format!(
            "soli-mig-e2e-{}-{}",
            std::process::id(),
            names.join("-")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("db/migrations")).expect("app dir");

        let mut connections = HashMap::new();
        for name in names {
            let db_path = root.join(format!("{name}.sqlite3"));
            connections.insert(
                (*name).to_string(),
                ConnectionSpec {
                    name: (*name).to_string(),
                    adapter: Adapter::Sqlite,
                    url: Some(format!("sqlite://{}", db_path.display())),
                    solidb_host: None,
                    solidb_database: None,
                    solidb_username: None,
                    solidb_password: None,
                    solidb_api_key: None,
                    pool_size: Some(2),
                },
            );
        }
        set_registry_for_tests(ConnectionRegistry {
            default: names[0].to_string(),
            connections,
            from_file: false,
        });

        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::interpreter::builtins::model::clear_all_model_registries();
                crate::db::introspect::clear_schema_cache();
                clear_registry_override();
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone());
        f(&root);
    }

    fn write_migration(app: &Path, filename: &str, source: &str) {
        fs::write(app.join("db/migrations").join(filename), source).expect("write migration");
    }

    fn runner(app: &Path) -> MigrationRunner {
        MigrationRunner::new(DbConfig::new("unused", "unused"), app)
    }

    #[test]
    fn migrate_up_and_down_create_and_drop_a_column_table() {
        with_sqlite_app(&["primary"], |app| {
            write_migration(
                app,
                "20260813000001_create_invoices.sl",
                r#"
def up(db)
  db.create_table("invoices", {
    "id": "pk",
    "code": { "type": "string", "limit": 32, "null": false },
    "total": "decimal(10,2)",
    "timestamps": true
  })
  db.add_index("invoices", ["code"], { "unique": true })
end

def down(db)
  db.drop_table("invoices")
end
"#,
            );

            let up = runner(app).migrate_up().expect("up");
            assert_eq!(up.applied, vec!["20260813000001_create_invoices"]);
            crate::db::introspect::get_schema("invoices").expect("table exists after up");

            let again = runner(app).migrate_up().expect("second up");
            assert!(
                again.applied.is_empty(),
                "a second up must be a no-op: {}",
                again.message
            );

            let down = runner(app).migrate_down().expect("down");
            assert_eq!(down.applied, vec!["20260813000001_create_invoices"]);
            crate::db::introspect::clear_schema_cache();
            assert!(
                crate::db::introspect::get_schema("invoices").is_err(),
                "table must be gone after down"
            );
        });
    }

    #[test]
    fn run_migration_source_executes_create_table_and_execute() {
        with_sqlite_app(&["interp"], |app| {
            write_migration(
                app,
                "20260813000002_notes.sl",
                r#"
def up(db)
  db.create_table("notes", {
    "id": "pk",
    "body": "text"
  })
  db.add_column("notes", "author", "string")
  db.execute("CREATE INDEX IF NOT EXISTS idx_notes_body ON notes (body)")
end

def down(db)
  db.drop_table("notes")
end
"#,
            );
            runner(app).migrate_up().expect("up");
            let raw = crate::db::sqlite::introspect_table("notes").expect("introspect");
            let names: Vec<&str> = raw.columns.iter().map(|(name, ..)| name.as_str()).collect();
            assert!(names.contains(&"body"), "{names:?}");
            assert!(
                names.contains(&"author"),
                "add_column must have run: {names:?}"
            );
        });
    }

    #[test]
    fn a_reserved_table_name_fails_the_migration() {
        with_sqlite_app(&["reserved"], |app| {
            write_migration(
                app,
                "20260813000003_drop_jobs.sl",
                r#"
def up(db)
  db.create_table("_jobs", { "id": "pk" })
end

def down(db)
end
"#,
            );
            let err = match runner(app).migrate_up() {
                Ok(ok) => panic!(
                    "reserved create_table should fail, applied {:?}",
                    ok.applied
                ),
                Err(e) => e,
            };
            assert!(err.contains("reserved") || err.contains("_jobs"), "{err}");
        });
    }

    #[test]
    fn a_duplicate_connection_declaration_fails_discovery() {
        with_sqlite_app(&["dup"], |app| {
            write_migration(
                app,
                "20260813000004_dup.sl",
                "connection \"legacy\"\nconnection \"warehouse\"\ndef up(db)\nend\n",
            );
            let err = runner(app).get_migrations().unwrap_err();
            assert!(err.contains("only once"), "{err}");
        });
    }

    #[test]
    fn connection_filter_skips_a_file_that_declared_another_database() {
        with_sqlite_app(&["primary", "warehouse"], |app| {
            write_migration(
                app,
                "20260813000005_warehouse_events.sl",
                r#"
connection "warehouse"

def up(db)
  db.create_table("wh_events", { "id": "pk", "name": "string" })
end

def down(db)
  db.drop_table("wh_events")
end
"#,
            );
            write_migration(
                app,
                "20260813000006_default_posts.sl",
                r#"
def up(db)
  db.create_table("def_posts", { "id": "pk", "title": "string" })
end

def down(db)
  db.drop_table("def_posts")
end
"#,
            );

            let result = runner(app)
                .with_connection_filter(Some("primary"))
                .migrate_up()
                .expect("filtered up");
            assert!(
                result.message.contains("skipped") || result.applied.len() == 1,
                "{}",
                result.message
            );
            assert_eq!(result.applied, vec!["20260813000006_default_posts"]);

            crate::db::introspect::get_schema("def_posts").expect("default table");
            crate::db::with_connection("warehouse", || {
                crate::db::introspect::clear_schema_cache();
                assert!(
                    crate::db::introspect::get_schema("wh_events").is_err(),
                    "warehouse migration must not have run"
                );
            });
        });
    }

    #[test]
    fn a_migration_then_the_soli_column_model_spec() {
        with_sqlite_app(&["spec"], |app| {
            write_migration(
                app,
                "20260813000007_sql_invoices.sl",
                r#"
def up(db)
  db.create_table("sql_invoices", {
    "id": "pk",
    "code": { "type": "string", "limit": 32, "null": false },
    "qty": "integer",
    "paid": { "type": "boolean", "default": false },
    "timestamps": true
  })
end

def down(db)
  db.drop_table("sql_invoices")
end
"#,
            );
            runner(app).migrate_up().expect("up");

            let spec = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/builtins/sql_column_model_spec.sl");
            let source = fs::read_to_string(&spec).expect("read spec");
            let (assertions, result) = crate::run_with_path_and_coverage(
                &source,
                Some(&spec),
                false,
                None,
                Some(&spec),
                &[],
            );
            result.unwrap_or_else(|e| panic!("sql_column_model_spec.sl failed: {e}"));
            assert!(
                assertions >= 8,
                "the spec must have run its assertions, got {assertions}"
            );
        });
    }
}
