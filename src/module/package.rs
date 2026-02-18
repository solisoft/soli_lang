//! Package file (soli.toml) parsing.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

const DEFAULT_VERSION: &str = env!("CARGO_PKG_VERSION", "0.2.0");

/// A Solilang package configuration.
#[derive(Debug, Clone, Default)]
pub struct Package {
    /// Package name
    pub name: String,
    /// Package version
    pub version: String,
    /// Package description
    pub description: Option<String>,
    /// Main entry point (default: app.sl)
    pub main: String,
    /// Dependencies: name -> path or version
    pub dependencies: HashMap<String, Dependency>,
}

/// A package dependency.
#[derive(Debug, Clone)]
pub enum Dependency {
    /// Local path dependency
    Path(String),
    /// Version from registry (future)
    Version(String),
}

/// Errors that can occur during package parsing.
#[derive(Debug)]
pub enum PackageError {
    IoError(std::io::Error),
    ParseError(String),
    InvalidField(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageError::IoError(e) => write!(f, "IO error: {}", e),
            PackageError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            PackageError::InvalidField(field) => write!(f, "Invalid field: {}", field),
        }
    }
}

impl std::error::Error for PackageError {}

impl From<std::io::Error> for PackageError {
    fn from(e: std::io::Error) -> Self {
        PackageError::IoError(e)
    }
}

impl Package {
    /// Create a new package with default values.
    pub fn new(name: &str) -> Self {
        Package {
            name: name.to_string(),
            version: DEFAULT_VERSION.to_string(),
            description: None,
            main: "app.sl".to_string(),
            dependencies: HashMap::new(),
        }
    }

    /// Load a package from a soli.toml file.
    pub fn load(path: &Path) -> Result<Self, PackageError> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse a soli.toml content string.
    ///
    /// Simple TOML subset parser supporting:
    /// - [package] section with name, version, description, main
    /// - [dependencies] section with name = "path" or name = { path = "..." }
    pub fn parse(content: &str) -> Result<Self, PackageError> {
        let mut package = Package::default();
        let mut current_section: Option<&str> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Section header
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];
                current_section = Some(match section {
                    "package" => "package",
                    "dependencies" => "dependencies",
                    _ => {
                        return Err(PackageError::ParseError(format!(
                            "Unknown section: {}",
                            section
                        )))
                    }
                });
                continue;
            }

            // Key = value
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match current_section {
                    Some("package") => {
                        let value = parse_string_value(value)?;
                        match key {
                            "name" => package.name = value,
                            "version" => package.version = value,
                            "description" => package.description = Some(value),
                            "main" => package.main = value,
                            _ => {
                                return Err(PackageError::InvalidField(format!("package.{}", key)))
                            }
                        }
                    }
                    Some("dependencies") => {
                        let dep = parse_dependency(value)?;
                        package.dependencies.insert(key.to_string(), dep);
                    }
                    None => {
                        return Err(PackageError::ParseError(
                            "Key-value outside of section".to_string(),
                        ))
                    }
                    _ => {}
                }
            }
        }

        // Validate required fields
        if package.name.is_empty() {
            return Err(PackageError::ParseError(
                "Missing required field: package.name".to_string(),
            ));
        }

        Ok(package)
    }

    /// Find the soli.toml in the given directory or parent directories.
    pub fn find(start_dir: &Path) -> Option<std::path::PathBuf> {
        let mut current = start_dir.to_path_buf();

        loop {
            let package_file = current.join("soli.toml");
            if package_file.exists() {
                return Some(package_file);
            }

            if !current.pop() {
                return None;
            }
        }
    }
}

/// Parse a TOML string value (with or without quotes).
fn parse_string_value(value: &str) -> Result<String, PackageError> {
    let value = value.trim();

    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        Ok(value[1..value.len() - 1].to_string())
    } else {
        // Unquoted value
        Ok(value.to_string())
    }
}

/// Parse a dependency value.
fn parse_dependency(value: &str) -> Result<Dependency, PackageError> {
    let value = value.trim();

    // Simple string: "path/to/dep"
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        let path = &value[1..value.len() - 1];
        if path.starts_with('.') || path.starts_with('/') || path.contains('/') {
            return Ok(Dependency::Path(path.to_string()));
        } else {
            return Ok(Dependency::Version(path.to_string()));
        }
    }

    // Inline table: { path = "..." }
    if value.starts_with('{') && value.ends_with('}') {
        let inner = &value[1..value.len() - 1].trim();
        if let Some((key, val)) = inner.split_once('=') {
            let key = key.trim();
            let val = parse_string_value(val.trim())?;

            match key {
                "path" => return Ok(Dependency::Path(val)),
                "version" => return Ok(Dependency::Version(val)),
                _ => return Err(PackageError::InvalidField(format!("dependency.{}", key))),
            }
        }
    }

    Err(PackageError::ParseError(format!(
        "Invalid dependency value: {}",
        value
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_package() {
        let content = r#"
[package]
name = "my-app"
version = "1.0.0"
main = "app.sl"
"#;

        let pkg = Package::parse(content).unwrap();
        assert_eq!(pkg.name, "my-app");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.main, "app.sl");
    }

    #[test]
    fn test_parse_with_dependencies() {
        let content = r#"
[package]
name = "my-app"
version = "0.2.0"

[dependencies]
utils = "./lib/utils"
http = { path = "../http-lib" }
"#;

        let pkg = Package::parse(content).unwrap();
        assert_eq!(pkg.name, "my-app");
        assert_eq!(pkg.dependencies.len(), 2);

        match &pkg.dependencies["utils"] {
            Dependency::Path(p) => assert_eq!(p, "./lib/utils"),
            _ => panic!("Expected path dependency"),
        }

        match &pkg.dependencies["http"] {
            Dependency::Path(p) => assert_eq!(p, "../http-lib"),
            _ => panic!("Expected path dependency"),
        }
    }
}
