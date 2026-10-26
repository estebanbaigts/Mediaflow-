//! Import planning: decides, before anything is written, what will be copied,
//! skipped (verified duplicate), flagged (conflict) or excluded, and how much
//! space each destination needs. The plan is a proposal: the executor
//! re-validates everything at execution time.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::classify::{Category, TypeConfidence};
use crate::error::Result;
use crate::hash::Sha256Hex;
use crate::matching::{compare, Facts, MatchLevel};
use crate::paths::collision_key;
use crate::profile::{ConflictPolicy, DuplicatePolicy, Profile};
use crate::scan::{FileStamp, ScanIssue, ScanResult, ScannedFile};
use crate::template::TemplateContext;
use crate::util::Control;

/// Provides (possibly cached) SHA-256 digests. Implementations must
/// invalidate their cache when the file stamp changes.
pub trait HashProvider {
    fn hash(&mut self, abs: &Path, stamp: &FileStamp, control: &Control) -> Result<Sha256Hex>;
}

/// Hash provider without cache.
pub struct DirectHasher;

impl HashProvider for DirectHasher {
    fn hash(&mut self, abs: &Path, _stamp: &FileStamp, control: &Control) -> Result<Sha256Hex> {
        crate::hash::hash_file(abs, control, |_| {}).map(|(h, _)| h)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanAction {
    /// Copy to `target_rel` (which is free at planning time).
    Copy,
    /// Identical content already exists at `existing_rel`. Skipped only after
    /// the executor re-verifies both digests; otherwise falls back to a copy.
    SkipDuplicate {
        existing_rel: String,
        level: MatchLevel,
    },
    /// A file with the same name but different content occupies the target and
    /// the profile says to flag rather than rename. Not copied.
    Conflict {
        existing_rel: String,
        level: MatchLevel,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanNote {
    /// Target name changed because the original name was taken by different content.
    RenamedExistingDifferent { original_rel: String },
    /// Target name changed because two source files mapped to the same path.
    RenamedPlanCollision { original_rel: String },
    /// A file of the same size exists elsewhere in the destination; content not verified.
    PossibleDuplicate {
        existing_rel: String,
        level: MatchLevel,
    },
    /// The same content appears more than once in the source.
    DuplicateInSource { other_rel: String },
    /// Signature contradicts the extension.
    TypeMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTarget {
    pub dest_index: usize,
    pub target_rel: String,
    pub action: PlanAction,
    pub notes: Vec<PlanNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exclusion {
    pub pattern: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub source: ScannedFile,
    pub category: Category,
    /// Digest computed during planning (only for duplicate candidates).
    pub source_sha256: Option<Sha256Hex>,
    pub excluded: Option<Exclusion>,
    pub targets: Vec<PlanTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanDestination {
    pub root: PathBuf,
    pub label: String,
    pub required_bytes: u64,
    pub available_bytes: Option<u64>,
    pub space_ok: bool,
    pub existing_files: usize,
    /// False if the destination index could not be fully read.
    pub index_complete: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlanSummary {
    pub files: usize,
    pub bytes: u64,
    pub to_copy: usize,
    pub bytes_to_copy: u64,
    pub skip_duplicates: usize,
    pub conflicts: usize,
    pub excluded: usize,
    pub renamed: usize,
    pub possible_duplicates: usize,
    pub type_mismatches: usize,
    pub scan_issues: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPlan {
    pub created_at: DateTime<Utc>,
    pub source_root: PathBuf,
    pub source_label: String,
    pub project: String,
    pub profile: Profile,
    pub destinations: Vec<PlanDestination>,
    pub items: Vec<PlanItem>,
    pub scan_issues: Vec<ScanIssue>,
    pub source_scan_complete: bool,
    pub summary: PlanSummary,
}

pub struct PlanInput<'a> {
    pub source: &'a ScanResult,
    pub source_label: &'a str,
    pub project: &'a str,
    pub profile: &'a Profile,
    /// Destination roots (canonical) with their current index (scan).
    pub destinations: Vec<(&'a ScanResult, String)>,
    /// Hash size-colliding candidates while planning so duplicates are proven.
    pub hash_candidates: bool,
}

/// Name with ` (n)` inserted before the extension.
pub fn suffixed(rel: &str, n: u32) -> String {
    let (dir, name) = match rel.rsplit_once('/') {
        Some((d, f)) => (Some(d), f),
        None => (None, rel),
    };
    let new = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    };
    match dir {
        Some(d) => format!("{d}/{new}"),
        None => new,
    }
}

struct DestIndex<'a> {
    by_key: HashMap<String, &'a ScannedFile>,
    by_size: HashMap<u64, Vec<&'a ScannedFile>>,
    /// Collision keys reserved by this plan.
    reserved: HashSet<String>,
    hashes: HashMap<String, Option<Sha256Hex>>,
}

pub fn build_plan(
    input: PlanInput<'_>,
    hasher: &mut dyn HashProvider,
    control: &Control,
    mut progress: impl FnMut(usize, usize),
) -> Result<ImportPlan> {
    let compiled = input.profile.compile()?;
    let src = input.source;

    let mut indexes: Vec<DestIndex> = input
        .destinations
        .iter()
        .map(|(scan, _)| {
            let mut by_size: HashMap<u64, Vec<&ScannedFile>> = HashMap::new();
            for f in &scan.files {
                by_size.entry(f.size).or_default().push(f);
            }
            DestIndex {
                by_key: scan
                    .files
                    .iter()
                    .map(|f| (collision_key(&f.rel_path), f))
                    .collect(),
                by_size,
                reserved: HashSet::new(),
                hashes: HashMap::new(),
            }
        })
        .collect();

    // Source sizes that also appear in some destination: only those are hashed.
    let candidate_sizes: HashSet<u64> = if input.hash_candidates {
        indexes
            .iter()
            .flat_map(|i| i.by_size.keys().copied())
            .collect()
    } else {
        HashSet::new()
    };
    // Sizes appearing more than once in the source (possible internal duplicates).
    let mut src_size_count: HashMap<u64, usize> = HashMap::new();
    for f in &src.files {
        *src_size_count.entry(f.size).or_default() += 1;
    }

    let mut src_hash_owner: HashMap<Sha256Hex, String> = HashMap::new();
    let mut items = Vec::with_capacity(src.files.len());
    let mut summary = PlanSummary {
        scan_issues: src.issues.len(),
        ..Default::default()
    };
    let mut required = vec![0u64; indexes.len()];

    for (i, f) in src.files.iter().enumerate() {
        control.checkpoint()?;
        progress(i, src.files.len());
        summary.files += 1;
        summary.bytes += f.size;

        if let Some((_, rule)) = compiled
            .exclusions
            .iter()
            .find(|(g, _)| g.is_match(&f.rel_path))
        {
            summary.excluded += 1;
            items.push(PlanItem {
                source: f.clone(),
                category: f.class.category,
                source_sha256: None,
                excluded: Some(Exclusion {
                    pattern: rule.pattern.clone(),
                    reason: rule.reason.clone(),
                }),
                targets: vec![],
            });
            continue;
        }

        let category = compiled
            .overrides
            .iter()
            .find(|(g, _)| g.is_match(&f.rel_path))
            .map(|(_, c)| *c)
            .unwrap_or(f.class.category);

        let ctx = TemplateContext {
            project: input.project,
            source_label: input.source_label,
            camera: None,
            date: f.modified.or(f.created),
            category,
            rel_path: &f.rel_path,
        };
        let base_rel = input.profile.template.render(&ctx)?;

        let needs_hash = candidate_sizes.contains(&f.size)
            || (input.hash_candidates && src_size_count.get(&f.size).copied().unwrap_or(0) > 1);
        let source_sha = if needs_hash {
            hasher
                .hash(&src.root.join(&f.rel_path), &f.stamp, control)
                .ok()
        } else {
            None
        };

        let mut common_notes = vec![];
        if f.class.confidence == TypeConfidence::Mismatch {
            common_notes.push(PlanNote::TypeMismatch);
            summary.type_mismatches += 1;
        }
        if let Some(h) = &source_sha {
            if let Some(other) = src_hash_owner.get(h) {
                common_notes.push(PlanNote::DuplicateInSource {
                    other_rel: other.clone(),
                });
            } else {
                src_hash_owner.insert(h.clone(), f.rel_path.clone());
            }
        }

        let mut targets = Vec::new();
        for (di, idx) in indexes.iter_mut().enumerate() {
            let dest_root = &input.destinations[di].0.root;
            let mut notes = common_notes.clone();
            let mut target_rel = base_rel.clone();
            let mut action = PlanAction::Copy;

            // Exact path occupied in the destination?
            if let Some(existing) = idx.by_key.get(&collision_key(&target_rel)).copied() {
                let ex_hash = if source_sha.is_some() {
                    cached_hash(idx, hasher, dest_root, existing, control)
                } else {
                    None
                };
                let level = compare(
                    &Facts {
                        name: f.file_name(),
                        size: Some(f.size),
                        mtime_ns: Some(f.stamp.mtime_ns),
                        sha256: source_sha.as_ref(),
                    },
                    &Facts {
                        name: existing.file_name(),
                        size: Some(existing.size),
                        mtime_ns: Some(existing.stamp.mtime_ns),
                        sha256: ex_hash.as_ref(),
                    },
                );
                let identical_candidate = matches!(
                    level,
                    MatchLevel::VerifiedIdentical
                        | MatchLevel::ProbableIdentical
                        | MatchLevel::SameSizeOnly
                );
                if identical_candidate && input.profile.duplicates == DuplicatePolicy::SkipVerified
                {
                    action = PlanAction::SkipDuplicate {
                        existing_rel: existing.rel_path.clone(),
                        level,
                    };
                } else if !identical_candidate && input.profile.conflicts == ConflictPolicy::Flag {
                    action = PlanAction::Conflict {
                        existing_rel: existing.rel_path.clone(),
                        level,
                    };
                } else {
                    let original = target_rel.clone();
                    target_rel = free_name(idx, &original);
                    notes.push(PlanNote::RenamedExistingDifferent {
                        original_rel: original,
                    });
                }
            } else if idx.reserved.contains(&collision_key(&target_rel)) {
                let original = target_rel.clone();
                target_rel = free_name(idx, &original);
                notes.push(PlanNote::RenamedPlanCollision {
                    original_rel: original,
                });
            }

            // Same content elsewhere in the destination (different name)?
            if matches!(action, PlanAction::Copy) {
                if let Some(cands) = idx.by_size.get(&f.size).cloned() {
                    for c in cands {
                        if collision_key(&c.rel_path) == collision_key(&target_rel) {
                            continue;
                        }
                        let ex_hash = if source_sha.is_some() {
                            cached_hash(idx, hasher, dest_root, c, control)
                        } else {
                            None
                        };
                        let level = compare(
                            &Facts {
                                name: f.file_name(),
                                size: Some(f.size),
                                mtime_ns: Some(f.stamp.mtime_ns),
                                sha256: source_sha.as_ref(),
                            },
                            &Facts {
                                name: c.file_name(),
                                size: Some(c.size),
                                mtime_ns: Some(c.stamp.mtime_ns),
                                sha256: ex_hash.as_ref(),
                            },
                        );
                        if level == MatchLevel::VerifiedIdentical
                            && input.profile.duplicates == DuplicatePolicy::SkipVerified
                        {
                            action = PlanAction::SkipDuplicate {
                                existing_rel: c.rel_path.clone(),
                                level,
                            };
                            break;
                        }
                        if matches!(
                            level,
                            MatchLevel::ProbableIdentical | MatchLevel::SameSizeOnly
                        ) {
                            notes.push(PlanNote::PossibleDuplicate {
                                existing_rel: c.rel_path.clone(),
                                level,
                            });
                            break;
                        }
                    }
                }
            }

            match &action {
                PlanAction::Copy => {
                    idx.reserved.insert(collision_key(&target_rel));
                    required[di] += f.size;
                }
                PlanAction::SkipDuplicate { .. } => {
                    // Reserve too: if re-verification fails the executor copies here.
                    idx.reserved.insert(collision_key(&target_rel));
                }
                PlanAction::Conflict { .. } => {}
            }
            targets.push(PlanTarget {
                dest_index: di,
                target_rel,
                action,
                notes,
            });
        }

        // Summary counts the first destination's view per file, plus any copy.
        if targets.iter().any(|t| matches!(t.action, PlanAction::Copy)) {
            summary.to_copy += 1;
            summary.bytes_to_copy += f.size;
        } else if targets
            .iter()
            .any(|t| matches!(t.action, PlanAction::Conflict { .. }))
        {
            summary.conflicts += 1;
        } else {
            summary.skip_duplicates += 1;
        }
        if targets.iter().any(|t| {
            t.notes.iter().any(|n| {
                matches!(
                    n,
                    PlanNote::RenamedExistingDifferent { .. }
                        | PlanNote::RenamedPlanCollision { .. }
                )
            })
        }) {
            summary.renamed += 1;
        }
        if targets.iter().any(|t| {
            t.notes
                .iter()
                .any(|n| matches!(n, PlanNote::PossibleDuplicate { .. }))
        }) {
            summary.possible_duplicates += 1;
        }
        items.push(PlanItem {
            source: f.clone(),
            category,
            source_sha256: source_sha,
            excluded: None,
            targets,
        });
    }
    progress(src.files.len(), src.files.len());

    let destinations = input
        .destinations
        .iter()
        .enumerate()
        .map(|(i, (scan, label))| {
            let available = fs4::available_space(&scan.root).ok();
            PlanDestination {
                root: scan.root.clone(),
                label: label.clone(),
                required_bytes: required[i],
                available_bytes: available,
                // Keep a 1% / 64 MiB safety margin for file system overhead.
                space_ok: available
                    .is_some_and(|a| a >= required[i] + (required[i] / 100).max(64 << 20).min(a)),
                existing_files: scan.files.len(),
                index_complete: scan.complete,
            }
        })
        .collect();

    Ok(ImportPlan {
        created_at: crate::util::now(),
        source_root: src.root.clone(),
        source_label: input.source_label.to_string(),
        project: input.project.to_string(),
        profile: input.profile.clone(),
        destinations,
        items,
        scan_issues: src.issues.clone(),
        source_scan_complete: src.complete,
        summary,
    })
}

fn cached_hash(
    idx: &mut DestIndex<'_>,
    hasher: &mut dyn HashProvider,
    root: &Path,
    f: &ScannedFile,
    control: &Control,
) -> Option<Sha256Hex> {
    idx.hashes
        .entry(f.rel_path.clone())
        .or_insert_with(|| hasher.hash(&root.join(&f.rel_path), &f.stamp, control).ok())
        .clone()
}

fn free_name(idx: &DestIndex<'_>, rel: &str) -> String {
    let mut n = 2;
    loop {
        let cand = suffixed(rel, n);
        let k = collision_key(&cand);
        if !idx.by_key.contains_key(&k) && !idx.reserved.contains(&k) {
            return cand;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_names() {
        assert_eq!(suffixed("a/b/C0001.MP4", 2), "a/b/C0001 (2).MP4");
        assert_eq!(suffixed("noext", 3), "noext (3)");
        assert_eq!(suffixed(".hidden", 2), ".hidden (2)");
        assert_eq!(suffixed("x.tar.gz", 2), "x.tar (2).gz");
    }
}
