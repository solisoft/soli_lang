use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub trait VirtualFileSystem: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    fn exists(&self, path: &str) -> bool;
    fn read_to_string(&self, path: &str) -> Result<String, String> {
        self.read(path)
            .and_then(|data| String::from_utf8(data).map_err(|e| format!("UTF-8 error: {}", e)))
    }
    fn walk_dir(&self, dir: &str) -> Result<Vec<String>, String>;
    fn is_dir(&self, path: &str) -> bool;
}

pub struct DiskFS {
    root: PathBuf,
}

impl DiskFS {
    pub fn new(root: &str) -> Self {
        DiskFS {
            root: PathBuf::from(root),
        }
    }

    /// Map a VFS path to a real filesystem path.
    ///
    /// Uses `Path` operations rather than string concatenation throughout.
    /// String handling looks equivalent and is not: it hardcodes `/`, so on
    /// Windows — where the root is `C:\...` — an "absolute" test against `/`
    /// fails, the under-root check never matches, and the root gets doubled.
    /// `Path::is_absolute` and `Path::starts_with` are separator-correct on
    /// every platform, and `starts_with` compares whole components, so a
    /// sibling directory like `/tmp/soli_1-evil` cannot masquerade as being
    /// under `/tmp/soli_1`.
    fn resolve(&self, path: &str) -> PathBuf {
        if path.is_empty() || path == "." || path == "/" {
            return self.root.clone();
        }

        let candidate = Path::new(path);
        if candidate.is_absolute() {
            // An absolute path already inside the root must be used verbatim:
            // serve-mode callers (the template engine, the static-file
            // handler) build absolute paths by joining the serve folder —
            // which IS this root — so re-prefixing would double it
            // (`/tmp/soli_X/tmp/soli_X/app/views/...`).
            if candidate == self.root || candidate.starts_with(&self.root) {
                return candidate.to_path_buf();
            }
            // Absolute but outside the root: graft it under the root, which is
            // what the string version did by concatenation.
            let relative = candidate
                .strip_prefix(std::path::Component::RootDir.as_os_str())
                .unwrap_or(candidate);
            return self.root.join(relative);
        }

        self.root.join(candidate)
    }
}

impl VirtualFileSystem for DiskFS {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let full = self.resolve(path);
        std::fs::read(&full).map_err(|e| format!("Failed to read '{}': {}", full.display(), e))
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve(path).exists()
    }

    fn read_to_string(&self, path: &str) -> Result<String, String> {
        let full = self.resolve(path);
        std::fs::read_to_string(&full)
            .map_err(|e| format!("Failed to read '{}': {}", full.display(), e))
    }

    fn walk_dir(&self, dir: &str) -> Result<Vec<String>, String> {
        let full = self.resolve(dir);
        if !full.is_dir() {
            return Err(format!("Not a directory: {}", full.display()));
        }
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&full)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            let entry = entry.map_err(|e| format!("Walk error: {}", e))?;
            if entry.file_type().is_file() {
                // Strip by path components rather than string prefix: the
                // string form hardcodes `/`, so on Windows nothing ever
                // matched and every entry was returned as an absolute path
                // pretending to be relative.
                let relative = entry
                    .path()
                    .strip_prefix(&self.root)
                    .unwrap_or(entry.path());
                // VFS keys stay `/`-separated on every platform so they match
                // the keys `BundleFS` builds from bundle entries.
                files.push(to_vfs_key(relative));
            }
        }
        files.sort();
        Ok(files)
    }

    fn is_dir(&self, path: &str) -> bool {
        self.resolve(path).is_dir()
    }
}

