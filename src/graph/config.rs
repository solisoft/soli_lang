//! Configuration for the generic (non-Soli) `soli graph build` path: which file
//! extensions to index and which paths to skip.
//!
//! Precedence: CLI flags > `.soligraph.toml` (project root) > built-in defaults.
//! SolidB settings stay in `.env` (loaded separately, like the Soli path).

use std::path::Path;

/// Directory names skipped by default during the walk (dot-directories are
/// always skipped too).
const DEFAULT_EXCLUDE_DIRS: &[&str] = &[
    "node_modules",
    "vendor",
    "tmp",
    "log",
    "logs",
    "target",
    "dist",
    "build",
    "coverage",
    "public",
    "__pycache__",
    ".soli",
];

/// Resolved config for a generic build.
#[derive(Debug, Clone)]
pub struct GraphConfig {
    /// Extensions to index (no leading dot, lowercased).
    pub extensions: Vec<String>,
    /// Directory names to skip (in addition to dot-dirs).
    pub exclude_dirs: Vec<String>,
    /// Substrings; a project-relative path containing any is skipped.
    pub exclude_globs: Vec<String>,
    /// Line window for chunk-embedding files without structural extraction.
    pub chunk_lines: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        GraphConfig {
            extensions: Vec::new(),
            exclude_dirs: DEFAULT_EXCLUDE_DIRS.iter().map(|s| s.to_string()).collect(),
            exclude_globs: Vec::new(),
            chunk_lines: 50,
        }
    }
}

/// `.soligraph.toml` shape.
#[derive(Debug, serde::Deserialize, Default)]
struct FileConfig {
    extensions: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    exclude_dirs: Option<Vec<String>>,
    chunk_lines: Option<usize>,
}

impl GraphConfig {
    /// Build the config from a `.soligraph.toml` (if present) overlaid with CLI
    /// flags. `ext_flag`/`exclude_flag` are comma-separated.
    pub fn load(
        app_path: &Path,
        ext_flag: Option<&str>,
        exclude_flag: Option<&str>,
        config_path: Option<&str>,
    ) -> GraphConfig {
        let mut cfg = GraphConfig::default();

        let cfg_file = config_path
            .map(|p| app_path.join(p))
            .unwrap_or_else(|| app_path.join(".soligraph.toml"));
        if let Ok(text) = std::fs::read_to_string(&cfg_file) {
            match toml::from_str::<FileConfig>(&text) {
                Ok(parsed) => {
                    if let Some(exts) = parsed.extensions {
                        cfg.extensions = normalize_exts(exts.iter().map(|s| s.as_str()));
                    }
                    if let Some(globs) = parsed.exclude {
                        cfg.exclude_globs = globs;
                    }
                    if let Some(dirs) = parsed.exclude_dirs {
                        cfg.exclude_dirs.extend(dirs);
                    }
                    if let Some(n) = parsed.chunk_lines {
                        cfg.chunk_lines = n.max(1);
                    }
                }
                Err(e) => eprintln!("Warning: ignoring {}: {}", cfg_file.display(), e),
            }
        }

        if let Some(exts) = ext_flag {
            cfg.extensions = normalize_exts(exts.split(','));
        }
        if let Some(globs) = exclude_flag {
            cfg.exclude_globs.extend(
                globs
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            );
        }
        cfg
    }

    /// Whether the generic (non-Soli) path should be used — i.e. the user asked
    /// for specific extensions via flag or config file.
    pub fn has_extensions(&self) -> bool {
        !self.extensions.is_empty()
    }

    /// Should this project-relative path be indexed? (extension matches and no
    /// exclude glob hits).
    pub fn matches(&self, relpath: &str) -> bool {
        let ext = relpath
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !self.extensions.iter().any(|e| e == &ext) {
            return false;
        }
        !self
            .exclude_globs
            .iter()
            .any(|g| relpath.contains(g.as_str()))
    }

    /// Should the walk descend into this directory name?
    pub fn allows_dir(&self, name: &str) -> bool {
        if name.starts_with('.') {
            return false;
        }
        !self.exclude_dirs.iter().any(|d| d == name)
    }
}

fn normalize_exts<'a>(exts: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in exts {
        let e = e.trim().trim_start_matches('.').to_ascii_lowercase();
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_matches_extensions() {
        let cfg = GraphConfig {
            extensions: normalize_exts([".RB", "erb", "erb", "  slim "].into_iter()),
            ..Default::default()
        };
        assert_eq!(cfg.extensions, vec!["rb", "erb", "slim"]);
        assert!(cfg.matches("app/models/user.rb"));
        assert!(cfg.matches("app/views/x.html.erb"));
        assert!(!cfg.matches("app/models/user.py"));
    }

    #[test]
    fn excludes_dirs_and_globs() {
        let cfg = GraphConfig {
            extensions: vec!["rb".to_string()],
            exclude_globs: vec!["spec/".to_string()],
            ..Default::default()
        };
        assert!(!cfg.allows_dir("node_modules"));
        assert!(!cfg.allows_dir(".git"));
        assert!(cfg.allows_dir("app"));
        assert!(!cfg.matches("spec/models/user_spec.rb"));
        assert!(cfg.matches("app/models/user.rb"));
    }
}
