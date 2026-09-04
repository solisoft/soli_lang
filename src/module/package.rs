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
    /// The Soli interpreter version this project needs, in one of two forms.
    ///
    /// * `"1.16.0"` — a **minimum**. An older soli refuses to start.
    /// * `"=1.16.0"` — an **exact pin**. `soli` in this project switches to
    ///   that version, fetching it if necessary (see [`Package::exact_pin`]).
    ///
    /// One field rather than two so a floor and a pin can never contradict each
    /// other, and because the manifest parser rejects unknown `[package]` keys
    /// outright — a *new* key would make `soli add` fail on every older soli,
    /// where an unrecognised `=` prefix merely degrades to "floor satisfied".
    pub soli_version: Option<String>,
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
            soli_version: None,
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
                            "soli_version" => package.soli_version = Some(value),
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
        if let Some(ref v) = self.soli_version {
            out.push_str(&format!("soli_version = \"{}\"\n", v));
        }
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

    /// The exact version this manifest pins, when `soli_version` uses the `=`
    /// form. `None` for the plain minimum form, and `None` for a pin whose
    /// version string is not safe to use as a path or URL component.
    ///
    /// Validating here rather than at the use site matters: a `soli.toml` is
    /// author-controlled content in any cloned repository, and the pin becomes
    /// both a cache path and a download URL.
    pub fn exact_pin(&self) -> Option<&str> {
        let raw = self.soli_version.as_deref()?;
        let pinned = raw.strip_prefix('=')?.trim();
        is_valid_version(pinned).then_some(pinned)
    }

    /// Check the running Soli version against this manifest's `soli_version`.
    ///
    /// For the minimum form, `Ok` when `running` is at least the required
    /// version. For the exact form, `Ok` only when `running` *is* that version.
    ///
    /// This is the backstop for when the version switch did not happen — the
    /// command bypassed it, `SOLI_NO_PIN` was set, or the switch is not
    /// implemented on this platform. In the ordinary case the process has
    /// already become the pinned version by the time this runs, and it passes.
    pub fn check_soli_version(&self, running: &str) -> Result<(), String> {
        let Some(req) = &self.soli_version else {
            return Ok(());
        };

        if let Some(pin) = self.exact_pin() {
            // String equality, deliberately not `compare_versions`: that
            // comparator ignores pre-release suffixes, so it calls
            // `2.1.0-rc1` and `2.1.0` equal. Fine for a floor, wrong for a
            // reproducibility pin — an rc must not satisfy `=2.1.0`.
            if running.trim_start_matches('v') == pin {
                return Ok(());
            }
            return Err(format!(
                "Error: this project pins soli {pin},\n\
                 but you are running soli {running}.\n\
                 Run soli from the project directory to switch automatically \
                 (and check SOLI_NO_PIN is unset), or edit soli_version."
            ));
        }

        // A malformed pin (`"=../../evil"`) reaches here as a minimum, where
        // `compare_versions` reads its leading non-digits as 0 and the check
        // passes. That is the intended degradation: an unusable pin is ignored
        // rather than blocking the project.
        if compare_versions(running, req) == std::cmp::Ordering::Less {
            return Err(format!(
                "Error: this project requires soli >= {req},\n\
                 but you are running soli {running}.\n\
                 Upgrade with: soli update"
            ));
        }
        Ok(())
    }
}

/// Is this version string safe to interpolate into a filesystem path and a URL?
///
/// This is a security control, not a nicety. A `soli.toml` is author-controlled
/// content in any repository you clone, and the pinned version becomes both a
/// cache directory name and a release URL component. Without this,
/// `soli_version = "=../../../../tmp/evil"` escapes the cache root and the
/// release prefix.
///
/// Requires: starts with a digit (which alone rejects `..`), ASCII
/// alphanumerics plus `.` and `-` only, and a bounded length.
pub fn is_valid_version(v: &str) -> bool {
    const MAX_LEN: usize = 32;

    if v.is_empty() || v.len() > MAX_LEN {
        return false;
    }
    if !v.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    v.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Compare two dotted version strings (e.g. "1.9.0", "1.10.0-rc1") numerically,
/// component by component. Each component's leading digits are parsed as a
/// number, so `1.10.0` correctly sorts *after* `1.9.0` (a plain string compare
/// would get this backwards). Pre-release / build suffixes are ignored for
/// ordering — good enough for "is the running version at least the required
/// one?" gates.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|component| {
                component
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
                    .parse::<u64>()
                    .unwrap_or(0)
            })
            .collect()
    }
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Walk up from `start_dir` for a `soli.toml` and return the exact version it
/// pins, with the manifest's path for use in messages.
///
/// `None` when there is no manifest, no `soli_version`, a minimum-only
/// `soli_version`, an unparseable manifest, or a pin that fails
/// [`is_valid_version`] — the same best-effort posture as
/// [`enforce_min_soli_version`]. A project whose manifest we cannot read must
/// keep working on the soli the user invoked.
///
/// **`start_dir` must be absolute.** [`Package::find`] walks by `PathBuf::pop`,
/// which on a relative `"."` pops once and stops, so a relative argument only
/// ever examines its own directory.
pub fn pinned_soli_version(start_dir: &Path) -> Option<(String, PathBuf)> {
    let manifest = Package::find(start_dir)?;
    let pkg = Package::load(&manifest).ok()?;
    let pin = pkg.exact_pin()?.to_string();
    Some((pin, manifest))
}

