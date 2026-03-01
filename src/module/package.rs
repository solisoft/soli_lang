//! Package file (soli.toml) parsing.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// Directory containing soli.toml (set by Package::load)
    pub package_dir: Option<PathBuf>,
}

/// A package dependency.
#[derive(Debug, Clone)]
pub enum Dependency {
    /// Local path dependency
    Path(String),
    /// Version from registry (future)
    Version(String),
    /// Git repository dependency
    Git {
        url: String,
        tag: Option<String>,
        branch: Option<String>,
        rev: Option<String>,
    },
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
            package_dir: None,
        }
    }

    /// Load a package from a soli.toml file.
    pub fn load(path: &Path) -> Result<Self, PackageError> {
        let content = fs::read_to_string(path)?;
        let mut pkg = Self::parse(&content)?;
        pkg.package_dir = path.parent().map(|p| p.to_path_buf());
        Ok(pkg)
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

    // Inline table: { path = "..." } or { git = "...", tag = "v1.0" }
    if value.starts_with('{') && value.ends_with('}') {
        let inner = value[1..value.len() - 1].trim();
        let pairs = parse_inline_table_pairs(inner)?;

        if pairs.contains_key("git") {
            return Ok(Dependency::Git {
                url: pairs.get("git").cloned().unwrap_or_default(),
                tag: pairs.get("tag").cloned(),
                branch: pairs.get("branch").cloned(),
                rev: pairs.get("rev").cloned(),
            });
        }

        if let Some(path) = pairs.get("path") {
            return Ok(Dependency::Path(path.clone()));
        }

        if let Some(version) = pairs.get("version") {
            return Ok(Dependency::Version(version.clone()));
        }

        return Err(PackageError::ParseError(format!(
            "Inline table must contain 'path', 'version', or 'git' key: {}",
            value
        )));
    }

    Err(PackageError::ParseError(format!(
        "Invalid dependency value: {}",
        value
    )))
}

/// Parse comma-separated key=value pairs from an inline table, respecting quoted values.
fn parse_inline_table_pairs(inner: &str) -> Result<HashMap<String, String>, PackageError> {
    let mut pairs = HashMap::new();
    let mut remaining = inner;

    while !remaining.is_empty() {
        // Find key = value
        let eq_pos = remaining.find('=').ok_or_else(|| {
            PackageError::ParseError(format!("Expected key = value in inline table: {}", inner))
        })?;

        let key = remaining[..eq_pos].trim();
        remaining = remaining[eq_pos + 1..].trim_start();

        // Parse the value (quoted string)
        let (val, rest) = if let Some(stripped) = remaining.strip_prefix('"') {
            // Find closing quote
            let end = stripped.find('"').ok_or_else(|| {
                PackageError::ParseError(format!("Unterminated string in inline table: {}", inner))
            })?;
            let val = &stripped[..end];
            let rest = stripped[end + 1..].trim_start();
            // Skip comma if present
            let rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
            (val.to_string(), rest)
        } else if let Some(stripped) = remaining.strip_prefix('\'') {
            let end = stripped.find('\'').ok_or_else(|| {
                PackageError::ParseError(format!("Unterminated string in inline table: {}", inner))
            })?;
            let val = &stripped[..end];
            let rest = stripped[end + 1..].trim_start();
            let rest = rest.strip_prefix(',').unwrap_or(rest).trim_start();
            (val.to_string(), rest)
        } else {
            // Unquoted value - read until comma or end
            if let Some(comma) = remaining.find(',') {
                let val = remaining[..comma].trim();
                (val.to_string(), remaining[comma + 1..].trim_start())
            } else {
                (remaining.trim().to_string(), "")
            }
        };

        pairs.insert(key.to_string(), val);
        remaining = rest;
    }

    Ok(pairs)
}

impl Package {
    /// Serialize the package back to TOML format.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();

