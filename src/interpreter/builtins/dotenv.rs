//! Dotenv file loading built-in functions.

use std::fs;
use std::path::Path;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Load a single .env file and return the count of loaded variables.
fn load_single_env_file(path: &Path) -> Result<i64, String> {
    if !path.exists() {
        return Ok(0);
    }

    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read .env file: {}", e))?;

    let mut loaded = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = parse_env_line(line) {
            std::env::set_var(&key, &value);
            loaded += 1;
        }
    }

    Ok(loaded)
}

/// Load environment variables from a .env file.
pub fn register_dotenv_builtins(env: &mut Environment) {
    env.define(
        "dotenv".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv", None, |args| {
            match args.first() {
                // If a path is provided, load only that specific file
                Some(Value::String(s)) => {
                    let path = Path::new(s);
                    if !path.exists() {
                        return Err(format!(".env file not found: {}", path.display()));
                    }
                    let loaded = load_single_env_file(path)?;
                    Ok(Value::Int(loaded))
                }
                Some(_) => Err("dotenv() expects optional string path".to_string()),
                // No path provided: load .env then .env.{APP_ENV}
                None => {
                    let base_path = Path::new(".env");
                    if !base_path.exists() {
                        return Err(format!(".env file not found: {}", base_path.display()));
                    }

                    // Load base .env first
                    let mut total_loaded = load_single_env_file(base_path)?;

                    // Then load environment-specific file if APP_ENV is set
                    if let Ok(app_env) = std::env::var("APP_ENV") {
                        let env_specific_path = format!(".env.{}", app_env);
                        let env_specific = Path::new(&env_specific_path);
                        if env_specific.exists() {
                            total_loaded += load_single_env_file(env_specific)?;
                        }
                    }

                    Ok(Value::Int(total_loaded))
                }
            }
        })),
    );

    env.define(
        "dotenv!".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv!", None, |args| {
            match args.first() {
                // If a path is provided, load only that specific file
                Some(Value::String(s)) => {
                    let path = Path::new(s);
                    if !path.exists() {
                        return Err(format!(".env file not found: {}", path.display()));
                    }
                    let loaded = load_single_env_file(path)?;
                    Ok(Value::Int(loaded))
                }
                Some(_) => Err("dotenv!() expects optional string path".to_string()),
                // No path provided: load .env then .env.{APP_ENV}
                None => {
                    let base_path = Path::new(".env");
                    if !base_path.exists() {
                        return Err(format!(".env file not found: {}", base_path.display()));
                    }

                    // Load base .env first
                    let mut total_loaded = load_single_env_file(base_path)?;

                    // Then load environment-specific file if APP_ENV is set
                    if let Ok(app_env) = std::env::var("APP_ENV") {
                        let env_specific_path = format!(".env.{}", app_env);
                        let env_specific = Path::new(&env_specific_path);
                        if env_specific.exists() {
                            total_loaded += load_single_env_file(env_specific)?;
                        }
                    }

                    Ok(Value::Int(total_loaded))
                }
            }
        })),
    );
}

fn parse_env_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();

    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let (key, value) = match line.split_once('=') {
        Some((k, v)) => (k.trim(), v.trim()),
        None => return None,
    };

    if key.is_empty() {
        return None;
    }

    let key = key.to_string();
    let value = unquote(value).to_string();

    Some((key, value))
}

fn unquote(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        &s[1..s.len() - 1]
    } else {
        s
    }
}
