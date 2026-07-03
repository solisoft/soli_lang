use std::collections::HashMap;
use std::path::Path;

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
    root: String,
}

impl DiskFS {
    pub fn new(root: &str) -> Self {
        DiskFS {
            root: root.to_string(),
        }
    }

    fn resolve(&self, path: &str) -> String {
        if path.is_empty() || path == "." || path == "/" {
            return self.root.clone();
        }
        if path.starts_with('/') {
            // An absolute path already inside the root must be used
            // verbatim: serve-mode callers (the template engine, the
            // static-file handler) build absolute paths by joining the
            // serve folder — which IS this root — so re-prefixing would
            // double it (`/tmp/soli_X/tmp/soli_X/app/views/...`).
            let root = self.root.trim_end_matches('/');
            if path == root
                || path
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                return path.to_string();
            }
            return format!("{}{}", self.root, path);
        }
        format!("{}/{}", self.root, path)
    }
}

impl VirtualFileSystem for DiskFS {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let full = self.resolve(path);
        std::fs::read(&full).map_err(|e| format!("Failed to read '{}': {}", full, e))
    }

    fn exists(&self, path: &str) -> bool {
        let full = self.resolve(path);
        Path::new(&full).exists()
    }

    fn read_to_string(&self, path: &str) -> Result<String, String> {
        let full = self.resolve(path);
        std::fs::read_to_string(&full).map_err(|e| format!("Failed to read '{}': {}", full, e))
    }

    fn walk_dir(&self, dir: &str) -> Result<Vec<String>, String> {
        let full = self.resolve(dir);
        let path = Path::new(&full);
        if !path.is_dir() {
            return Err(format!("Not a directory: {}", full));
        }
        let mut files = Vec::new();
        let prefix = if self.root.ends_with('/') {
            self.root.clone()
        } else {
            format!("{}/", self.root)
        };
        for entry in walkdir::WalkDir::new(path)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
        {
            let entry = entry.map_err(|e| format!("Walk error: {}", e))?;
            if entry.file_type().is_file() {
                let full_path = entry.path().to_string_lossy().to_string();
                let rel = full_path
                    .strip_prefix(&prefix)
                    .unwrap_or(&full_path)
                    .to_string();
                files.push(rel);
            }
        }
        files.sort();
        Ok(files)
    }

    fn is_dir(&self, path: &str) -> bool {
        let full = self.resolve(path);
        Path::new(&full).is_dir()
    }
}

const MAGIC: &[u8; 4] = b"SOLB";

pub struct BundleFS {
    entries: HashMap<String, Vec<u8>>,
}

impl BundleFS {
    pub fn new(data: Vec<u8>) -> Result<Self, String> {
        let mut pos = 0usize;

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

            if pos + path_len > data.len() {
                return Err("Truncated bundle: path".to_string());
            }
            let path_bytes = &data[pos..pos + path_len];
            let path_str =
                String::from_utf8(path_bytes.to_vec()).map_err(|_| "Invalid UTF-8 path")?;
            pos += path_len;

            if pos + 8 > data.len() {
                return Err("Truncated bundle: content length".to_string());
            }
            let content_len = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()) as usize;
            pos += 8;

            if pos + content_len > data.len() {
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
    fn test_disk_fs_exists() {
        let fs = DiskFS::new("/tmp");
        assert!(fs.exists("."));
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