        out.push_str("[package]\n");
        out.push_str(&format!("name = \"{}\"\n", self.name));
        out.push_str(&format!("version = \"{}\"\n", self.version));
        if let Some(ref desc) = self.description {
            out.push_str(&format!("description = \"{}\"\n", desc));
        }
        if self.main != "app.sl" {
            out.push_str(&format!("main = \"{}\"\n", self.main));
        }

        if !self.dependencies.is_empty() {
            out.push_str("\n[dependencies]\n");
            // Sort dependencies for deterministic output
            let mut deps: Vec<_> = self.dependencies.iter().collect();
            deps.sort_by_key(|(k, _)| (*k).clone());
            for (name, dep) in deps {
                match dep {
                    Dependency::Path(p) => {
                        out.push_str(&format!("{} = {{ path = \"{}\" }}\n", name, p));
                    }
                    Dependency::Version(v) => {
                        out.push_str(&format!("{} = \"{}\"\n", name, v));
                    }
                    Dependency::Git {
                        url,
                        tag,
                        branch,
                        rev,
                    } => {
                        let mut parts = vec![format!("git = \"{}\"", url)];
                        if let Some(t) = tag {
                            parts.push(format!("tag = \"{}\"", t));
                        }
                        if let Some(b) = branch {
                            parts.push(format!("branch = \"{}\"", b));
                        }
                        if let Some(r) = rev {
                            parts.push(format!("rev = \"{}\"", r));
                        }
                        out.push_str(&format!("{} = {{ {} }}\n", name, parts.join(", ")));
                    }
                }
            }
        }

        out
    }
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

    #[test]
    fn test_parse_git_dependency() {
        let content = r#"
[package]
name = "my-app"
version = "1.0.0"

[dependencies]
math = { git = "https://github.com/user/soli-math", tag = "v1.0.0" }
utils = { git = "https://github.com/user/soli-utils", branch = "main" }
core = { git = "https://github.com/user/soli-core", rev = "abc123" }
"#;

        let pkg = Package::parse(content).unwrap();
        assert_eq!(pkg.dependencies.len(), 3);

        match &pkg.dependencies["math"] {
            Dependency::Git {
                url,
                tag,
                branch,
                rev,
            } => {
                assert_eq!(url, "https://github.com/user/soli-math");
                assert_eq!(tag.as_deref(), Some("v1.0.0"));
                assert!(branch.is_none());
                assert!(rev.is_none());
            }
            _ => panic!("Expected git dependency"),
        }

        match &pkg.dependencies["utils"] {
            Dependency::Git {
                url,
                tag,
                branch,
                rev,
            } => {
                assert_eq!(url, "https://github.com/user/soli-utils");
                assert!(tag.is_none());
                assert_eq!(branch.as_deref(), Some("main"));
                assert!(rev.is_none());
            }
            _ => panic!("Expected git dependency"),
        }

        match &pkg.dependencies["core"] {
            Dependency::Git {
                url,
                tag,
                branch,
                rev,
            } => {
                assert_eq!(url, "https://github.com/user/soli-core");
                assert!(tag.is_none());
                assert!(branch.is_none());
                assert_eq!(rev.as_deref(), Some("abc123"));
            }
            _ => panic!("Expected git dependency"),
        }
    }

    #[test]
    fn test_to_toml_roundtrip() {
        let content = r#"
[package]
name = "my-app"
version = "1.0.0"

[dependencies]
math = { git = "https://github.com/user/soli-math", tag = "v1.0.0" }
utils = { path = "../utils" }
"#;

        let pkg = Package::parse(content).unwrap();
        let toml_str = pkg.to_toml();
        let pkg2 = Package::parse(&toml_str).unwrap();

        assert_eq!(pkg2.name, "my-app");
        assert_eq!(pkg2.version, "1.0.0");
        assert_eq!(pkg2.dependencies.len(), 2);

        match &pkg2.dependencies["math"] {
            Dependency::Git { url, tag, .. } => {
                assert_eq!(url, "https://github.com/user/soli-math");
                assert_eq!(tag.as_deref(), Some("v1.0.0"));
            }
            _ => panic!("Expected git dependency"),
        }
    }
}
