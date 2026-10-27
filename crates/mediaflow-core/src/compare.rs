//! Completeness check: does each destination hold every file of a source?
//!
//! Matching is by content (anywhere in the destination), because imported
//! files are usually reorganized. A destination is declared complete only in
//! `Verified` mode, when every source file has a SHA-256-identical copy and
//! both sides were fully readable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::{CachedHasher, Engine};
use crate::error::Result;
use crate::hash::Sha256Hex;
use crate::paths::collision_key;
use crate::plan::HashProvider;
use crate::scan::{self, ScanOptions, ScanResult, ScannedFile};
use crate::util::Control;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareMode {
    /// Name + size only. Fast, never proves anything.
    Quick,
    /// SHA-256 of every candidate. Slow, conclusive.
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Presence {
    PresentVerified {
        rel_path: String,
    },
    PresentUnverified {
        rel_path: String,
    },
    /// A file with the same name exists but its content differs.
    Modified {
        rel_path: String,
    },
    /// MediaFlow previously verified this copy, and it no longer matches.
    Corrupted {
        rel_path: String,
    },
    /// Destination fully read; no copy found.
    Missing,
    /// Destination could not be fully read; absence is not established.
    Undetermined,
    /// Destination volume not connected.
    DestinationUnavailable,
    SourceUnreadable {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareEntry {
    pub source_rel: String,
    pub size: u64,
    pub per_destination: Vec<Presence>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DestinationVerdict {
    pub root: String,
    pub available: bool,
    pub index_complete: bool,
    pub present_verified: usize,
    pub present_unverified: usize,
    pub modified: usize,
    pub corrupted: usize,
    pub missing: usize,
    pub undetermined: usize,
    /// Files in the destination that match no source file.
    pub other_files: usize,
    /// True only when the check proves every source file is present.
    pub complete: bool,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReport {
    pub id: String,
    pub at: DateTime<Utc>,
    pub mode: CompareMode,
    pub source_root: String,
    pub source_complete: bool,
    pub source_files: usize,
    pub entries: Vec<CompareEntry>,
    pub destinations: Vec<DestinationVerdict>,
    /// Source files present in every destination (verified in Verified mode).
    pub everywhere: usize,
    /// Source files left out of the check by the chosen exclusion rules.
    pub excluded: Vec<String>,
    pub previous_check_at: Option<DateTime<Utc>>,
}

impl Engine {
    /// `exclusions`: source files matching these rules are listed in
    /// `excluded` and not required at destinations (visible, never silent).
    #[allow(clippy::too_many_arguments)]
    pub fn compare(
        &self,
        source: &Path,
        destinations: &[PathBuf],
        mode: CompareMode,
        exclusions: &[crate::profile::ExclusionRule],
        control: &Control,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<CompareReport> {
        let src_root = self.check_authorized(source, false)?;
        let previous_check_at = self
            .db()
            .last_check(&src_root.to_string_lossy())?
            .map(|(t, _)| t);
        let mut src = scan::scan(&src_root, &ScanOptions::default(), control, |_, _| {})?;
        let matchers = exclusions
            .iter()
            .map(|r| {
                globset::GlobBuilder::new(&r.pattern)
                    .case_insensitive(true)
                    .build()
                    .map(|g| g.compile_matcher())
                    .map_err(|e| {
                        crate::MfError::InvalidInput(format!(
                            "invalid pattern {:?}: {e}",
                            r.pattern
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let (excluded, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut src.files)
            .into_iter()
            .partition(|f| matchers.iter().any(|m| m.is_match(&f.rel_path)));
        src.files = kept;
        let excluded: Vec<String> = excluded.into_iter().map(|f| f.rel_path).collect();

        let mut dest_scans: Vec<Option<ScanResult>> = Vec::new();
        for d in destinations {
            if !d.is_dir() {
                dest_scans.push(None);
                continue;
            }
            let root = self.check_authorized(d, false)?;
            dest_scans.push(Some(scan::scan(
                &root,
                &ScanOptions::default(),
                control,
                |_, _| {},
            )?));
        }

        let mut hasher = CachedHasher { engine: self };
        let mut dest_hash: Vec<HashMap<Sha256Hex, String>> =
            vec![HashMap::new(); destinations.len()];
        let mut hashed_sizes: Vec<std::collections::HashSet<u64>> =
            vec![Default::default(); destinations.len()];
        let by_size: Vec<HashMap<u64, Vec<&ScannedFile>>> = dest_scans
            .iter()
            .map(|s| {
                let mut m: HashMap<u64, Vec<&ScannedFile>> = HashMap::new();
                if let Some(s) = s {
                    for f in &s.files {
                        m.entry(f.size).or_default().push(f);
                    }
                }
                m
            })
            .collect();
        let by_name: Vec<HashMap<String, Vec<&ScannedFile>>> = dest_scans
            .iter()
            .map(|s| {
                let mut m: HashMap<String, Vec<&ScannedFile>> = HashMap::new();
                if let Some(s) = s {
                    for f in &s.files {
                        m.entry(collision_key(f.file_name())).or_default().push(f);
                    }
                }
                m
            })
            .collect();
        let mut matched: Vec<std::collections::HashSet<String>> =
            vec![Default::default(); destinations.len()];

        let mut entries = Vec::new();
        for (n, f) in src.files.iter().enumerate() {
            control.checkpoint()?;
            progress(n, src.files.len());
            let mut per = Vec::new();
            let src_hash = if mode == CompareMode::Verified {
                match hasher.hash(&src.root.join(&f.rel_path), &f.stamp, control) {
                    Ok(h) => Some(h),
                    Err(e @ (crate::MfError::Cancelled | crate::MfError::Paused)) => return Err(e),
                    Err(e) => {
                        entries.push(CompareEntry {
                            source_rel: f.rel_path.clone(),
                            size: f.size,
                            per_destination: vec![
                                Presence::SourceUnreadable {
                                    message: e.user_message()
                                };
                                destinations.len()
                            ],
                        });
                        continue;
                    }
                }
            } else {
                None
            };
            for (di, ds) in dest_scans.iter().enumerate() {
                let Some(ds) = ds else {
                    per.push(Presence::DestinationUnavailable);
                    continue;
                };
                let key = collision_key(f.file_name());
                let presence = match &src_hash {
                    None => {
                        // Quick mode: name + size.
                        match by_name[di]
                            .get(&key)
                            .and_then(|v| v.iter().find(|c| c.size == f.size))
                        {
                            Some(c) => {
                                matched[di].insert(c.rel_path.clone());
                                Presence::PresentUnverified {
                                    rel_path: c.rel_path.clone(),
                                }
                            }
                            None if by_name[di].contains_key(&key) => Presence::Modified {
                                rel_path: by_name[di][&key][0].rel_path.clone(),
                            },
                            None if ds.complete => Presence::Missing,
                            None => Presence::Undetermined,
                        }
                    }
                    Some(sh) => {
                        // Hash each destination size bucket once; then O(1) lookups.
                        if hashed_sizes[di].insert(f.size) {
                            for c in by_size[di].get(&f.size).into_iter().flatten() {
                                if let Ok(h) =
                                    hasher.hash(&ds.root.join(&c.rel_path), &c.stamp, control)
                                {
                                    dest_hash[di].entry(h).or_insert_with(|| c.rel_path.clone());
                                }
                            }
                        }
                        let found = dest_hash[di].get(sh).cloned();
                        match found {
                            Some(rel) => {
                                matched[di].insert(rel.clone());
                                Presence::PresentVerified { rel_path: rel }
                            }
                            None => {
                                // A copy MediaFlow verified earlier that no longer matches?
                                let lib = self.db().library(f.file_name(), 50).unwrap_or_default();
                                let root_s = ds.root.to_string_lossy();
                                let corrupted = lib.iter().find(|l| {
                                    l.dest_root == root_s
                                        && l.sha256 == sh.as_str()
                                        && l.source_rel == f.rel_path
                                });
                                if let Some(l) = corrupted {
                                    Presence::Corrupted {
                                        rel_path: l.rel_path.clone(),
                                    }
                                } else if let Some(v) = by_name[di].get(&key) {
                                    Presence::Modified {
                                        rel_path: v[0].rel_path.clone(),
                                    }
                                } else if ds.complete {
                                    Presence::Missing
                                } else {
                                    Presence::Undetermined
                                }
                            }
                        }
                    }
                };
                per.push(presence);
            }
            entries.push(CompareEntry {
                source_rel: f.rel_path.clone(),
                size: f.size,
                per_destination: per,
            });
        }
        progress(src.files.len(), src.files.len());

        let mut verdicts = Vec::new();
        for (di, ds) in dest_scans.iter().enumerate() {
            let mut v = DestinationVerdict {
                root: destinations[di].to_string_lossy().into_owned(),
                available: ds.is_some(),
                index_complete: ds.as_ref().is_some_and(|d| d.complete),
                ..Default::default()
            };
            for e in &entries {
                match &e.per_destination[di] {
                    Presence::PresentVerified { .. } => v.present_verified += 1,
                    Presence::PresentUnverified { .. } => v.present_unverified += 1,
                    Presence::Modified { .. } => v.modified += 1,
                    Presence::Corrupted { .. } => v.corrupted += 1,
                    Presence::Missing => v.missing += 1,
                    _ => v.undetermined += 1,
                }
            }
            v.other_files = ds
                .as_ref()
                .map(|d| d.files.len().saturating_sub(matched[di].len()))
                .unwrap_or(0);
            v.complete = mode == CompareMode::Verified
                && v.available
                && v.index_complete
                && src.complete
                && v.present_verified == entries.len();
            v.explanation = if !v.available {
                "Destination not connected: nothing can be concluded.".into()
            } else if v.complete {
                "Every source file has a verified identical copy here.".into()
            } else if mode == CompareMode::Quick {
                format!("Quick check (names and sizes only): {} of {} found. Run a verified check to prove completeness.", v.present_unverified, entries.len())
            } else if !src.complete || !v.index_complete {
                "Some folders could not be read: completeness cannot be established.".into()
            } else {
                format!(
                    "{} file(s) missing, {} modified, {} corrupted.",
                    v.missing, v.modified, v.corrupted
                )
            };
            verdicts.push(v);
        }
        let everywhere = entries
            .iter()
            .filter(|e| {
                e.per_destination.iter().all(|p| match mode {
                    CompareMode::Verified => matches!(p, Presence::PresentVerified { .. }),
                    CompareMode::Quick => matches!(p, Presence::PresentUnverified { .. }),
                })
            })
            .count();
        let report = CompareReport {
            id: uuid::Uuid::new_v4().to_string(),
            at: crate::util::now(),
            mode,
            source_root: src.root.to_string_lossy().into_owned(),
            source_complete: src.complete,
            source_files: entries.len(),
            entries,
            destinations: verdicts,
            everywhere,
            excluded,
            previous_check_at,
        };
        let summary = serde_json::json!({ "mode": mode, "destinations": report.destinations, "everywhere": everywhere, "source_files": report.source_files });
        self.db()
            .save_check(&report.id, &report.source_root, &summary.to_string())?;
        Ok(report)
    }
}
