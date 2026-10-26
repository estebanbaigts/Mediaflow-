//! Path safety: validation of user/template supplied relative paths,
//! confinement inside authorized roots, and symlink-aware resolution.
//!
//! Rules enforced here are the engine's security boundary. They are applied
//! again at execution time, not only while planning.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::error::{MfError, Result};

/// Maximum length of one path segment (bytes). Most file systems cap at 255.
pub const MAX_SEGMENT: usize = 255;

const WINDOWS_RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn reject(path: &str, reason: &'static str) -> MfError {
    MfError::PathRejected {
        path: path.to_string(),
        reason,
    }
}

/// Validate a single path segment destined to be created on disk. We apply the
/// strictest common denominator (Windows + macOS + Linux) so a layout planned on
/// one OS stays valid on another drive.
pub fn validate_segment(seg: &str) -> Result<()> {
    if seg.is_empty() {
        return Err(reject(seg, "empty name"));
    }
    if seg == "." || seg == ".." {
        return Err(reject(seg, "relative navigation is not allowed"));
    }
    if seg.len() > MAX_SEGMENT {
        return Err(reject(seg, "name is too long"));
    }
    if seg.chars().any(|c| {
        c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
    }) {
        return Err(reject(seg, "name contains a forbidden character"));
    }
    if seg.ends_with(' ') || seg.ends_with('.') {
        return Err(reject(seg, "name ends with a space or a dot"));
    }
    let stem = seg.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        return Err(reject(seg, "name is reserved on Windows"));
    }
    Ok(())
}

/// Turn arbitrary text (camera model, project name…) into a safe segment by
/// replacing forbidden characters. Never returns an empty string.
pub fn sanitize_segment(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() || s == "." || s == ".." {
        s = "_".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        s.insert(0, '_');
    }
    while s.len() > MAX_SEGMENT {
        s.pop();
    }
    s
}

/// Validate a relative path made of `/`-separated segments. Rejects absolute
/// paths, drive prefixes, `..`, empty segments and forbidden names.
pub fn validate_relative(rel: &str) -> Result<PathBuf> {
    if rel.is_empty() {
        return Err(reject(rel, "empty path"));
    }
    if rel.contains('\0') {
        return Err(reject(rel, "contains a NUL byte"));
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(reject(rel, "absolute paths are not allowed"));
    }
    let mut out = PathBuf::new();
    for seg in rel.split(['/', '\\']) {
        validate_segment(seg)?;
        out.push(seg);
    }
    // Defence in depth: re-check with the platform parser.
    for c in out.components() {
        if !matches!(c, Component::Normal(_)) {
            return Err(reject(rel, "unexpected path component"));
        }
    }
    Ok(out)
}

/// Lexically check a relative path read from disk (scanner output). Unlike
/// [`validate_relative`] it accepts any name the OS accepted, but still refuses
/// navigation components.
pub fn check_scanned_relative(rel: &Path) -> Result<()> {
    for c in rel.components() {
        match c {
            Component::Normal(_) => {}
            _ => return Err(reject(&rel.to_string_lossy(), "unexpected path component")),
        }
    }
    Ok(())
}

/// Canonicalize an existing directory that the user explicitly authorized.
pub fn canonical_dir(p: &Path) -> Result<PathBuf> {
    let c = dunce_canonicalize(p)?;
    let md = std::fs::metadata(&c).map_err(|e| MfError::io(&c, e))?;
    if !md.is_dir() {
        return Err(reject(&p.to_string_lossy(), "not a folder"));
    }
    Ok(c)
}

/// `std::fs::canonicalize`, but on Windows strip the `\\?\` prefix when it is
/// safe, so paths stay readable and comparable.
pub fn dunce_canonicalize(p: &Path) -> Result<PathBuf> {
    let c = std::fs::canonicalize(p).map_err(|e| MfError::io(p, e))?;
    #[cfg(windows)]
    {
        let s = c.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            if !rest.starts_with("UNC\\") && rest.len() < 260 {
                return Ok(PathBuf::from(rest));
            }
        }
    }
    Ok(c)
}

/// True if `child` (canonical) is `root` (canonical) or lies under it.
pub fn is_within(root: &Path, child: &Path) -> bool {
    child.starts_with(root)
}

