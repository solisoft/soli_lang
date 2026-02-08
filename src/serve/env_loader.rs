//! Environment file loading utilities

use std::path::Path;

/// Load environment variables from .env files in the application directory.
/// This loads .env first, then .env.{APP_ENV} if APP_ENV is set.
pub fn load_env_files(folder: &Path) {
    load_env_file(folder, ".env", false);

    if let Ok(app_env) = std::env::var("APP_ENV") {
        load_env_file(folder, &format!(".env.{}", app_env), true);
    }
}

/// Load a single .env file
///
/// # Arguments
/// * `folder` - The directory containing the .env file
/// * `filename` - The name of the .env file
/// * `override_existing` - Whether to override existing environment variables
pub fn load_env_file(folder: &Path, filename: &str, override_existing: bool) {
    let env_file = folder.join(filename);
    if !env_file.exists() {
        return;
    }

    let Ok(content) = std::fs::read_to_string(&env_file) else {
        return;
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');

        if override_existing || std::env::var(key).is_err() {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var(key, value) };
        }
    }
}
