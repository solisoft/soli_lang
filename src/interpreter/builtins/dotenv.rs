//! Dotenv file loading built-in functions.

use std::fs;
use std::path::Path;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Load environment variables from a .env file.
pub fn register_dotenv_builtins(env: &mut Environment) {
    env.define(
        "dotenv".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv", None, |args| {
            let path = match args.first() {
                Some(Value::String(s)) => s.clone(),
                Some(_) => return Err("dotenv() expects optional string path".to_string()),
                None => ".env".to_string(),
            };

            let path = Path::new(&path);
            if !path.exists() {
                return Err(format!(".env file not found: {}", path.display()));
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

            Ok(Value::Int(loaded))
        })),
    );

    env.define(
        "dotenv!".to_string(),
        Value::NativeFunction(NativeFunction::new("dotenv!", None, |args| {
            let path = match args.first() {
                Some(Value::String(s)) => s.clone(),
                Some(_) => return Err("dotenv!() expects optional string path".to_string()),
                None => ".env".to_string(),
            };

            let path = Path::new(&path);
            if !path.exists() {
                return Err(format!(".env file not found: {}", path.display()));
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

            Ok(Value::Int(loaded))
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
