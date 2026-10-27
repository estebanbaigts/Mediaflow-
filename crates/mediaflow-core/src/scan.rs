//! Source/destination scanning. Cheap information only (paths, sizes, dates,
//! type sniffing). Hashes are computed later, only when needed.
//!
//! Nothing is skipped silently: symlinks, special files and unreadable entries
//! are reported as [`ScanIssue`]s.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::{classify_file, Classification};
use crate::error::{MfError, Result};
use crate::paths::{self, is_link};
use crate::util::Control;

/// Stat-level identity of a file at a point in time. Used to detect files that
/// changed between scan and copy, and to invalidate cached hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch (0 if unavailable).
    pub mtime_ns: i64,
    /// Inode / file index when the platform exposes it.
    pub inode: Option<u64>,
}

impl FileStamp {
    pub fn of(md: &std::fs::Metadata) -> Self {
        let mtime_ns = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            Some(md.ino())
        };
        #[cfg(not(unix))]
        let inode = None;
        FileStamp {
            size: md.len(),
            mtime_ns,
            inode,
        }
    }

    pub fn read(path: &Path) -> Result<Self> {
        let md = std::fs::symlink_metadata(path).map_err(|e| MfError::io(path, e))?;
        if is_link(&md) {
            return Err(MfError::PathRejected {
                path: path.to_string_lossy().into_owned(),
                reason: "symbolic links are not followed",
            });
        }
        Ok(Self::of(&md))
    }

    pub fn modified(&self) -> Option<DateTime<Utc>> {
        if self.mtime_ns == 0 {
            None
        } else {
            DateTime::from_timestamp(
                self.mtime_ns.div_euclid(1_000_000_000),
                self.mtime_ns.rem_euclid(1_000_000_000) as u32,
            )
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannedFile {
    /// Path relative to the scanned root, `/`-separated.
    pub rel_path: String,
    pub size: u64,
    pub stamp: FileStamp,
    pub modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub class: Classification,
}

impl ScannedFile {
    pub fn file_name(&self) -> &str {
        self.rel_path.rsplit('/').next().unwrap_or(&self.rel_path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanIssueKind {
    SymlinkSkipped,
    SpecialFileSkipped,
    Unreadable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanIssue {
    pub rel_path: String,
    pub kind: ScanIssueKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub root: PathBuf,
    pub files: Vec<ScannedFile>,
    pub issues: Vec<ScanIssue>,
    pub total_bytes: u64,
    /// False if any directory could not be listed: the inventory is partial.
    pub complete: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Sniff file signatures (reads 512 bytes per file).
    pub classify: bool,
    /// Relative paths (prefix match, `/`-separated) to skip entirely. Used to
    /// keep MediaFlow's own bookkeeping out of destination indexes.
    pub skip_prefixes: Vec<String>,
}

/// Name prefix of MediaFlow temporary files. Scanners report them as issues
/// rather than treating them as media.
pub const TEMP_PREFIX: &str = ".mfpart-";

/// Walk `root` without following symlinks.
pub fn scan(
    root: &Path,
    opts: &ScanOptions,
    control: &Control,
    mut progress: impl FnMut(usize, u64),
) -> Result<ScanResult> {
    let root = paths::canonical_dir(root)?;
    let mut files = Vec::new();
    let mut issues = Vec::new();
    let mut total = 0u64;
    let mut complete = true;

    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name();
    for entry in walker {
        control.checkpoint()?;
        let entry = match entry {
            Ok(e) => e,
            Err(err) => {
                complete = false;
                let rel = err
                    .path()
                    .and_then(|p| p.strip_prefix(&root).ok())
                    .map(paths::rel_to_string)
                    .unwrap_or_default();
                // If the root itself vanished, the volume was disconnected.
                if !root.exists() {
                    return Err(MfError::VolumeUnavailable(root));
                }
                issues.push(ScanIssue {
                    rel_path: rel,
                    kind: ScanIssueKind::Unreadable,
                    message: err.to_string(),
                });
                continue;
            }
        };
        if entry.depth() == 0 {
            continue;
        }
        let rel_path = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        let rel = paths::rel_to_string(rel_path);
        if opts
            .skip_prefixes
            .iter()
            .any(|p| rel == *p || rel.starts_with(&format!("{p}/")))
        {
            continue;
        }
        let ft = entry.file_type();
        if ft.is_dir() {
            continue;
        }
        if ft.is_symlink() {
            issues.push(ScanIssue {
                rel_path: rel,
                kind: ScanIssueKind::SymlinkSkipped,
                message: "Symbolic link not followed".into(),
            });
            continue;
        }
        if !ft.is_file() {
            issues.push(ScanIssue {
                rel_path: rel,
                kind: ScanIssueKind::SpecialFileSkipped,
                message: "Not a regular file".into(),
            });
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            issues.push(ScanIssue {
                rel_path: rel,
                kind: ScanIssueKind::SpecialFileSkipped,
                message: "Incomplete MediaFlow temporary file".into(),
            });
            continue;
        }
        let md = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                issues.push(ScanIssue {
                    rel_path: rel,
                    kind: ScanIssueKind::Unreadable,
                    message: e.to_string(),
                });
                continue;
            }
        };
        if is_link(&md) {
            issues.push(ScanIssue {
                rel_path: rel,
                kind: ScanIssueKind::SymlinkSkipped,
                message: "Reparse point not followed".into(),
            });
            continue;
        }
        let stamp = FileStamp::of(&md);
        let class = if opts.classify {
            classify_file(entry.path())
        } else {
            crate::classify::classify_bytes(&rel, &[])
        };
        total += stamp.size;
        files.push(ScannedFile {
            rel_path: rel,
            size: stamp.size,
            modified: stamp.modified(),
            created: md.created().ok().map(DateTime::<Utc>::from),
            stamp,
            class,
        });
        progress(files.len(), total);
    }
    Ok(ScanResult {
        root,
        files,
        issues,
        total_bytes: total,
        complete,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_reports_symlinks() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("DCIM/100")).unwrap();
        std::fs::write(d.path().join("DCIM/100/A.JPG"), [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        std::fs::write(d.path().join("b.txt"), b"hi").unwrap();
        std::fs::write(d.path().join(".mfpart-x"), b"partial").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", d.path().join("evil")).unwrap();
        let r = scan(
            d.path(),
            &ScanOptions {
                classify: true,
                ..Default::default()
            },
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
        let names: Vec<_> = r.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(names, vec!["DCIM/100/A.JPG", "b.txt"]);
        assert_eq!(r.total_bytes, 6);
        assert!(r.complete);
        #[cfg(unix)]
        assert!(r
            .issues
            .iter()
            .any(|i| i.kind == ScanIssueKind::SymlinkSkipped && i.rel_path == "evil"));
        assert!(r.issues.iter().any(|i| i.rel_path == ".mfpart-x"));
        assert_eq!(
            r.files[0].class.confidence,
            crate::classify::TypeConfidence::Confirmed
        );
    }

    #[test]
    fn skip_prefixes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join(".mediaflow")).unwrap();
        std::fs::write(d.path().join(".mediaflow/x"), b"1").unwrap();
        std::fs::write(d.path().join("y"), b"1").unwrap();
        let r = scan(
            d.path(),
            &ScanOptions {
                classify: false,
                skip_prefixes: vec![".mediaflow".into()],
            },
            &Control::new(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(r.files.len(), 1);
    }
}
