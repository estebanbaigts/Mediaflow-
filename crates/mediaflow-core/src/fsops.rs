//! Low-level file operations with the strongest guarantees each platform
//! offers: exclusive creation, durable writes, and *no-clobber* finalization.
//!
//! `std::fs::rename` silently replaces an existing target on every platform,
//! so it is never used to finalize a copy onto a name we do not own.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Create a new file, failing if anything exists at `path` (O_EXCL).
pub fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Flush directory metadata (entry creation/rename) to stable storage.
/// No-op where unsupported (Windows has no portable directory fsync).
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// How the last finalization was performed (recorded for diagnostics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalizeMethod {
    /// Kernel-level atomic rename that refuses to replace (renameat2 /
    /// renamex_np / MoveFileEx without REPLACE_EXISTING).
    AtomicNoReplace,
    /// Exclusive placeholder created with O_EXCL, then replaced by rename.
    /// Never replaces a file we did not create; not atomic against a
    /// concurrent writer that opens our placeholder.
    Placeholder,
}

/// Move `from` to `to` only if `to` does not exist. Returns
/// `ErrorKind::AlreadyExists` if the target is taken.
pub fn rename_no_replace(from: &Path, to: &Path) -> io::Result<FinalizeMethod> {
    match native_no_replace(from, to) {
        Ok(()) => Ok(FinalizeMethod::AtomicNoReplace),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(e) if is_unsupported(&e) => {
            placeholder_rename(from, to)?;
            Ok(FinalizeMethod::Placeholder)
        }
        Err(e) => Err(e),
    }
}

fn placeholder_rename(from: &Path, to: &Path) -> io::Result<()> {
    // Claim the name exclusively; fails with AlreadyExists if taken.
    drop(create_new(to)?);
    if let Err(e) = std::fs::rename(from, to) {
        // Release only the empty placeholder we created ourselves.
        if std::fs::metadata(to).map(|m| m.len() == 0).unwrap_or(false) {
            let _ = std::fs::remove_file(to);
        }
        return Err(e);
    }
    Ok(())
}

fn is_unsupported(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            e.raw_os_error(),
            Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::ENOTSUP)
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn native_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let f = CString::new(from.as_os_str().as_bytes())?;
    let t = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: valid NUL-terminated strings; AT_FDCWD with absolute/relative paths.
    let r = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            f.as_ptr(),
            libc::AT_FDCWD,
            t.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn native_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let f = CString::new(from.as_os_str().as_bytes())?;
    let t = CString::new(to.as_os_str().as_bytes())?;
    // SAFETY: valid NUL-terminated strings.
    let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn native_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
    let w = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (f, t) = (w(from), w(to));
    // SAFETY: valid NUL-terminated UTF-16 strings. Without
    // MOVEFILE_REPLACE_EXISTING the call fails if the target exists.
    let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if ok != 0 {
        Ok(())
    } else {
        let e = io::Error::last_os_error();
        // ERROR_ALREADY_EXISTS (183) / ERROR_FILE_EXISTS (80)
        if matches!(e.raw_os_error(), Some(183) | Some(80)) {
            Err(io::Error::new(io::ErrorKind::AlreadyExists, e))
        } else {
            Err(e)
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
fn native_no_replace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no native no-replace rename",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_replaces_existing() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"precious").unwrap();
        let e = rename_no_replace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&b).unwrap(), b"precious");
        assert!(a.exists());
    }

    #[test]
    fn renames_when_free() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"x").unwrap();
        rename_no_replace(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"x");
    }

    #[test]
    fn placeholder_fallback_never_replaces() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"precious").unwrap();
        assert_eq!(
            placeholder_rename(&a, &b).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&b).unwrap(), b"precious");
        std::fs::remove_file(&b).unwrap();
        placeholder_rename(&a, &b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), b"x");
    }
}
