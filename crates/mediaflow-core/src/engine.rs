//! Public facade of the engine. UIs (Tauri app, CLI) only talk to [`Engine`].
//!
//! Authorization: every path used for reading must be inside a *source* or
//! *destination* grant, every path written must be inside a *destination*
//! grant. Grants are created only by explicit user action (native folder
//! picker in the app, command-line arguments in the CLI). Checks are repeated
//! at execution time.

use std::collections::HashSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::db::{Db, DestRow, FileRow, Grant, GrantKind, LibraryRow, OpRow, SessionRow};
use crate::error::{MfError, Result};
use crate::fsops;
use crate::hash::Sha256Hex;
use crate::paths::{self, canonical_dir};
use crate::plan::{build_plan, HashProvider, ImportPlan, PlanAction, PlanInput};
use crate::profile::Profile;
use crate::scan::{self, FileStamp, ScanOptions, ScanResult, TEMP_PREFIX};
use crate::state::{OpEvent, OpState, SessionEvent, SessionState};
use crate::util::Control;

pub struct Engine {
    db: Mutex<Db>,
    running: Mutex<HashSet<String>>,
    _lock: Option<File>,
    db_path: Option<PathBuf>,
}

/// Hash provider backed by the engine's invalidating cache.
pub struct CachedHasher<'a> {
    pub engine: &'a Engine,
}

