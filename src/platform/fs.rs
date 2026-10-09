//! The file access app loading needs. Natively it is `std::fs`; once a tree is
//! mounted — the edge build has no filesystem, so the host hands the app's files
//! over — every call answers from that in-memory tree instead. Mounting also
//! installs the tree as the global VFS, which templates already read through.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// An in-memory tree of files, keyed by absolute path. Directories exist by
/// virtue of holding a file.
#[derive(Default)]
pub struct MemFs {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
}

impl MemFs {
    pub fn new(files: Vec<(String, Vec<u8>)>) -> Self {
        let mut fs = MemFs::default();
        for (path, bytes) in files {
            let path = PathBuf::from(path);
            let mut dir = path.parent();
            while let Some(d) = dir {
                fs.dirs.insert(d.to_path_buf());
                dir = d.parent();
            }
            fs.files.insert(path, bytes);
        }
        fs
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files.get(path).cloned().ok_or_else(|| not_found(path))
    }

    fn is_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path)
    }

    fn read_dir(&self, dir: &Path) -> io::Result<Vec<DirEntry>> {
        if !self.is_dir(dir) {
            return Err(not_found(dir));
        }
        let child = |p: &&PathBuf| p.parent() == Some(dir);
        let dirs = self.dirs.iter().filter(child).map(|p| (p, true));
        let files = self.files.keys().filter(child).map(|p| (p, false));
        Ok(dirs
            .chain(files)
            .map(|(path, dir)| DirEntry {
                path: path.clone(),
                dir,
            })
            .collect())
    }
}

static MOUNTED: OnceLock<Arc<MemFs>> = OnceLock::new();

/// Mount `files` (absolute paths) as the filesystem, for the rest of the
/// process. First call wins.
pub fn mount(files: Vec<(String, Vec<u8>)>) {
    let fs = Arc::new(MemFs::new(files));
    if MOUNTED.set(fs.clone()).is_ok() {
        crate::serve::init_global_vfs(Shared(fs));
    }
}

fn mounted() -> Option<&'static MemFs> {
    MOUNTED.get().map(|fs| fs.as_ref())
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{}: not in the mounted tree", path.display()),
    )
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    let path = path.as_ref();
    match mounted() {
        Some(fs) => fs.read(path),
        None => std::fs::read(path),
    }
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn exists(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match mounted() {
        Some(fs) => fs.is_file(path) || fs.is_dir(path),
        None => path.exists(),
    }
}

pub fn is_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match mounted() {
        Some(fs) => fs.is_dir(path),
        None => path.is_dir(),
    }
}

pub fn is_file(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    match mounted() {
        Some(fs) => fs.is_file(path),
        None => path.is_file(),
    }
}

#[derive(Clone, Copy)]
pub struct FileType {
    dir: bool,
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.dir
    }
    pub fn is_file(&self) -> bool {
        !self.dir
    }
    pub fn is_symlink(&self) -> bool {
        false
    }
}

/// What [`read_dir`] yields: the parts of `std::fs::DirEntry` app loading uses.
pub struct DirEntry {
    path: PathBuf,
    dir: bool,
}

impl DirEntry {
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }
    pub fn file_name(&self) -> OsString {
        self.path.file_name().unwrap_or_default().to_os_string()
    }
    pub fn file_type(&self) -> io::Result<FileType> {
        Ok(FileType { dir: self.dir })
    }
}

/// Immediate children of `dir`, like `std::fs::read_dir` (and in no particular
/// order either: callers sort).
pub fn read_dir(dir: impl AsRef<Path>) -> io::Result<std::vec::IntoIter<io::Result<DirEntry>>> {
    let dir = dir.as_ref();
    let entries: Vec<io::Result<DirEntry>> = match mounted() {
        Some(fs) => fs.read_dir(dir)?.into_iter().map(Ok).collect(),
        None => std::fs::read_dir(dir)?
            .map(|entry| {
                let entry = entry?;
                Ok(DirEntry {
                    path: entry.path(),
                    dir: entry.file_type()?.is_dir(),
                })
            })
            .collect(),
    };
    Ok(entries.into_iter())
}

/// The mounted tree as the global VFS (`vfs_read`, `vfs_exists`…), which takes
/// paths as strings.
struct Shared(Arc<MemFs>);

impl crate::virtual_fs::VirtualFileSystem for Shared {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.0.read(Path::new(path)).map_err(|e| e.to_string())
    }
    fn exists(&self, path: &str) -> bool {
        let path = Path::new(path);
        self.0.is_file(path) || self.0.is_dir(path)
    }
    fn walk_dir(&self, dir: &str) -> Result<Vec<String>, String> {
        let dir = Path::new(dir);
        Ok(self
            .0
            .files
            .keys()
            .filter(|p| p.starts_with(dir))
            .map(|p| p.to_string_lossy().into_owned())
            .collect())
    }
    fn is_dir(&self, path: &str) -> bool {
        self.0.is_dir(Path::new(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> MemFs {
        MemFs::new(vec![
            ("/app/config/routes.sl".into(), b"get(\"/\")".to_vec()),
            (
                "/app/app/controllers/home.sl".into(),
                b"class Home".to_vec(),
            ),
            (
                "/app/app/views/home/index.html.slv".into(),
                b"<h1>".to_vec(),
            ),
        ])
    }

    #[test]
    fn files_and_the_directories_holding_them() {
        let fs = tree();
        assert_eq!(
            fs.read(Path::new("/app/config/routes.sl")).unwrap(),
            b"get(\"/\")"
        );
        assert!(fs.is_file(Path::new("/app/app/controllers/home.sl")));
        for dir in ["/", "/app", "/app/app", "/app/app/views/home"] {
            assert!(fs.is_dir(Path::new(dir)), "{dir}");
        }
        assert!(!fs.is_dir(Path::new("/app/config/routes.sl")));
        let err = fs.read(Path::new("/app/missing.sl")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn read_dir_lists_immediate_children_only() {
        let fs = tree();
        let mut names: Vec<(String, bool)> = fs
            .read_dir(Path::new("/app/app"))
            .unwrap()
            .into_iter()
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                (name, e.file_type().unwrap().is_dir())
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("controllers".to_string(), true),
                ("views".to_string(), true)
            ]
        );
        assert!(fs.read_dir(Path::new("/app/nope")).is_err());
    }

    #[test]
    fn unmounted_it_is_the_real_filesystem() {
        // Nothing in the library's unit tests mounts a tree.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        assert!(is_file(&manifest));
        assert!(read_to_string(&manifest).unwrap().contains("[package]"));
        assert!(read_dir(env!("CARGO_MANIFEST_DIR")).unwrap().count() > 0);
    }
}
