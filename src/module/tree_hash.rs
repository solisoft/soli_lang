//! Content hash of an extracted package tree (supply-chain R7).
//!
//! GitHub/GitLab archive tarballs are not byte-stable — the host may
//! recompress them at any time — so the lock file cannot pin a digest of the
//! downloaded archive. It pins a digest of what the archive *extracts to*
//! instead: every regular file under the package directory, identified by its
//! `/`-separated relative path and executable bit, with its bytes.
//!
//! The digest is independent of the order in which the filesystem lists
//! directory entries: files are collected first and sorted by relative path.
//! Each field is length-prefixed so that no two different trees can produce
//! the same byte stream fed to SHA-256.
//!
//! Symlinks and anything that is neither a regular file nor a directory are
//! refused: the extractor (`tar_extract`) never creates them, so their
//! presence means the cached tree was altered after extraction.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Prefix of every digest this module produces, e.g. `sha256-3f7a…`.
pub const HASH_PREFIX: &str = "sha256-";

/// Domain-separation tag, versioned so the scheme can change later without
/// a new digest ever being mistaken for an old one.
const DOMAIN_TAG: &[u8] = b"soli-package-tree-v1\0";

/// One regular file found under the tree root.
struct TreeFile {
    /// Path relative to the root, components joined with `/`.
    rel_path: String,
    executable: bool,
    abs_path: std::path::PathBuf,
}

/// Compute the content digest of the tree rooted at `root`.
///
/// Returns `sha256-<64 lowercase hex chars>`.
pub fn hash_tree(root: &Path) -> Result<String, String> {
    let mut files: Vec<TreeFile> = Vec::new();
    collect_files(root, "", &mut files)?;
    files.sort_by(|a, b| a.rel_path.as_bytes().cmp(b.rel_path.as_bytes()));

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_TAG);
    hasher.update((files.len() as u64).to_le_bytes());
    for file in &files {
        let contents = fs::read(&file.abs_path).map_err(|e| {
            format!(
                "Failed to read '{}' while hashing package: {}",
                file.abs_path.display(),
                e
            )
        })?;
        let path_bytes = file.rel_path.as_bytes();
        hasher.update((path_bytes.len() as u64).to_le_bytes());
        hasher.update(path_bytes);
        hasher.update([u8::from(file.executable)]);
        hasher.update((contents.len() as u64).to_le_bytes());
        hasher.update(contents);
    }

    let digest = hasher.finalize();
    let mut out = String::with_capacity(HASH_PREFIX.len() + digest.len() * 2);
    out.push_str(HASH_PREFIX);
    for byte in digest.iter() {
        out.push_str(&format!("{:02x}", byte));
    }
    Ok(out)
}

fn collect_files(dir: &Path, rel_prefix: &str, out: &mut Vec<TreeFile>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| {
        format!(
            "Failed to read directory '{}' while hashing package: {}",
            dir.display(),
            e
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            format!(
                "Failed to read directory entry in '{}' while hashing package: {}",
                dir.display(),
                e
            )
        })?;
        let abs_path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel_path = if rel_prefix.is_empty() {
            name
        } else {
            format!("{}/{}", rel_prefix, name)
        };
        // `symlink_metadata` does not follow links, so a symlink is seen as one.
        let meta = fs::symlink_metadata(&abs_path).map_err(|e| {
            format!(
                "Failed to stat '{}' while hashing package: {}",
                abs_path.display(),
                e
            )
        })?;
        let file_type = meta.file_type();
        if file_type.is_dir() {
            collect_files(&abs_path, &rel_path, out)?;
        } else if file_type.is_file() {
            out.push(TreeFile {
                rel_path,
                executable: is_executable(&meta),
                abs_path,
            });
        } else {
            return Err(format!(
                "Refusing to hash package tree: '{}' is not a regular file or directory \
                 (package extraction never creates links or special files)",
                abs_path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, contents: &[u8]) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn hash_has_expected_shape() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.sl", b"x");
        let hash = hash_tree(dir.path()).unwrap();
        assert!(hash.starts_with(HASH_PREFIX));
        assert_eq!(hash.len(), HASH_PREFIX.len() + 64);
        assert!(hash[HASH_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hash_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "soli.toml", b"[package]\nname = \"m\"\n");
        write(dir.path(), "src/lib.sl", b"def f { 1 }\n");
        write(dir.path(), "src/nested/deep.sl", b"def g { 2 }\n");
        let first = hash_tree(dir.path()).unwrap();
        let second = hash_tree(dir.path()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn hash_is_independent_of_creation_order() {
        // Directory iteration order generally follows creation order on
        // many filesystems; build the same tree in two different orders.
        let first = tempfile::tempdir().unwrap();
        write(first.path(), "b.sl", b"bee");
        write(first.path(), "a/z.sl", b"zed");
        write(first.path(), "a/y.sl", b"why");
        write(first.path(), "c.sl", b"sea");

        let second = tempfile::tempdir().unwrap();
        write(second.path(), "c.sl", b"sea");
        write(second.path(), "a/y.sl", b"why");
        write(second.path(), "b.sl", b"bee");
        write(second.path(), "a/z.sl", b"zed");

        assert_eq!(
            hash_tree(first.path()).unwrap(),
            hash_tree(second.path()).unwrap()
        );
    }

    #[test]
    fn hash_changes_when_a_byte_changes() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.sl", b"def f { 1 }\n");
        let before = hash_tree(dir.path()).unwrap();
        write(dir.path(), "src/lib.sl", b"def f { 2 }\n");
        let after = hash_tree(dir.path()).unwrap();
        assert_ne!(before, after);
    }

    #[test]
    fn hash_changes_when_a_file_is_renamed_or_added() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.sl", b"same");
        let original = hash_tree(dir.path()).unwrap();

        fs::rename(dir.path().join("a.sl"), dir.path().join("b.sl")).unwrap();
        let renamed = hash_tree(dir.path()).unwrap();
        assert_ne!(original, renamed);

        write(dir.path(), "extra.sl", b"");
        let added = hash_tree(dir.path()).unwrap();
        assert_ne!(renamed, added);
    }

    #[test]
    fn path_and_content_boundaries_are_unambiguous() {
        // "ab" + "c" must not hash like "a" + "bc".
        let first = tempfile::tempdir().unwrap();
        write(first.path(), "ab", b"c");
        let second = tempfile::tempdir().unwrap();
        write(second.path(), "a", b"bc");
        assert_ne!(
            hash_tree(first.path()).unwrap(),
            hash_tree(second.path()).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn hash_covers_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "bin/tool", b"#!/bin/sh\n");
        let path = dir.path().join("bin/tool");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let plain = hash_tree(dir.path()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let executable = hash_tree(dir.path()).unwrap();
        assert_ne!(plain, executable);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_tree_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.sl", b"x");
        std::os::unix::fs::symlink(dir.path().join("a.sl"), dir.path().join("link.sl")).unwrap();
        let err = hash_tree(dir.path()).unwrap_err();
        assert!(err.contains("not a regular file"), "{}", err);
    }
}