/// Join `rel` under `root` and make sure no existing component of the result
/// is a symlink (or Windows reparse point) and that it stays under `root`.
/// Components that do not exist yet are allowed; they will be created by
/// [`create_dirs_within`].
pub fn resolve_within(root: &Path, rel: &Path) -> Result<PathBuf> {
    check_scanned_relative(rel)?;
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        if let Component::Normal(seg) = c {
            cur.push(seg);
            match std::fs::symlink_metadata(&cur) {
                Ok(md) if is_link(&md) => {
                    return Err(reject(
                        &cur.to_string_lossy(),
                        "symbolic links are not followed",
                    ))
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(MfError::io(&cur, e)),
            }
        }
    }
    if !is_within(root, &cur) {
        return Err(MfError::NotAuthorized(cur));
    }
    Ok(cur)
}

/// Create the parent directories of `target` (which must be under `root`),
/// one level at a time, refusing to traverse symlinks created concurrently.
pub fn create_dirs_within(root: &Path, target_dir: &Path) -> Result<()> {
    let rel = target_dir
        .strip_prefix(root)
        .map_err(|_| MfError::NotAuthorized(target_dir.to_path_buf()))?;
    let mut cur = root.to_path_buf();
    for c in rel.components() {
        let Component::Normal(seg) = c else {
            return Err(reject(&rel.to_string_lossy(), "unexpected path component"));
        };
        cur.push(seg);
        match std::fs::create_dir(&cur) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(MfError::io(&cur, e)),
        }
        let md = std::fs::symlink_metadata(&cur).map_err(|e| MfError::io(&cur, e))?;
        if is_link(&md) || !md.is_dir() {
            return Err(reject(&cur.to_string_lossy(), "expected a real folder"));
        }
    }
    Ok(())
}

pub fn is_link(md: &std::fs::Metadata) -> bool {
    if md.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Portable display form of a relative path (always `/`).
pub fn rel_to_string(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn file_name_lossy(p: &Path) -> String {
    p.file_name()
        .map(OsStr::to_string_lossy)
        .unwrap_or_default()
        .into_owned()
}

/// Comparison key used to detect collisions on case-insensitive file systems
/// (APFS/NTFS/exFAT default) and Unicode normalization differences (macOS
/// historically stores NFD). We lowercase and strip combining marks: two
/// planned names with the same key are treated as colliding, which is the
/// conservative choice.
pub fn collision_key(rel: &str) -> String {
    rel.chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !('\u{0300}'..='\u{036F}').contains(c))
        .map(fold_accent)
        .collect()
}

fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_absolute() {
        for bad in [
            "../etc/passwd",
            "a/../../b",
            "/abs",
            "\\abs",
            "a//b",
            "a/./b",
            "C:/x",
            "a\0b",
            "",
            "con/x",
            "a/NUL.txt",
            "trailing./x",
            "a/b ",
        ] {
            assert!(
                validate_relative(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn accepts_unicode() {
        let p = validate_relative("Projet été/2024-05-01/Caméra A/clip 001.MOV").unwrap();
        assert_eq!(p.components().count(), 4);
        validate_relative("日本語/ファイル.mp4").unwrap();
    }

    #[test]
    fn sanitize_never_produces_invalid() {
        for raw in [
            "",
            "..",
            "a/b",
            "Canon: EOS R5?",
            "CON",
            "  x. ",
            "\u{7}bell",
            &"x".repeat(400),
        ] {
            let s = sanitize_segment(raw);
            validate_segment(&s).unwrap_or_else(|e| panic!("{raw:?} -> {s:?}: {e}"));
        }
    }

    #[test]
    fn collision_key_folds_case_and_accents() {
        assert_eq!(collision_key("Clip.MOV"), collision_key("clip.mov"));
        assert_eq!(collision_key("été"), collision_key("e\u{301}te\u{301}"));
        assert_eq!(collision_key("Été"), collision_key("ete"));
    }

    #[test]
    fn resolve_within_blocks_symlinks() {
        let d = tempfile::tempdir().unwrap();
        let root = canonical_dir(d.path()).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        assert!(resolve_within(&root, Path::new("real/new/file")).is_ok());
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
            let e = resolve_within(&root, Path::new("link/file")).unwrap_err();
            assert_eq!(e.code(), "path_rejected");
            let e = create_dirs_within(&root, &root.join("link").join("sub")).unwrap_err();
            assert_eq!(e.code(), "path_rejected");
        }
        assert!(resolve_within(&root, Path::new("../x")).is_err());
    }

    #[test]
    fn create_dirs_refuses_outside_root() {
        let d = tempfile::tempdir().unwrap();
        let root = canonical_dir(d.path()).unwrap();
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            create_dirs_within(&root, other.path()).unwrap_err().code(),
            "not_authorized"
        );
    }
}
