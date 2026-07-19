//! Creating directories only their owner can read.
//!
//! Used for the extraction directory that decrypted application source is
//! written to. Getting this wrong is a confidentiality failure, not a
//! convenience one: on a shared machine the app's source would be readable by
//! every other local user.

use std::path::Path;

/// Create `dir` (and parents) restricted to the current user.
///
/// The permissions are applied *at creation*, so there is no window in which
/// the directory exists with default access.
pub fn create_private_dir(dir: &Path) -> Result<(), String> {
    create_private_dir_impl(dir)
}

#[cfg(unix)]
fn create_private_dir_impl(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("cannot create {}: {}", dir.display(), e))
}

#[cfg(windows)]
fn create_private_dir_impl(dir: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    // Parents first: CreateDirectoryW is not recursive. They get default
    // permissions, which is fine — only the leaf holds decrypted content.
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {}", parent.display(), e))?;
    }
    if dir.is_dir() {
        return Ok(());
    }

    // D:PAI(A;OICI;FA;;;OW) — a protected DACL granting full access to the
    // owner alone. `P` is the important letter: it blocks inherited ACEs, which
    // is what makes this equivalent to 0700 rather than "0700 plus whatever the
    // parent grants". OICI propagates it to anything created inside.
    let sddl: Vec<u16> = "D:PAI(A;OICI;FA;;;OW)"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut descriptor: *mut core::ffi::c_void = std::ptr::null_mut();
    // SAFETY: `sddl` is a NUL-terminated UTF-16 string; the descriptor is
    // written to `descriptor` and freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!(
            "cannot build security descriptor: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let wide: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both pointers are NUL-terminated and live for the call.
    let created = unsafe { CreateDirectoryW(wide.as_ptr(), &mut attributes) };
    // Freed on both paths — the descriptor is not referenced after the call.
    unsafe { LocalFree(descriptor) };

    if created == 0 {
        return Err(format!(
            "cannot create {}: {}",
            dir.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn create_private_dir_impl(dir: &Path) -> Result<(), String> {
    Err(format!(
        "cannot create a private directory on this platform ({})",
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_the_directory() {
        let base = tempfile::tempdir().expect("scratch");
        let dir = base.path().join("private");
        create_private_dir(&dir).expect("create");
        assert!(dir.is_dir());
    }

    #[test]
    #[cfg(unix)]
    fn restricts_access_to_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().expect("scratch");
        let dir = base.path().join("private");
        create_private_dir(&dir).expect("create");
        let mode = std::fs::metadata(&dir).expect("stat").permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o700,
            "decrypted source must not be readable by other local users"
        );
    }
}
