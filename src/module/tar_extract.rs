//! Hardened tar extraction for package archives.
//!
//! Used by both `module::registry` (flat package tarballs) and
//! `module::installer` (GitHub/GitLab archives wrapped in a top-level
//! `repo-sha/` directory). SEC-075 / SEC-075a: a malicious or compromised
//! archive must not be able to write outside `dest` via `..`, absolute
//! roots, prefix components, or symlink/hardlink entries.

use std::fs;
use std::io;
use std::path::{Component, Path};

/// Extract a tarball into `dest`. When `strip_top_level` is true the
/// first path component of every entry is dropped before the entry is
/// joined onto `dest` (GitHub/GitLab archives nest contents under
/// `repo-sha/`); otherwise paths are extracted as-is.
///
/// Refuses any entry whose retained components are not all `Component::Normal`
/// (i.e. no `..`, no absolute roots, no prefixes), and refuses symlink and
/// hardlink entry types outright.
pub(super) fn extract_archive<R: io::Read>(
    archive: &mut tar::Archive<R>,
    dest: &Path,
    strip_top_level: bool,
) -> Result<(), String> {
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive entries: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to read entry path: {}", e))?
            .into_owned();

        let components: Vec<Component> = path.components().collect();
        let retained: &[Component] = if strip_top_level {
            if components.len() <= 1 {
                continue; // top-level dir itself, nothing to extract
            }
            &components[1..]
        } else {
            if components.is_empty() {
                continue;
            }
            &components[..]
        };

        validate_components(retained, &path)?;

        let entry_type = entry.header().entry_type();
        // Symlinks and hardlinks can point outside `dest` regardless of how
        // safe the entry path itself looks. Refuse them — package archives
        // do not need to ship link entries.
        if entry_type == tar::EntryType::Symlink || entry_type == tar::EntryType::Link {
            return Err(format!(
                "Refusing to extract link entry from package tarball: {}",
                path.display()
            ));
        }

        let mut out_path = dest.to_path_buf();
        for component in retained {
            out_path.push(component);
        }

        if entry_type == tar::EntryType::Directory {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                // Defence in depth: even after rejecting `..`/root components
                // above, re-canonicalise and confirm the resolved parent is
                // still under `dest`. The only way to reach this branch with
                // an unsafe parent is via a pre-existing symlink in the
                // ancestor chain — also worth refusing.
                if let (Ok(canon_parent), Ok(canon_root)) =
                    (fs::canonicalize(parent), fs::canonicalize(dest))
                {
                    if !canon_parent.starts_with(&canon_root) {
                        return Err(format!(
                            "Refusing to extract package entry that resolves outside the cache directory: {}",
                            path.display()
                        ));
                    }
                }
            }
            let mut out_file =
                fs::File::create(&out_path).map_err(|e| format!("Failed to create file: {}", e))?;
            io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }

    Ok(())
}

/// Reject path components that are anything other than `Normal(_)`.
/// `original` is the full entry path, used for the error message.
fn validate_components(components: &[Component], original: &Path) -> Result<(), String> {
    for component in components {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!(
                "Refusing to extract package entry with unsafe path: {}",
                original.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Append a regular file entry whose name is written directly into the
    /// header buffer, bypassing `tar::Header::set_path`'s built-in rejection
    /// of `..` and absolute roots — we *want* to ship malicious entries here
    /// so we can prove the extractor refuses them on its own.
    fn append_raw_entry(builder: &mut tar::Builder<Vec<u8>>, name: &[u8], body: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        let buf = &mut header.as_old_mut().name;
        for slot in buf.iter_mut() {
            *slot = 0;
        }
        let n = name.len().min(buf.len());
        buf[..n].copy_from_slice(&name[..n]);
        header.set_cksum();
        builder.append(&header, body).unwrap();
    }

    fn build_safe_tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, body) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, path, &body[..]).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn validator_accepts_normal_relative_paths() {
        let path = Path::new("src/lib.sl");
        let comps: Vec<_> = path.components().collect();
        assert!(validate_components(&comps, path).is_ok());
    }

    #[test]
    fn validator_rejects_parent_dir_segment() {
        let path = Path::new("../.bashrc");
        let comps: Vec<_> = path.components().collect();
        let err = validate_components(&comps, path).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn validator_rejects_buried_parent_dir_segment() {
        let path = Path::new("a/../../pwned");
        let comps: Vec<_> = path.components().collect();
        let err = validate_components(&comps, path).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn validator_rejects_absolute_path() {
        let path = Path::new("/tmp/pwned");
        let comps: Vec<_> = path.components().collect();
        let err = validate_components(&comps, path).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    // ---- no-strip mode (registry tarballs) ----

    #[test]
    fn extract_no_strip_accepts_safe_nested_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = build_safe_tarball(&[("pkg/lib.sl", b"hello")]);
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        extract_archive(&mut archive, tmp.path(), false).unwrap();
        let extracted = std::fs::read(tmp.path().join("pkg/lib.sl")).unwrap();
        assert_eq!(extracted, b"hello");
    }

    #[test]
    fn extract_no_strip_rejects_parent_traversal_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), false).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
        assert!(!tmp.path().parent().unwrap().join("pwned").exists());
    }

    #[test]
    fn extract_no_strip_rejects_buried_traversal_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"a/../../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), false).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn extract_no_strip_rejects_absolute_path_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        append_raw_entry(&mut builder, b"/tmp/pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), false).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn extract_rejects_symlink_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("/etc/passwd").expect("symlink target");
        header.set_cksum();
        builder.append_data(&mut header, "passwd", &[][..]).unwrap();
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), false).unwrap_err();
        assert!(err.contains("link entry"), "{}", err);
    }

    // ---- strip-top-level mode (GitHub/GitLab archives) ----

    #[test]
    fn extract_strip_accepts_safe_nested_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = build_safe_tarball(&[("repo-sha/", b""), ("repo-sha/src/lib.sl", b"hello")]);
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        extract_archive(&mut archive, tmp.path(), true).unwrap();
        let extracted = std::fs::read(tmp.path().join("src/lib.sl")).unwrap();
        assert_eq!(extracted, b"hello");
        // The bare top-level dir should not have been recreated under dest.
        assert!(!tmp.path().join("repo-sha").exists());
    }

    #[test]
    fn extract_strip_rejects_parent_traversal_after_strip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        // After stripping `repo-sha/`, the retained component is `..`.
        append_raw_entry(&mut builder, b"repo-sha/../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), true).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
        assert!(!tmp.path().parent().unwrap().join("pwned").exists());
    }

    #[test]
    fn extract_strip_rejects_buried_traversal_after_strip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        // After strip, retained components are `a/../../pwned`.
        append_raw_entry(&mut builder, b"repo-sha/a/../../pwned", b"escape");
        let bytes = builder.into_inner().unwrap();
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        let err = extract_archive(&mut archive, tmp.path(), true).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    // Note: strip mode cannot meaningfully test absolute-root rejection.
    // A `/foo/bar` entry parses as `[RootDir, Normal("foo"), Normal("bar")]`,
    // and `&components[1..]` (the strip) drops the `RootDir`, leaving a safe
    // `[Normal, Normal]` slice. POSIX collapses runs of `/` so an embedded
    // root mid-path is impossible. The realistic attack surface for strip
    // mode is `..` after strip — covered by the two preceding tests.
}
