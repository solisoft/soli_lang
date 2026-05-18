use std::collections::HashMap;
use std::path::Path;

const BUNDLE_MAGIC: &[u8; 4] = b"SOLB";

const BUNDLE_EXTENSIONS: &[&str] = &[
    "sl", "slv", "yml", "yaml", "css", "js", "md", "erb", "toml", "json", "env",
];

const BUNDLE_SPECIAL_FILES: &[&str] = &["soli.toml", ".solivrc"];

pub struct BundleBuilder;

impl BundleBuilder {
    pub fn build(source_dir: &Path) -> Result<Vec<u8>, String> {
        let mut entries: HashMap<String, Vec<u8>> = HashMap::new();
        let source_dir = source_dir.canonicalize().map_err(|e| {
            format!("Failed to resolve source directory '{}': {}", source_dir.display(), e)
        })?;

        Self::collect_entries(&source_dir, &source_dir, &mut entries)?;

        // Also look for special files at root
        for special in BUNDLE_SPECIAL_FILES {
            let special_path = source_dir.join(special);
            if special_path.is_file() {
                let data = std::fs::read(&special_path)
                    .map_err(|e| format!("Failed to read '{}': {}", special_path.display(), e))?;
                entries.insert(special.to_string(), data);
            }
        }

        Self::serialize(&entries)
    }

    fn collect_entries(
        source_dir: &Path,
        current_dir: &Path,
        entries: &mut HashMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let read_dir = std::fs::read_dir(current_dir)
            .map_err(|e| format!("Failed to read directory '{}': {}", current_dir.display(), e))?;

        for entry in read_dir {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Skip hidden files and common non-app directories
            if name_str.starts_with('.') || name_str == "node_modules" || name_str == "target" {
                continue;
            }

            if path.is_dir() {
                Self::collect_entries(source_dir, &path, entries)?;
            } else if path.is_file() {
                // Check if the extension is one we want
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                let is_special = BUNDLE_SPECIAL_FILES.iter().any(|s| *s == name_str.as_ref());
                let is_bundled_ext = BUNDLE_EXTENSIONS.contains(&ext);

                if is_special || is_bundled_ext {
                    let relative = path
                        .strip_prefix(source_dir)
                        .map_err(|e| format!("Failed to compute relative path: {}", e))?;
                    let relative_str = relative.to_string_lossy().to_string();

                    let data = std::fs::read(&path)
                        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;
                    entries.insert(relative_str, data);
                }
            }
        }

        Ok(())
    }

    fn serialize(entries: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();

        // Magic
        buf.extend_from_slice(BUNDLE_MAGIC);

        // Entry count
        let count = entries.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());

        // Sort entries by path for deterministic output
        let mut sorted: Vec<(&String, &Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));

        for (path, content) in sorted {
            let path_bytes = path.as_bytes();
            let path_len = path_bytes.len() as u32;
            buf.extend_from_slice(&path_len.to_le_bytes());
            buf.extend_from_slice(path_bytes);

            let content_len = content.len() as u64;
            buf.extend_from_slice(&content_len.to_le_bytes());
            buf.extend_from_slice(content);
        }

        Ok(buf)
    }
}

/// Read and iterate entries from a serialized bundle.
pub struct BundleReader<'a> {
    _data: &'a [u8],
    entries: Vec<(String, &'a [u8])>,
}

impl<'a> BundleReader<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, String> {
        if data.len() < 8 || &data[..4] != BUNDLE_MAGIC {
            return Err("Invalid bundle: bad magic".to_string());
        }

        let count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
        let mut offset = 8;
        let mut entries = Vec::with_capacity(count);

        for _ in 0..count {
            if offset + 4 > data.len() {
                return Err("Invalid bundle: truncated path length".to_string());
            }
            let path_len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;

            if offset + path_len > data.len() {
                return Err("Invalid bundle: truncated path".to_string());
            }
            let path = std::str::from_utf8(&data[offset..offset + path_len])
                .map_err(|_| "Invalid bundle: non-UTF-8 path".to_string())?;
            offset += path_len;

            if offset + 8 > data.len() {
                return Err("Invalid bundle: truncated content length".to_string());
            }
            let content_len = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]) as usize;
            offset += 8;

            if offset + content_len > data.len() {
                return Err("Invalid bundle: truncated content".to_string());
            }
            entries.push((path.to_string(), &data[offset..offset + content_len]));
            offset += content_len;
        }

        Ok(BundleReader { _data: data, entries })
    }

    pub fn entries(&self) -> &[(String, &'a [u8])] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_fs::VirtualFileSystem;
    use std::fs;

    #[test]
    fn test_build_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let app_controllers = dir.path().join("app/controllers");
        fs::create_dir_all(&app_controllers).unwrap();
        fs::write(app_controllers.join("home_controller.sl"), b"class HomeController {}").unwrap();

        let config = dir.path().join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(config.join("routes.sl"), b"get('/') { \"hello\" }").unwrap();

        let bundle = BundleBuilder::build(dir.path()).unwrap();

        // Parse and verify
        let vfs = crate::virtual_fs::BundleFS::new(bundle).unwrap();
        assert!(vfs.exists("app/controllers/home_controller.sl"));
        assert!(vfs.exists("config/routes.sl"));
        assert_eq!(
            vfs.read_to_string("app/controllers/home_controller.sl").unwrap(),
            "class HomeController {}"
        );
    }

    #[test]
    fn test_build_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden.sl"), b"secret").unwrap();
        fs::write(dir.path().join("visible.sl"), b"ok").unwrap();

        let bundle = BundleBuilder::build(dir.path()).unwrap();
        let vfs = crate::virtual_fs::BundleFS::new(bundle).unwrap();
        assert!(!vfs.exists(".hidden.sl"));
        assert!(vfs.exists("visible.sl"));
    }
}