/// Walk up from `start_dir` for a `soli.toml` and enforce its `soli_version`.
///
/// Best-effort: no manifest, no `soli_version`, or a manifest the subset parser
/// can't fully read all resolve to `Ok(())` — this check never *newly* blocks a
/// project the parser chokes on. Returns `Err` only when a minimum is declared
/// and the running interpreter is older than it.
pub fn enforce_min_soli_version(start_dir: &Path) -> Result<(), String> {
    let Some(manifest) = Package::find(start_dir) else {
        return Ok(());
    };
    let Ok(pkg) = Package::load(&manifest) else {
        return Ok(());
    };
    pkg.check_soli_version(env!("CARGO_PKG_VERSION"))
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

    #[test]
    fn test_parse_soli_version() {
        let content = r#"
[package]
name = "my-app"
version = "1.0.0"
soli_version = "1.16.0"
"#;

        let pkg = Package::parse(content).unwrap();
        assert_eq!(pkg.soli_version.as_deref(), Some("1.16.0"));
    }

    #[test]
    fn test_soli_version_roundtrips_through_to_toml() {
        // `soli add`/`remove` rewrite the manifest via to_toml — the minimum
        // version must survive that round-trip or it would be silently dropped.
        let content = r#"
[package]
name = "my-app"
version = "1.0.0"
soli_version = "1.16.0"

[dependencies]
utils = { path = "../utils" }
"#;

        let pkg = Package::parse(content).unwrap();
        let reparsed = Package::parse(&pkg.to_toml()).unwrap();
        assert_eq!(reparsed.soli_version.as_deref(), Some("1.16.0"));
    }

    #[test]
    fn test_check_soli_version_gate() {
        let mut pkg = Package::new("my-app");

        // No minimum declared → always Ok.
        assert!(pkg.check_soli_version("0.1.0").is_ok());

        pkg.soli_version = Some("1.16.0".to_string());

        // Running older than required → Err (numeric compare, not lexicographic).
        assert!(pkg.check_soli_version("1.15.9").is_err());
        assert!(pkg.check_soli_version("1.9.0").is_err());

        // Running equal or newer → Ok.
        assert!(pkg.check_soli_version("1.16.0").is_ok());
        assert!(pkg.check_soli_version("1.20.0").is_ok());
        assert!(pkg.check_soli_version("2.0.0").is_ok());
    }

    // ---- exact version pins (`soli_version = "=X.Y.Z"`) --------------------

    fn pinned(spec: &str) -> Package {
        let mut pkg = Package::new("my-app");
        pkg.soli_version = Some(spec.to_string());
        pkg
    }

    #[test]
    fn exact_pin_recognises_the_equals_prefix() {
        assert_eq!(pinned("=2.1.0").exact_pin(), Some("2.1.0"));
        // Whitespace after the operator is a natural thing to type.
        assert_eq!(pinned("= 2.1.0").exact_pin(), Some("2.1.0"));
    }

    #[test]
    fn a_plain_minimum_is_not_a_pin() {
        assert_eq!(pinned("1.16.0").exact_pin(), None);
        assert_eq!(Package::new("my-app").exact_pin(), None);
    }

    #[test]
    fn an_exact_pin_accepts_only_that_version() {
        let pkg = pinned("=2.1.0");

        assert!(pkg.check_soli_version("2.1.0").is_ok());
        // A leading `v` on the running version is tolerated, as elsewhere.
        assert!(pkg.check_soli_version("v2.1.0").is_ok());

        // Both directions are a mismatch — a pin is not a floor.
        assert!(pkg.check_soli_version("2.1.1").is_err());
        assert!(pkg.check_soli_version("2.0.9").is_err());
    }

    /// The reason exact pins compare by string equality rather than through
    /// `compare_versions`: that comparator deliberately ignores pre-release
    /// suffixes, so it calls `2.1.0-rc1` and `2.1.0` equal. Fine for a floor,
    /// wrong for a reproducibility pin.
    #[test]
    fn a_prerelease_does_not_satisfy_an_exact_pin() {
        assert_eq!(
            compare_versions("2.1.0-rc1", "2.1.0"),
            std::cmp::Ordering::Equal,
            "guard: this test exists because compare_versions says they are equal"
        );
        assert!(pinned("=2.1.0").check_soli_version("2.1.0-rc1").is_err());
        assert!(pinned("=2.1.0-rc1").check_soli_version("2.1.0").is_err());
        // But an rc can be pinned exactly.
        assert!(pinned("=2.1.0-rc1").check_soli_version("2.1.0-rc1").is_ok());
    }

    /// Pinning an *older* soli is the point of a pin, so the message must not
    /// be the "upgrade with: soli update" one, which would be wrong advice.
    #[test]
    fn pinning_an_older_version_is_a_mismatch_not_an_upgrade_prompt() {
        let err = pinned("=1.22.0")
            .check_soli_version("2.0.5")
            .expect_err("a newer running soli does not satisfy an exact pin");

        assert!(err.contains("pins soli 1.22.0"), "{err}");
        assert!(!err.contains("soli update"), "{err}");
        // The message must not assume SOLI_NO_PIN is why the switch did not
        // happen — the usual reason is being outside the project directory.
        assert!(err.contains("project directory"), "{err}");
    }

    #[test]
    fn is_valid_version_rejects_path_traversal_and_urls() {
        for bad in [
            "",
            "..",
            "../../etc",
            "2.1.0/../..",
            "/etc/passwd",
            "https://evil.example",
            "v2.1.0", // must start with a digit
            "2.1.0 ", // no whitespace
            "2.1.0;rm -rf /",
            "2.1.0%2f..",
        ] {
            assert!(!is_valid_version(bad), "{bad:?} should be rejected");
        }
        // 33 characters, one past the cap.
        assert!(!is_valid_version(&"1".repeat(33)));

        for good in ["2.1.0", "2.1.0-rc1", "10.0.0", "0.1.0", "2"] {
            assert!(is_valid_version(good), "{good:?} should be accepted");
        }
    }

    /// A pin that could escape the cache directory or the release URL is
    /// ignored rather than acted on — and, because it then falls through to the
    /// minimum path where `compare_versions` reads its leading non-digits as
    /// zero, it does not block the project either.
    #[test]
    fn a_malformed_pin_is_ignored_rather_than_obeyed() {
        let pkg = pinned("=../../../../tmp/evil");

        assert_eq!(pkg.exact_pin(), None);
        assert!(pkg.check_soli_version("2.0.5").is_ok());
    }

    #[test]
    fn to_toml_round_trips_an_exact_pin() {
        let pkg = pinned("=2.1.0");
        let reparsed = Package::parse(&pkg.to_toml()).unwrap();

        assert_eq!(reparsed.soli_version.as_deref(), Some("=2.1.0"));
        assert_eq!(reparsed.exact_pin(), Some("2.1.0"));
    }

    /// The upward walk is the whole point: you run `soli` from `app/models/`,
    /// and the manifest is three levels up.
    #[test]
    fn pinned_version_is_found_from_a_nested_subdirectory() {
        let root = tempfile::tempdir().expect("tempdir");
        let nested = root.path().join("app").join("models").join("deep");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(
            root.path().join("soli.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nsoli_version = \"=2.1.0\"\n",
        )
        .expect("write manifest");

        let (pin, manifest) =
            pinned_soli_version(&nested).expect("the walk must reach the grandparent manifest");

        assert_eq!(pin, "2.1.0");
        assert_eq!(manifest, root.path().join("soli.toml"));
    }

    #[test]
    fn pinned_version_is_none_without_a_manifest_or_without_a_pin() {
        let empty = tempfile::tempdir().expect("tempdir");
        assert!(pinned_soli_version(empty.path()).is_none());

        let floor_only = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            floor_only.path().join("soli.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nsoli_version = \"1.16.0\"\n",
        )
        .expect("write manifest");
        assert!(pinned_soli_version(floor_only.path()).is_none());
    }
}
