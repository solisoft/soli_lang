//! Migration scaffolding generator

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::migration;
use crate::scaffold::utils::{to_pascal_case, to_snake_case_plural};
use crate::scaffold::FieldDefinition;

/// Create a migration for a scaffold
pub fn create_migration(
    app_path: &Path,
    name: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let collection_name = to_snake_case_plural(name);
    let migration_name = format!("create_{}", collection_name);

    // Generate timestamp for migration filename
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs();

    let filename = format!("{}{}_{}.sl", timestamp, migration_name, timestamp);
    let migrations_dir = app_path.join("db/migrations");
    let migration_path = migrations_dir.join(&filename);

    // Create migrations directory if it doesn't exist
    fs::create_dir_all(&migrations_dir)
        .map_err(|e| format!("Failed to create migrations directory: {}", e))?;

    // Create indexes for unique fields
    let unique_indexes: Vec<String> = fields
        .iter()
        .filter(|f| matches!(f.field_type.as_str(), "email" | "password"))
        .map(|f| {
            format!(
                r#"    db.create_index("{collection}", "idx_{field_name}", ["{field_name}"], {{ "unique": true }});"#,
                collection = collection_name,
                field_name = f.to_snake_case()
            )
        })
        .collect();

    let indexes = if unique_indexes.is_empty() {
        "    // No indexes defined".to_string()
    } else {
        unique_indexes.join("\n")
    };

    let content = migration::migration_template(
        &migration_name,
        name,
        &to_pascal_case(name),
        &collection_name,
        &indexes,
    );

    write_file(&migration_path, &content)?;

    Ok(())
}
