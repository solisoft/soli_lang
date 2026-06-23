//! Seed scaffolding generator
//!
//! Creates timestamped seed files under `db/seeds/`. Seeds are ordinary Soli
//! scripts run by `soli db:seed`; unlike migrations they are NOT tracked, so
//! they run every invocation — authors keep them idempotent themselves.

use std::fs;
use std::path::{Path, PathBuf};

/// Generate a new seed file at `db/seeds/<timestamp>_<name>.sl`.
pub fn generate_seed(app_path: &Path, name: &str) -> Result<PathBuf, String> {
    let seeds_path = app_path.join("db/seeds");

    // Create the seeds directory if it doesn't exist
    fs::create_dir_all(&seeds_path)
        .map_err(|e| format!("Failed to create seeds directory: {}", e))?;

    // Generate timestamp (same format as migrations)
    let timestamp = chrono::Local::now().format("%Y%m%d%H%M%S");

    // Sanitize name
    let safe_name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    let filename = format!("{}_{}.sl", timestamp, safe_name);
    let filepath = seeds_path.join(&filename);

    let template = seed_template(name);

    fs::write(&filepath, template).map_err(|e| format!("Failed to write seed file: {}", e))?;

    Ok(filepath)
}

/// The starter body written into a generated seed file (and the new-app stub).
pub fn seed_template(name: &str) -> String {
    format!(
        r#"# Seed: {name}
# Run with: soli db:seed
#
# Seeds are NOT tracked and re-run every time, so make them idempotent.
# Guard with first_by / find_by instead of a blind create():
#
#   10.times do |i|
#     let email = "user\(i)@example.com"
#     User.create({{ "name": "User \(i)", "email": email }}) if User.first_by("email", email).nil?
#   end

print("Seeded {name}")
"#,
        name = name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_seed_creates_timestamped_file_under_db_seeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = generate_seed(dir.path(), "demo_users").unwrap();

        assert!(path.is_file());
        assert!(path.starts_with(dir.path().join("db/seeds")));

        let filename = path.file_name().unwrap().to_str().unwrap();
        // <14-digit timestamp>_demo_users.sl
        assert!(filename.ends_with("_demo_users.sl"), "got {}", filename);
        let stamp = filename.split('_').next().unwrap();
        assert_eq!(stamp.len(), 14);
        assert!(stamp.chars().all(|c| c.is_ascii_digit()));

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("# Seed: demo_users"));
        assert!(body.contains("print(\"Seeded demo_users\")"));
    }

    #[test]
    fn generate_seed_sanitizes_unsafe_name_characters() {
        let dir = tempfile::tempdir().unwrap();
        let path = generate_seed(dir.path(), "weird/../name").unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();
        // The four non-alphanumeric chars `/ . . /` each become `_`.
        assert!(filename.ends_with("_weird____name.sl"), "got {}", filename);
    }

    #[test]
    fn seed_template_uses_soli_interpolation_not_hash_braces() {
        let body = seed_template("things");
        // Soli string interpolation is `\(expr)`, never `#{expr}`.
        assert!(!body.contains("#{"));
        assert!(body.contains("\\(i)"));
    }
}