/// Render a relative path as a platform-neutral VFS key.
fn to_vfs_key(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

const MAGIC: &[u8; 4] = b"SOLB";

pub struct BundleFS {
    entries: HashMap<String, Vec<u8>>,
}

impl BundleFS {
    pub fn new(data: Vec<u8>) -> Result<Self, String> {
        let mut pos = 0usize;

        if crate::bundle::is_encrypted_bundle(&data) {
            return Err(crate::bundle::ENCRYPTED_BUNDLE_HINT.to_string());
        }
        if data.len() < 8 {
            return Err("Bundle too short".to_string());
        }

        if &data[0..4] != MAGIC {
            return Err("Invalid bundle magic".to_string());
        }
        pos += 4;

        let entry_count = u32::from_le_bytes(
            data[pos..pos + 4]
                .try_into()
                .map_err(|_| "Invalid entry count")?,
        );
        pos += 4;

        let mut entries = HashMap::new();

        for _ in 0..entry_count {
            if pos + 4 > data.len() {
                return Err("Truncated bundle: path length".to_string());
            }
            let path_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;

            if pos.checked_add(path_len).is_none_or(|end| end > data.len()) {
                return Err("Truncated bundle: path".to_string());
            }
            let path_bytes = &data[pos..pos + path_len];
            let path_str =
                String::from_utf8(path_bytes.to_vec()).map_err(|_| "Invalid UTF-8 path")?;
            crate::bundle::validate_entry_path(&path_str)?;
            pos += path_len;

            if pos + 8 > data.len() {
                return Err("Truncated bundle: content length".to_string());
            }
            let content_len = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
            pos += 8;

            if pos
                .checked_add(content_len)
                .is_none_or(|end| end > data.len())
            {
                return Err("Truncated bundle: content".to_string());
            }
            let content = data[pos..pos + content_len].to_vec();
            pos += content_len;

            entries.insert(path_str, content);
        }

        Ok(BundleFS { entries })
    }

    pub fn into_entries(self) -> HashMap<String, Vec<u8>> {
        self.entries
    }

    pub fn entries(&self) -> &HashMap<String, Vec<u8>> {
        &self.entries
    }
}

impl VirtualFileSystem for BundleFS {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        // Normalize path: remove leading slash if present
        let normalized = path.trim_start_matches('/');
        self.entries
            .get(normalized)
            .cloned()
            .ok_or_else(|| format!("File not found in bundle: {}", path))
    }

    fn exists(&self, path: &str) -> bool {
        let normalized = path.trim_start_matches('/');
        if normalized.is_empty() || self.entries.contains_key(normalized) {
            return true;
        }
        let prefix = format!("{}/", normalized);
        self.entries.keys().any(|k| k.starts_with(&prefix))
    }

    fn read_to_string(&self, path: &str) -> Result<String, String> {
        let data = self.read(path)?;
        String::from_utf8(data).map_err(|e| format!("UTF-8 error in '{}': {}", path, e))
    }

    fn walk_dir(&self, dir: &str) -> Result<Vec<String>, String> {
        let prefix = dir.trim_start_matches('/');
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", prefix)
        };

        let mut files: Vec<String> = self
            .entries
            .keys()
            .filter(|k| k.starts_with(&prefix) && k.len() > prefix.len())
            .map(|k| {
                // Return the full relative path
                if prefix.is_empty() {
                    k.clone()
                } else {
                    k[prefix.len()..].to_string()
                }
            })
            .collect();
        files.sort();
        Ok(files)
    }

    fn is_dir(&self, path: &str) -> bool {
        let normalized = path.trim_start_matches('/');
        if normalized.is_empty() {
            return true;
        }
        let prefix = format!("{}/", normalized);
        self.entries.keys().any(|k| k.starts_with(&prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_round_trip() {
        // Create a bundle manually
        let mut entries = HashMap::new();
        entries.insert(
            "app/controllers/home_controller.sl".to_string(),
            b"class HomeController {}".to_vec(),
        );
        entries.insert("app/models/user.sl".to_string(), b"let x = 1;".to_vec());
        entries.insert(
            "config/routes.sl".to_string(),
            b"get('/') { \"hello\" }".to_vec(),
        );

        // Serialize
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        let count = entries.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());

        let mut sorted: Vec<(&String, &Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (path, content) in &sorted {
            let path_bytes = path.as_bytes();
            buf.extend_from_slice(&(path_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(path_bytes);
            buf.extend_from_slice(&(content.len() as u64).to_le_bytes());
            buf.extend_from_slice(content);
        }

        // Parse back
        let fs = BundleFS::new(buf).unwrap();

        assert!(fs.exists("app/controllers/home_controller.sl"));
        assert!(fs.exists("config/routes.sl"));
        assert!(!fs.exists("nonexistent"));

        assert_eq!(
            fs.read_to_string("app/models/user.sl").unwrap(),
            "let x = 1;"
        );

        // Test walk_dir
        let app_files = fs.walk_dir("app").unwrap();
        assert!(app_files.contains(&"controllers/home_controller.sl".to_string()));
        assert!(app_files.contains(&"models/user.sl".to_string()));

        // Test is_dir
        assert!(fs.is_dir("app"));
        assert!(fs.is_dir("app/controllers"));
        assert!(!fs.is_dir("nonexistent"));
    }

    #[test]
    fn test_bundle_invalid_magic() {
        let data = vec![0, 0, 0, 0];
        assert!(BundleFS::new(data).is_err());
    }

    #[test]
    fn test_bundle_fs_rejects_encrypted_magic() {
        let err = match BundleFS::new(b"SOLE\x01whatever-follows".to_vec()) {
            Ok(_) => panic!("expected an error on encrypted magic"),
            Err(e) => e,
        };
        assert!(err.contains("encrypted"), "got: {err}");
        assert!(err.contains("SOLI_BUNDLE_KEY"), "got: {err}");
    }

    #[test]
    fn test_disk_fs_exists() {
        let fs = DiskFS::new("/tmp");
        assert!(fs.exists("."));
    }

    #[test]
    fn test_disk_fs_walk_dir_returns_relative_slash_separated_keys() {
        // walk_dir's results are used as VFS keys and must match the keys
        // BundleFS builds from bundle entries, which are always relative and
        // '/'-separated. Returning absolute paths here (what the old string
        // prefix-strip did whenever the separator did not match) makes every
        // subsequent lookup miss.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(dir.path().join("app/views")).unwrap();
        std::fs::write(dir.path().join("app/views/a.erb"), b"a").unwrap();
        std::fs::write(dir.path().join("app/views/b.erb"), b"b").unwrap();

        let fs = DiskFS::new(&root);
        let found = fs.walk_dir("app/views").unwrap();
        assert_eq!(found, vec!["app/views/a.erb", "app/views/b.erb"]);
        for key in &found {
            assert!(!key.starts_with('/'), "key must be relative: {}", key);
            // Every key must round-trip back through the same VFS.
            assert!(fs.exists(key), "walk_dir key must resolve: {}", key);
        }
    }

    #[test]
    fn test_disk_fs_under_root_check_respects_component_boundaries() {
        // `/root-evil/x` shares `/root` as a *string* prefix but is not under
        // it. Path::starts_with compares whole components, so it is correctly
        // treated as outside and grafted under the root instead of being
        // served verbatim from a sibling directory.
        let fs = DiskFS::new("/srv/app");
        assert_eq!(
            fs.resolve("/srv/app/views/x.erb"),
            Path::new("/srv/app/views/x.erb")
        );
        assert_eq!(
            fs.resolve("/srv/app-evil/x.erb"),
            Path::new("/srv/app/srv/app-evil/x.erb")
        );
        assert_eq!(
            fs.resolve("app/views/x.erb"),
            Path::new("/srv/app/app/views/x.erb")
        );
        assert_eq!(fs.resolve("/"), Path::new("/srv/app"));
        assert_eq!(fs.resolve("."), Path::new("/srv/app"));
    }

    #[test]
    fn test_disk_fs_absolute_path_under_root_not_reprefixed() {
        // Regression: serve-mode callers (template engine, static files)
        // pass absolute paths built from the serve folder — the DiskFS
        // root itself. Re-prefixing doubled the root and every template
        // lookup 404'd when serving a .soli bundle.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        let views = dir.path().join("app/views/home");
        std::fs::create_dir_all(&views).unwrap();
        std::fs::write(views.join("index.html.erb"), b"<h1>hi</h1>").unwrap();

        let fs = DiskFS::new(&root);
        let absolute = format!("{}/app/views/home/index.html.erb", root);
        assert!(fs.exists(&absolute), "absolute path under root must hit");
        assert_eq!(fs.read_to_string(&absolute).unwrap(), "<h1>hi</h1>");
        assert!(fs.exists(&root), "the root itself must hit");

        // A sibling path that merely shares the root as a string prefix
        // (`/tmp/xyzABC-other`) is NOT under the root: it must still be
        // treated as VFS-relative and get prefixed (and thus miss).
        let sibling = format!("{}-other/app/views/home/index.html.erb", root);
        assert!(!fs.exists(&sibling));

        // VFS-relative lookups keep working, with and without a leading /.
        assert!(fs.exists("app/views/home/index.html.erb"));
        assert!(fs.exists("/app/views/home/index.html.erb"));
    }
}
