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
///
/// `SOLI_PROTECT_ENV` (comma-separated list of variable names) names env
/// vars the parent process explicitly set and that this loader must NOT
/// override, even when `override_existing` is true. Used by the parallel
/// test runner so each per-worker `SOLIDB_DATABASE=test_wN` survives the
/// `.env.test` reload that happens during server startup.
pub fn load_env_file(folder: &Path, filename: &str, override_existing: bool) {
    let env_file = folder.join(filename);
    if !env_file.exists() {
        return;
    }

    let Ok(content) = std::fs::read_to_string(&env_file) else {
        return;
    };

    let protected: Vec<String> = std::env::var("SOLI_PROTECT_ENV")
        .ok()
        .map(|s| {
            s.split(',')
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default();

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

        let is_protected = protected.iter().any(|p| p == key);
        let already_set = std::env::var(key).is_ok();

        if (override_existing && !is_protected) || !already_set {
            // TODO: Audit that the environment access only happens in single-threaded code.
            unsafe { std::env::set_var(key, value) };
        }
    }
}