impl HashProvider for CachedHasher<'_> {
    fn hash(&mut self, abs: &Path, stamp: &FileStamp, control: &Control) -> Result<Sha256Hex> {
        let key = abs.to_string_lossy();
        if let Some(h) = self.engine.db().cached_hash(&key, stamp)? {
            return Ok(h);
        }
        let before = FileStamp::read(abs)?;
        let (h, _) = crate::hash::hash_file(abs, control, |_| {})?;
        let after = FileStamp::read(abs)?;
        // Only cache if the file did not change while we read it.
        if before == after && after == *stamp {
            self.engine.db().put_hash(&key, stamp, &h)?;
        }
        Ok(h)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRequest {
    pub source: PathBuf,
    pub source_label: Option<String>,
    pub destinations: Vec<PathBuf>,
    pub project: String,
    pub profile: Profile,
    /// Hash duplicate candidates so duplicates can be proven (recommended).
    pub hash_candidates: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub state: SessionState,
    pub project: String,
    pub profile_name: String,
    pub source_root: String,
    pub source_label: String,
    pub destinations: Vec<DestRow>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub counts: OpCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpCounts {
    pub total: usize,
    pub pending: usize,
    pub done: usize,
    pub skipped_duplicate: usize,
    pub conflict: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub bytes_done: u64,
}

/// Folders that must never be used as a destination.
fn is_forbidden_destination(p: &Path) -> bool {
    if p.parent().is_none() {
        return true; // filesystem root, e.g. "/" or "C:\"
    }
    let s = p.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    const DENY: [&str; 12] = [
        "/bin",
        "/sbin",
        "/usr",
        "/etc",
        "/system",
        "/library",
        "/private/etc",
        "/proc",
        "/sys",
        "/dev",
        "c:/windows",
        "c:/program files",
    ];
    DENY.iter()
        .any(|d| s == *d || s.starts_with(&format!("{d}/")))
        || dirs_home().is_some_and(|h| h == p)
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .and_then(|h| paths::dunce_canonicalize(&h).ok())
}

impl Engine {
    /// Open (or create) the library database. Takes an exclusive lock so only
    /// one process drives the journal, then marks sessions that were running
    /// when the previous process died as `interrupted`.
    pub fn open(db_path: &Path) -> Result<Engine> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MfError::io(parent, e))?;
        }
        let lock_path = db_path.with_extension("lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| MfError::io(&lock_path, e))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(MfError::Locked(lock_path)),
            Err(std::fs::TryLockError::Error(e)) => return Err(MfError::io(&lock_path, e)),
        }
        let db = Db::open(db_path)?;
        let e = Engine {
            db: Mutex::new(db),
            running: Mutex::new(HashSet::new()),
            _lock: Some(lock),
            db_path: Some(db_path.to_path_buf()),
        };
        e.recover_crashed_sessions()?;
        Ok(e)
    }

    pub fn open_in_memory() -> Result<Engine> {
        Ok(Engine {
            db: Mutex::new(Db::open_in_memory()?),
            running: Mutex::new(HashSet::new()),
            _lock: None,
            db_path: None,
        })
    }

    pub fn db_path(&self) -> Option<&Path> {
        self.db_path.as_deref()
    }

    pub(crate) fn db(&self) -> MutexGuard<'_, Db> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn recover_crashed_sessions(&self) -> Result<()> {
        let db = self.db();
        for s in db.list_sessions(10_000)? {
            if s.state.is_active() {
                let next = s.state.apply(SessionEvent::Interrupt)?;
                db.set_session_state(
                    &s.id,
                    next,
                    Some("MediaFlow stopped while this import was running."),
                )?;
                db.log_event(
                    Some(&s.id),
                    "warning",
                    "interrupted",
                    "Import was interrupted (application stopped). It can be resumed.",
                    None,
                )?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ grants

    /// Register a folder chosen by the user. Only call this from an explicit
    /// user action.
    pub fn authorize(&self, path: &Path, kind: GrantKind) -> Result<Grant> {
        let c = canonical_dir(path)?;
        if kind == GrantKind::Destination && is_forbidden_destination(&c) {
            return Err(MfError::PathRejected {
                path: c.to_string_lossy().into_owned(),
                reason: "system folders, drive roots of the system disk and the home folder itself cannot be destinations; choose or create a sub-folder",
            });
        }
        let label = c
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| c.to_string_lossy().into_owned());
        self.db().add_grant(&c.to_string_lossy(), kind, &label)
    }

    pub fn grants(&self) -> Result<Vec<Grant>> {
        self.db().list_grants()
    }

    pub fn grant(&self, id: &str) -> Result<Grant> {
        self.db().get_grant(id)
    }

    pub fn revoke(&self, id: &str) -> Result<()> {
        self.db().revoke_grant(id)
    }

    /// Canonicalize `path` and check that it lies in a grant of `kind`
    /// (for reads, destination grants also allow reading).
    pub fn check_authorized(&self, path: &Path, write: bool) -> Result<PathBuf> {
        let c = paths::dunce_canonicalize(path)?;
        let ok = self.db().list_grants()?.iter().any(|g| {
            (g.kind == GrantKind::Destination || !write) && paths::is_within(Path::new(&g.path), &c)
        });
        if ok {
            Ok(c)
        } else {
            Err(MfError::NotAuthorized(c))
        }
    }

    // ------------------------------------------------------------- scanning

    pub fn scan(
        &self,
        path: &Path,
        control: &Control,
        progress: impl FnMut(usize, u64),
    ) -> Result<ScanResult> {
        let root = self.check_authorized(path, false)?;
        scan::scan(
            &root,
            &ScanOptions {
                classify: true,
                skip_prefixes: vec![],
            },
            control,
            progress,
        )
    }

    pub fn plan(
        &self,
        req: &PlanRequest,
        control: &Control,
        mut progress: impl FnMut(&str, usize, usize),
    ) -> Result<ImportPlan> {
        if req.destinations.is_empty() {
            return Err(MfError::InvalidInput(
                "choose at least one destination".into(),
            ));
        }
        req.profile.validate()?;
        let src = self.check_authorized(&req.source, false)?;
        let mut dests = Vec::new();
        for d in &req.destinations {
            let c = self.check_authorized(d, true)?;
            if paths::is_within(&src, &c) || paths::is_within(&c, &src) {
                return Err(MfError::PathRejected {
                    path: c.to_string_lossy().into_owned(),
                    reason: "source and destination must not contain each other",
                });
            }
            if dests.contains(&c) {
                return Err(MfError::InvalidInput(
                    "the same destination was chosen twice".into(),
                ));
            }
            dests.push(c);
        }
        progress("scanning_source", 0, 0);
        let source_scan = scan::scan(
            &src,
            &ScanOptions {
                classify: true,
                skip_prefixes: vec![],
            },
            control,
            |n, _| progress("scanning_source", n, 0),
        )?;
        let mut dest_scans = Vec::new();
        for d in &dests {
            progress("indexing_destination", 0, 0);
            let s = scan::scan(
                d,
                &ScanOptions {
                    classify: false,
                    skip_prefixes: vec![],
                },
                control,
                |n, _| progress("indexing_destination", n, 0),
            )?;
            let label = d
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| d.to_string_lossy().into_owned());
            dest_scans.push((s, label));
        }
        let label = req.source_label.clone().unwrap_or_else(|| {
            src.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Source".into())
        });
        let mut hasher = CachedHasher { engine: self };
        build_plan(
            PlanInput {
                source: &source_scan,
                source_label: &label,
                project: &req.project,
                profile: &req.profile,
                destinations: dest_scans.iter().map(|(s, l)| (s, l.clone())).collect(),
                hash_candidates: req.hash_candidates,
            },
            &mut hasher,
            control,
            |i, n| progress("planning", i, n),
        )
    }

    /// Persist a plan as a new session (state `planned`). Authorization is
    /// checked again here.
    pub fn create_session(&self, plan: &ImportPlan) -> Result<String> {
        self.check_authorized(&plan.source_root, false)?;
        for d in &plan.destinations {
            self.check_authorized(&d.root, true)?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let state = SessionState::Queued.apply(SessionEvent::Planned)?;
        let row = SessionRow {
            id: id.clone(),
            state,
            project: plan.project.clone(),
            profile_json: serde_json::to_string(&plan.profile)?,
            source_root: plan.source_root.to_string_lossy().into_owned(),
            source_label: plan.source_label.clone(),
            source_scan_complete: plan.source_scan_complete,
            created_at: crate::util::now(),
            started_at: None,
            finished_at: None,
            active_seconds: 0.0,
            last_error: None,
            scan_issues_json: serde_json::to_string(&plan.scan_issues)?,
        };
        let dests: Vec<DestRow> = plan
            .destinations
            .iter()
            .enumerate()
            .map(|(i, d)| DestRow {
                idx: i as i64,
                root: d.root.to_string_lossy().into_owned(),
                label: d.label.clone(),
            })
            .collect();
        let mut files = Vec::new();
        let mut ops = Vec::new();
        for (i, item) in plan.items.iter().enumerate() {
            let fid = i as i64;
            files.push(FileRow {
                file_id: fid,
                rel_path: item.source.rel_path.clone(),
                size: item.source.size,
                mtime_ns: item.source.stamp.mtime_ns,
                inode: item.source.stamp.inode,
                category: item.category.as_str().into(),
                confidence: serde_json::to_value(item.source.class.confidence)?
                    .as_str()
                    .unwrap_or("unknown")
                    .into(),
                excluded_pattern: item.excluded.as_ref().map(|e| e.pattern.clone()),
                excluded_reason: item.excluded.as_ref().map(|e| e.reason.clone()),
                source_sha256: item.source_sha256.as_ref().map(|h| h.to_string()),
            });
            for t in &item.targets {
                let (action, aref, level) = match &t.action {
                    PlanAction::Copy => ("copy", None, None),
                    PlanAction::SkipDuplicate {
                        existing_rel,
                        level,
                    } => ("skip_duplicate", Some(existing_rel.clone()), Some(*level)),
                    PlanAction::Conflict {
                        existing_rel,
                        level,
                    } => ("conflict", Some(existing_rel.clone()), Some(*level)),
                };
                ops.push(OpRow {
                    file_id: fid,
                    dest_idx: t.dest_index as i64,
                    target_rel: t.target_rel.clone(),
                    final_rel: None,
                    action: action.into(),
                    action_ref: aref,
                    action_level: level.map(|l| {
                        serde_json::to_value(l)
                            .ok()
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default()
                    }),
                    state: OpState::Pending,
                    temp_name: None,
                    sha256: None,
                    bytes: 0,
                    final_stamp_json: None,
                    error_code: None,
                    error_message: None,
                    notes_json: serde_json::to_string(&t.notes)?,
                });
            }
        }
        let mut db = self.db();
        db.insert_session(&row, &dests, &files, &ops)?;
        db.log_event(
            Some(&id),
            "info",
            "planned",
            &format!(
                "Import planned: {} files, {} to copy.",
                plan.summary.files, plan.summary.to_copy
            ),
            None,
        )?;
        Ok(id)
    }

    // ------------------------------------------------------------- sessions

    pub(crate) fn mark_running(&self, id: &str) -> Result<RunningGuard<'_>> {
        let mut r = self.running.lock().unwrap_or_else(|e| e.into_inner());
        if !r.insert(id.to_string()) {
            return Err(MfError::InvalidInput(
                "this import is already running".into(),
            ));
        }
        Ok(RunningGuard {
            engine: self,
            id: id.to_string(),
        })
    }

    pub fn is_running(&self, id: &str) -> bool {
        self.running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(id)
    }

    pub fn session(&self, id: &str) -> Result<SessionInfo> {
        let db = self.db();
        let s = db.get_session(id)?;
        self.info(&db, s)
    }

    pub fn sessions(&self, limit: usize) -> Result<Vec<SessionInfo>> {
        let db = self.db();
        db.list_sessions(limit)?
            .into_iter()
            .map(|s| self.info(&db, s))
            .collect()
    }

    fn info(&self, db: &Db, s: SessionRow) -> Result<SessionInfo> {
        let ops = db.ops(&s.id)?;
        let mut c = OpCounts {
            total: ops.len(),
            ..Default::default()
        };
        for o in &ops {
            match o.state {
                OpState::Pending | OpState::Copying | OpState::Verifying => c.pending += 1,
                OpState::Done => {
                    c.done += 1;
                    c.bytes_done += o.bytes;
                }
                OpState::SkippedDuplicate => c.skipped_duplicate += 1,
                OpState::Conflict => c.conflict += 1,
                OpState::Failed => c.failed += 1,
                OpState::Cancelled => c.cancelled += 1,
            }
        }
        let profile_name = serde_json::from_str::<Profile>(&s.profile_json)
            .map(|p| p.name)
            .unwrap_or_default();
        Ok(SessionInfo {
            destinations: db.destinations(&s.id)?,
            id: s.id,
            state: s.state,
            project: s.project,
            profile_name,
            source_root: s.source_root,
            source_label: s.source_label,
            created_at: s.created_at,
            finished_at: s.finished_at,
            last_error: s.last_error,
            counts: c,
        })
    }

    pub fn session_ops(&self, id: &str) -> Result<(Vec<FileRow>, Vec<OpRow>)> {
        let db = self.db();
        Ok((db.files(id)?, db.ops(id)?))
    }

    pub fn events(&self, id: &str) -> Result<Vec<crate::db::EventRow>> {
        self.db().events(id)
    }

    /// Cancel a session that is not currently running (paused, interrupted,
    /// failed or planned). Our own incomplete temporary files are removed;
    /// finalized copies and all source files are left untouched.
    pub fn cancel_session(&self, id: &str) -> Result<()> {
        if self.is_running(id) {
            return Err(MfError::InvalidInput(
                "stop the running import with its cancel control".into(),
            ));
        }
        let db = self.db();
        let s = db.get_session(id)?;
        let next = s.state.apply(SessionEvent::Cancel)?;
        let dests = db.destinations(id)?;
        for o in db.ops(id)? {
            if matches!(
                o.state,
                OpState::Pending | OpState::Copying | OpState::Verifying
            ) {
                if let (Some(t), Some(d)) =
                    (&o.temp_name, dests.iter().find(|d| d.idx == o.dest_idx))
                {
                    remove_own_temp(Path::new(&d.root), &o.target_rel, t);
                }
                db.update_op(
                    id,
                    o.file_id,
                    o.dest_idx,
                    o.state.apply(OpEvent::Cancel)?,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )?;
            }
        }
        db.set_session_state(id, next, None)?;
        db.log_event(
            Some(id),
            "info",
            "cancelled",
            "Import cancelled. Files already copied and verified were kept.",
            None,
        )?;
        Ok(())
    }

    pub fn library(&self, query: &str, limit: usize) -> Result<Vec<LibraryRow>> {
        self.db().library(query, limit)
    }
}

pub(crate) struct RunningGuard<'a> {
    engine: &'a Engine,
    id: String,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.engine
            .running
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Path of the temporary file of an op: same directory as the target.
pub(crate) fn temp_path(dest_root: &Path, target_rel: &str, temp_name: &str) -> PathBuf {
    let rel = Path::new(target_rel);
    let dir = rel
        .parent()
        .map(|p| dest_root.join(p))
        .unwrap_or_else(|| dest_root.to_path_buf());
    dir.join(temp_name)
}

/// Remove a MediaFlow temporary file, only if it is a regular file whose name
/// carries our prefix. Returns true if something was removed.
pub(crate) fn remove_own_temp(dest_root: &Path, target_rel: &str, temp_name: &str) -> bool {
    if !temp_name.starts_with(TEMP_PREFIX) || temp_name.contains(['/', '\\']) {
        return false;
    }
    let p = temp_path(dest_root, target_rel, temp_name);
    match std::fs::symlink_metadata(&p) {
        Ok(md) if md.is_file() && !paths::is_link(&md) => {
            let ok = std::fs::remove_file(&p).is_ok();
            if ok {
                if let Some(parent) = p.parent() {
                    let _ = fsops::sync_dir(parent);
                }
            }
            ok
        }
        _ => false,
    }
}
