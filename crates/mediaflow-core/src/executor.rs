//! Import execution: verified copy with a durable journal.
//!
//! Per file, for every destination:
//! 1. re-check authorization, path safety and that the source is unchanged;
//! 2. journal `copying` + temp name, create the temp file exclusively;
//! 3. read the source once, hash it and fan the bytes out to every temp file;
//! 4. fsync, re-check the source stamp (modified during copy?);
//! 5. journal `verifying`, re-read each temp file and compare SHA-256;
//! 6. finalize with a no-replace rename, fsync the directory, journal `done`.
//!
//! A crash at any point leaves either a temp file (discarded on resume) or a
//! finalized file whose content is re-verified on resume before being
//! accepted. Source files are only ever opened for reading.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::db::{FileRow, LibraryRow, OpRow};
use crate::engine::{remove_own_temp, Engine};
use crate::error::{MfError, Result};
use crate::fsops;
use crate::hash::{hash_file, Hasher, Sha256Hex, CHUNK};
use crate::paths;
use crate::plan::suffixed;
use crate::profile::{Profile, VerifyLevel};
use crate::progress::{CurrentFile, FileOutcome, FilePhase, Progress, RateMeter};
use crate::scan::{FileStamp, ScanIssue, ScanIssueKind, TEMP_PREFIX};
use crate::state::{OpEvent, OpState, SessionEvent, SessionState};
use crate::util::Control;

/// Fault injection for tests. Production code uses [`NoHooks`].
pub enum Fault {
    Io(std::io::Error),
    /// Stop immediately without cleanup, like a killed process.
    Crash,
}

pub trait IoHooks: Send + Sync {
    fn after_write(
        &self,
        _rel: &str,
        _dest_idx: usize,
        _written: u64,
    ) -> std::result::Result<(), Fault> {
        Ok(())
    }
    fn before_verify(
        &self,
        _rel: &str,
        _dest_idx: usize,
        _temp: &Path,
    ) -> std::result::Result<(), Fault> {
        Ok(())
    }
    fn before_finalize(&self, _rel: &str, _dest_idx: usize) -> std::result::Result<(), Fault> {
        Ok(())
    }
    fn after_finalize(&self, _rel: &str, _dest_idx: usize) -> std::result::Result<(), Fault> {
        Ok(())
    }
}

pub struct NoHooks;
impl IoHooks for NoHooks {}

#[derive(Clone)]
pub struct RunOptions {
    pub hooks: Arc<dyn IoHooks>,
    /// Debug only: sleep between chunks (used by the kill/restart test).
    pub throttle: Option<Duration>,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            hooks: Arc::new(NoHooks),
            throttle: None,
        }
    }
}

struct Dest {
    idx: i64,
    root: PathBuf,
}

struct Writer {
    op: usize,
    /// Open while writing; closed after fsync so the temp file can be
    /// verified and renamed without an open handle (required on Windows).
    file: Option<std::fs::File>,
    temp: PathBuf,
    target: PathBuf,
    target_rel: String,
}

enum Stop {
    /// Pause / cancel / volume loss / crash: stop the whole session.
    Session(MfError),
}

pub struct Runner<'e> {
    engine: &'e Engine,
    id: String,
    control: Control,
    opts: RunOptions,
    profile: Profile,
    source_root: PathBuf,
    source_label: String,
    project: String,
    dests: Vec<Dest>,
    files: Vec<FileRow>,
    ops: Vec<OpRow>,
    state: SessionState,
    progress: Progress,
    meter: RateMeter,
    last_emit: Instant,
    on_progress: &'e mut dyn FnMut(&Progress),
}

impl Engine {
    /// Run (or resume) a session on the calling thread until it completes,
    /// pauses, is cancelled or is interrupted. Returns the resulting state.
    pub fn run_session(
        &self,
        id: &str,
        control: &Control,
        opts: RunOptions,
        on_progress: &mut dyn FnMut(&Progress),
    ) -> Result<SessionState> {
        let _guard = self.mark_running(id)?;
        control.set_stop_on_pause(true);
        let (s, dests, files, ops) = {
            let db = self.db();
            (
                db.get_session(id)?,
                db.destinations(id)?,
                db.files(id)?,
                db.ops(id)?,
            )
        };
        let profile: Profile = serde_json::from_str(&s.profile_json)?;
        let mut r = Runner {
            engine: self,
            id: id.to_string(),
            control: control.clone(),
            opts,
            profile,
            source_root: PathBuf::from(&s.source_root),
            source_label: s.source_label.clone(),
            project: s.project.clone(),
            dests: dests
                .iter()
                .map(|d| Dest {
                    idx: d.idx,
                    root: PathBuf::from(&d.root),
                })
                .collect(),
            files,
            state: s.state,
            progress: Progress {
                session_id: id.to_string(),
                ops_total: ops.len(),
                ..Default::default()
            },
            ops,
            meter: RateMeter::new(),
            last_emit: Instant::now() - Duration::from_secs(1),
            on_progress,
        };
        let started = Instant::now();
        let res = r.run(&s.scan_issues_json);
        let _ = self
            .db()
            .add_active_seconds(id, started.elapsed().as_secs_f64());
        res
    }
}

impl Runner<'_> {
    fn set_state(&mut self, ev: SessionEvent, error: Option<&str>) -> Result<()> {
        let next = self.state.apply(ev)?;
        self.engine.db().set_session_state(&self.id, next, error)?;
        self.state = next;
        self.progress.state = Some(next);
        self.emit(true);
        Ok(())
    }

    fn log(&self, level: &str, code: &str, msg: &str, rel: Option<&str>) {
        let _ = self
            .engine
            .db()
            .log_event(Some(&self.id), level, code, msg, rel);
    }

    fn emit(&mut self, force: bool) {
        if force || self.last_emit.elapsed() >= Duration::from_millis(150) {
            self.meter
                .record(self.progress.bytes_copied + self.progress.bytes_verified);
            self.progress.recompute(&self.meter);
            (self.on_progress)(&self.progress);
            self.last_emit = Instant::now();
        }
    }

    /// Apply an op transition in memory and in the journal.
    #[allow(clippy::too_many_arguments)]
    fn op_to(
        &mut self,
        i: usize,
        ev: OpEvent,
        temp: Option<&str>,
        final_rel: Option<&str>,
        sha: Option<&Sha256Hex>,
        stamp: Option<&FileStamp>,
        err: Option<&MfError>,
    ) -> Result<()> {
        let next = self.ops[i].state.apply(ev)?;
        let bytes = if next == OpState::Done {
            Some(self.file(i).size)
        } else {
            None
        };
        let e = err.map(|e| (e.code(), e.user_message()));
        self.engine.db().update_op(
            &self.id,
            self.ops[i].file_id,
            self.ops[i].dest_idx,
            next,
            temp,
            final_rel,
            sha.map(|h| h.as_str()),
            bytes,
            stamp,
            e.as_ref().map(|(c, m)| (*c, m.as_str())),
        )?;
        let o = &mut self.ops[i];
        o.state = next;
        o.temp_name = temp.map(String::from);
        if let Some(f) = final_rel {
            o.final_rel = Some(f.into());
        }
        if let Some(h) = sha {
            o.sha256 = Some(h.to_string());
        }
        if let Some(s) = stamp {
            o.final_stamp_json = serde_json::to_string(s).ok();
        }
        if let Some(b) = bytes {
            o.bytes = b;
        }
        o.error_code = e.as_ref().map(|(c, _)| c.to_string());
        o.error_message = e.map(|(_, m)| m);
        if matches!(
            next,
            OpState::Done | OpState::SkippedDuplicate | OpState::Failed | OpState::Conflict
        ) {
            match next {
                OpState::Failed => self.progress.ops_failed += 1,
                OpState::SkippedDuplicate | OpState::Conflict => self.progress.ops_skipped += 1,
                _ => self.progress.ops_done += 1,
            }
            let f = self.file(i).clone();
            self.progress.last_outcome = Some(FileOutcome {
                file_id: f.file_id,
                dest_idx: self.ops[i].dest_idx,
                rel_path: f.rel_path.clone(),
                state: next,
                error_code: self.ops[i].error_code.clone(),
                message: self.ops[i].error_message.clone(),
            });
            if let Some(err) = err {
                let rel = f.rel_path.clone();
                self.log("error", err.code(), &err.user_message(), Some(&rel));
            }
            self.emit(true);
        }
        Ok(())
    }

    fn fail(&mut self, i: usize, err: &MfError) -> Result<()> {
        self.op_to(i, OpEvent::Fail, None, None, None, None, Some(err))
    }

    fn file(&self, op: usize) -> &FileRow {
        let fid = self.ops[op].file_id;
        &self.files[fid as usize]
    }

    fn dest(&self, op: usize) -> &Dest {
        let di = self.ops[op].dest_idx;
        self.dests
            .iter()
            .find(|d| d.idx == di)
            .expect("destination of op")
    }

    fn run(&mut self, scan_issues_json: &str) -> Result<SessionState> {
        let ev = match self.state {
            SessionState::Planned => SessionEvent::Start,
            s if s.is_resumable() => SessionEvent::Resume,
            s => {
                return Err(MfError::InvalidTransition {
                    from: s.as_str().into(),
                    event: "Start".into(),
                });
            }
        };
        // Authorization is re-checked at execution time.
        if let Err(e) = self.recheck_authorization() {
            self.engine.db().log_event(
                Some(&self.id),
                "error",
                e.code(),
                &e.user_message(),
                None,
            )?;
            return Err(e);
        }
        self.engine.db().clear_session_error(&self.id)?;
        self.set_state(ev, None)?;
        self.log(
            "info",
            if ev == SessionEvent::Start {
                "started"
            } else {
                "resumed"
            },
            if ev == SessionEvent::Start {
                "Import started."
            } else {
                "Import resumed; checking the state of the disks first."
            },
            None,
        );

        match self.execute() {
            Ok(()) => {}
            Err(e) => return self.stop(e),
        }
        self.set_state(SessionEvent::CopyFinished, None)?;
        match self.final_verification() {
            Ok(()) => {}
            Err(e) => return self.stop(e),
        }
        let issues: Vec<ScanIssue> = serde_json::from_str(scan_issues_json).unwrap_or_default();
        let unreadable = issues
            .iter()
            .filter(|i| i.kind == ScanIssueKind::Unreadable)
            .count();
        let all_ok = self.ops.iter().all(|o| o.state.is_success()) && unreadable == 0;
        if all_ok {
            self.set_state(SessionEvent::VerifyOk, None)?;
            self.log("info", "completed", "Import completed. Every file was copied and verified, or already present with verified identical content.", None);
        } else {
            let failed = self.ops.iter().filter(|o| !o.state.is_success()).count();
            let msg = format!(
                "Import finished with problems: {failed} operation(s) not completed{}. Review them before formatting the card.",
                if unreadable > 0 { format!(", {unreadable} unreadable source item(s)") } else { String::new() }
            );
            self.set_state(SessionEvent::VerifyWithErrors, Some(&msg))?;
            self.log("warning", "completed_with_errors", &msg, None);
        }
        Ok(self.state)
    }

    fn recheck_authorization(&self) -> Result<()> {
        self.engine.check_authorized(&self.source_root, false)?;
        for d in &self.dests {
            if d.root.exists() {
                self.engine.check_authorized(&d.root, true)?;
            }
        }
        Ok(())
    }

    /// Translate a session-level stop into the right state.
    fn stop(&mut self, e: MfError) -> Result<SessionState> {
        match e {
            MfError::SimulatedCrash => Err(e),
            MfError::Paused => {
                self.set_state(SessionEvent::Pause, None)?;
                self.log(
                    "info",
                    "paused",
                    "Import paused. Files already verified are kept; resume to continue.",
                    None,
                );
                Ok(self.state)
            }
            MfError::Cancelled => {
                for i in 0..self.ops.len() {
                    if matches!(
                        self.ops[i].state,
                        OpState::Pending | OpState::Copying | OpState::Verifying
                    ) {
                        self.op_to(i, OpEvent::Cancel, None, None, None, None, None)?;
                    }
                }
                self.set_state(SessionEvent::Cancel, None)?;
                self.log("info", "cancelled", "Import cancelled. Files already copied and verified were kept; nothing was deleted from the source.", None);
                Ok(self.state)
            }
            MfError::VolumeUnavailable(_) => {
                let msg = e.user_message();
                self.set_state(SessionEvent::Interrupt, Some(&msg))?;
                self.log("error", e.code(), &msg, None);
                Ok(self.state)
            }
            other => {
                let msg = other.user_message();
                let _ = self.set_state(SessionEvent::Fail, Some(&msg));
                self.log("error", other.code(), &msg, None);
                Err(other)
            }
        }
    }

    fn require_volumes(&self) -> Result<()> {
        if !self.source_root.is_dir() {
            return Err(MfError::VolumeUnavailable(self.source_root.clone()));
        }
        for d in &self.dests {
            if !d.root.is_dir() {
                return Err(MfError::VolumeUnavailable(d.root.clone()));
            }
        }
        Ok(())
    }

    fn execute(&mut self) -> Result<()> {
        self.require_volumes()?;
        self.recover()?;

        // Totals for progress, from the work that remains.
        self.progress.ops_total = self.ops.len();
        self.progress.ops_done = self.ops.iter().filter(|o| o.state == OpState::Done).count();
        self.progress.ops_skipped = self
            .ops
            .iter()
            .filter(|o| matches!(o.state, OpState::SkippedDuplicate | OpState::Conflict))
            .count();
        self.progress.ops_failed = self
            .ops
            .iter()
            .filter(|o| o.state == OpState::Failed)
            .count();
        let mut by_file: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
        for (i, o) in self.ops.iter().enumerate() {
            if o.state == OpState::Pending {
                by_file.entry(o.file_id).or_default().push(i);
            }
        }
        for (fid, ops) in &by_file {
            let size = self.files[*fid as usize].size;
            if ops.iter().any(|&i| self.ops[i].action != "conflict") {
                self.progress.bytes_to_copy += size;
                self.progress.bytes_to_verify += size * ops.len() as u64;
            }
        }
        self.emit(true);

        for (fid, ops) in by_file {
            self.control.checkpoint()?;
            match self.process_file(fid, &ops) {
                Ok(()) => {}
                Err(Stop::Session(e)) => return Err(e),
            }
        }
        self.progress.current = None;
        Ok(())
    }

    /// Resume-time revalidation of the journal against the real disk state.
    fn recover(&mut self) -> Result<()> {
        for i in 0..self.ops.len() {
            let o = self.ops[i].clone();
            let root = self.dest(i).root.clone();
            match o.state {
                OpState::Copying | OpState::Verifying => {
                    if let Some(t) = &o.temp_name {
                        if remove_own_temp(&root, &o.target_rel, t) {
                            self.log("info", "temp_discarded", "An incomplete copy from the interrupted run was discarded; the file will be copied again.", Some(&o.target_rel));
                        }
                    }
                    self.op_to(i, OpEvent::Reset, None, None, None, None, None)?;
                }
                OpState::Done => {
                    let rel = o.final_rel.clone().unwrap_or(o.target_rel.clone());
                    let abs = paths::resolve_within(&root, Path::new(&rel))?;
                    let recorded: Option<FileStamp> = o
                        .final_stamp_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok());
                    match std::fs::symlink_metadata(&abs) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            self.log("warning", "copy_missing", "A file copied earlier is no longer at the destination; it will be copied again.", Some(&rel));
                            self.op_to(i, OpEvent::Reset, None, None, None, None, None)?;
                        }
                        Err(e) => return Err(MfError::io(&abs, e)),
                        Ok(md) => {
                            let now = FileStamp::of(&md);
                            if recorded
                                .is_some_and(|r| r.size != now.size || r.mtime_ns != now.mtime_ns)
                                || paths::is_link(&md)
                            {
                                let err = MfError::VerificationFailed {
                                    path: abs.clone(),
                                    expected: "unchanged copy".into(),
                                    actual: "modified on disk".into(),
                                };
                                self.fail(i, &err)?;
                            }
                        }
                    }
                }
                OpState::SkippedDuplicate => {
                    let rel = o
                        .final_rel
                        .clone()
                        .or(o.action_ref.clone())
                        .unwrap_or(o.target_rel.clone());
                    if !paths::resolve_within(&root, Path::new(&rel))?.is_file() {
                        self.op_to(i, OpEvent::Reset, None, None, None, None, None)?;
                    }
                }
                OpState::Failed | OpState::Cancelled => {
                    self.op_to(i, OpEvent::Reset, None, None, None, None, None)?;
                }
                OpState::Pending | OpState::Conflict => {}
            }
        }
        Ok(())
    }

    fn crash_or(&mut self, f: Fault) -> std::result::Result<std::io::Error, Stop> {
        match f {
            Fault::Crash => Err(Stop::Session(MfError::SimulatedCrash)),
            Fault::Io(e) => Ok(e),
        }
    }

    /// Distinguish "volume vanished" (stop everything) from a per-file error.
    fn classify_dest_error(
        &self,
        op: usize,
        path: &Path,
        e: std::io::Error,
    ) -> std::result::Result<MfError, Stop> {
        let root = &self.dest(op).root;
        if !root.is_dir() {
            return Err(Stop::Session(MfError::VolumeUnavailable(root.clone())));
        }
        Ok(MfError::io(path, e))
    }

    fn source_error(&self, path: &Path, e: std::io::Error) -> std::result::Result<MfError, Stop> {
        if !self.source_root.is_dir() {
            return Err(Stop::Session(MfError::VolumeUnavailable(
                self.source_root.clone(),
            )));
        }
        Ok(MfError::io(path, e))
    }

    fn set_current(&mut self, fid: i64, phase: FilePhase, done: u64) {
        let f = &self.files[fid as usize];
        self.progress.current = Some(CurrentFile {
            file_id: fid,
            rel_path: f.rel_path.clone(),
            size: f.size,
            phase,
            done,
        });
    }

    fn process_file(&mut self, fid: i64, ops: &[usize]) -> std::result::Result<(), Stop> {
        let s = |e: MfError| Stop::Session(e);
        let file = self.files[fid as usize].clone();
        self.set_current(fid, FilePhase::Checking, 0);
        self.emit(false);

        // 1. Source checks.
        let src = match paths::resolve_within(&self.source_root, Path::new(&file.rel_path)) {
            Ok(p) => p,
            Err(e) => {
                for &i in ops {
                    self.fail(i, &e).map_err(s)?;
                }
                return Ok(());
            }
        };
        let stamp = match FileStamp::read(&src) {
            Ok(st) => st,
            Err(MfError::Io { source, .. }) => {
                let e = self.source_error(&src, source)?;
                let e = if e.code() == "io_not_found" {
                    MfError::SourceChanged(src.clone())
                } else {
                    e
                };
                for &i in ops {
                    self.fail(i, &e).map_err(s)?;
                }
                return Ok(());
            }
            Err(e) => {
                for &i in ops {
                    self.fail(i, &e).map_err(s)?;
                }
                return Ok(());
            }
        };
        let recorded = file.stamp();
        if stamp.size != recorded.size
            || stamp.mtime_ns != recorded.mtime_ns
            || (stamp.inode.is_some() && recorded.inode.is_some() && stamp.inode != recorded.inode)
        {
            let e = MfError::SourceChanged(src.clone());
            for &i in ops {
                self.fail(i, &e).map_err(s)?;
            }
            return Ok(());
        }

        let mut src_hash: Option<Sha256Hex> = None;
        let mut copies: Vec<usize> = Vec::new();

        // 2. Conflicts and duplicate candidates.
        for &i in ops {
            match self.ops[i].action.as_str() {
                "conflict" => {
                    let existing = self.ops[i].action_ref.clone().unwrap_or_default();
                    let e = MfError::Conflict(self.dest(i).root.join(&existing));
                    self.op_to(i, OpEvent::FlagConflict, None, None, None, None, Some(&e))
                        .map_err(s)?;
                }
                "skip_duplicate" => {
                    let existing_rel = self.ops[i].action_ref.clone().unwrap_or_default();
                    if src_hash.is_none() {
                        match self.hash_source(fid, &src)? {
                            Ok(h) => src_hash = Some(h),
                            Err(e) => {
                                for &j in ops {
                                    if self.ops[j].state == OpState::Pending {
                                        self.fail(j, &e).map_err(s)?;
                                    }
                                }
                                return Ok(());
                            }
                        }
                    }
                    match self.hash_existing(i, &existing_rel)? {
                        Some(h) if Some(&h) == src_hash.as_ref() => {
                            self.op_to(
                                i,
                                OpEvent::SkipVerifiedDuplicate,
                                None,
                                Some(&existing_rel),
                                Some(&h),
                                None,
                                None,
                            )
                            .map_err(s)?;
                            self.record_library(i, &existing_rel, &h);
                        }
                        _ => {
                            self.log("info", "duplicate_not_confirmed", "Possible duplicate not confirmed by content check; the file will be copied.", Some(&file.rel_path));
                            copies.push(i);
                        }
                    }
                }
                _ => copies.push(i),
            }
        }
        if copies.is_empty() {
            return Ok(());
        }

        // 3. Prepare targets and temp files.
        let mut writers: Vec<Writer> = Vec::new();
        for i in copies {
            if let Some(w) = self.prepare_target(i, fid, &src, &mut src_hash)? {
                writers.push(w);
            }
        }
        if writers.is_empty() {
            return Ok(());
        }

        // 4. Single read of the source, fanned out to every temp file.
        self.set_current(fid, FilePhase::Copying, 0);
        let mut reader = match std::fs::File::open(&src) {
            Ok(f) => f,
            Err(e) => {
                let err = self.source_error(&src, e)?;
                return self.abort_writers(writers, &err);
            }
        };
        let mut hasher = Hasher::new();
        let mut buf = vec![0u8; CHUNK];
        let mut read_total = 0u64;
        loop {
            if let Err(e) = self.control.checkpoint() {
                self.discard_writers(writers, &e)?;
                return Err(Stop::Session(e));
            }
            if let Some(t) = self.opts.throttle {
                std::thread::sleep(t);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let err = self.source_error(&src, e)?;
                    return self.abort_writers(writers, &err);
                }
            };
            hasher.update(&buf[..n]);
            read_total += n as u64;
            let mut k = 0;
            while k < writers.len() {
                let w = &mut writers[k];
                let mut res = match w.file.as_mut() {
                    Some(f) => f.write_all(&buf[..n]),
                    None => Err(std::io::Error::other("temporary file already closed")),
                };
                if res.is_ok() {
                    if let Err(f) = self.opts.hooks.after_write(
                        &file.rel_path,
                        self.ops[w.op].dest_idx as usize,
                        read_total,
                    ) {
                        res = Err(self.crash_or(f)?);
                    }
                }
                if let Err(e) = res {
                    let w = writers.remove(k);
                    let err = match self.classify_dest_error(w.op, &w.temp, e) {
                        Ok(err) => err,
                        Err(stop) => {
                            writers.push(w);
                            let Stop::Session(se) = &stop;
                            self.discard_writers(writers, se)?;
                            return Err(stop);
                        }
                    };
                    self.drop_writer(w, Some(&err))?;
                    continue;
                }
                k += 1;
            }
            if writers.is_empty() {
                return Ok(());
            }
            self.progress.bytes_copied += n as u64;
            if let Some(c) = self.progress.current.as_mut() {
                c.done = read_total;
            }
            self.emit(false);
        }
        drop(reader);

        // 5. The source must not have changed while we read it.
        let after = FileStamp::read(&src).ok();
        let digest = hasher.finish();
        let src_changed = read_total != file.size
            || after.is_none_or(|a| a != stamp)
            || src_hash.as_ref().is_some_and(|h| *h != digest);
        if src_changed {
            return self.abort_writers(writers, &MfError::SourceChanged(src.clone()));
        }
        self.engine
            .db()
            .set_file_hash(&self.id, fid, digest.as_str())
            .map_err(s)?;

        // fsync temps, journal `verifying`.
        let mut k = 0;
        while k < writers.len() {
            let sync = match writers[k].file.take() {
                Some(f) => f.sync_all(),
                None => Ok(()),
            };
            if let Err(e) = sync {
                let w = writers.remove(k);
                let err = match self.classify_dest_error(w.op, &w.temp, e) {
                    Ok(err) => err,
                    Err(stop) => {
                        writers.push(w);
                        let Stop::Session(se) = &stop;
                        self.discard_writers(writers, se)?;
                        return Err(stop);
                    }
                };
                self.drop_writer(w, Some(&err))?;
                continue;
            }
            let op = writers[k].op;
            let tn = writers[k]
                .temp
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            self.op_to(
                op,
                OpEvent::CopyWritten,
                tn.as_deref(),
                None,
                None,
                None,
                None,
            )
            .map_err(s)?;
            k += 1;
        }

        // 6. Verify: re-read every temp file (in parallel across destinations).
        self.set_current(fid, FilePhase::Verifying, 0);
        for w in &writers {
            if let Err(Fault::Crash) = self.opts.hooks.before_verify(
                &file.rel_path,
                self.ops[w.op].dest_idx as usize,
                &w.temp,
            ) {
                return Err(Stop::Session(MfError::SimulatedCrash));
            }
        }
        let results = self.verify_parallel(&writers);
        let mut verified = Vec::new();
        for (w, r) in writers.into_iter().zip(results) {
            match r {
                Ok(h) if h == digest => verified.push(w),
                Ok(h) => {
                    let err = MfError::VerificationFailed {
                        path: w.target.clone(),
                        expected: digest.to_string(),
                        actual: h.to_string(),
                    };
                    self.drop_writer(w, Some(&err))?;
                }
                Err(MfError::Cancelled) | Err(MfError::Paused) => {
                    let e = if self.control.is_cancelled() {
                        MfError::Cancelled
                    } else {
                        MfError::Paused
                    };
                    verified.push(w);
                    self.discard_writers(verified, &e)?;
                    return Err(Stop::Session(e));
                }
                Err(MfError::Io { source, path }) => {
                    let err = match self.classify_dest_error(w.op, &path, source) {
                        Ok(err) => err,
                        Err(stop) => {
                            verified.push(w);
                            let Stop::Session(se) = &stop;
                            self.discard_writers(verified, se)?;
                            return Err(stop);
                        }
                    };
                    self.drop_writer(w, Some(&err))?;
                }
                Err(e) => self.drop_writer(w, Some(&e))?,
            }
        }

        // 7. Finalize with a no-replace rename.
        self.set_current(fid, FilePhase::Finalizing, file.size);
        for w in verified {
            self.finalize(w, &file, &stamp, &digest)?;
        }
        Ok(())
    }

    /// Hash the source. Outer error stops the session; inner error is a
    /// per-file read failure.
    fn hash_source(
        &mut self,
        fid: i64,
        src: &Path,
    ) -> std::result::Result<Result<Sha256Hex>, Stop> {
        self.set_current(fid, FilePhase::Checking, 0);
        match hash_file(src, &self.control, |_| {}) {
            Ok((h, _)) => Ok(Ok(h)),
            Err(e @ (MfError::Cancelled | MfError::Paused)) => Err(Stop::Session(e)),
            Err(MfError::Io { source, path }) => Ok(Err(self.source_error(&path, source)?)),
            Err(e) => Ok(Err(e)),
        }
    }

    fn hash_existing(
        &mut self,
        op: usize,
        rel: &str,
    ) -> std::result::Result<Option<Sha256Hex>, Stop> {
        let root = self.dest(op).root.clone();
        let Ok(abs) = paths::resolve_within(&root, Path::new(rel)) else {
            return Ok(None);
        };
        match std::fs::symlink_metadata(&abs) {
            Ok(md) if md.is_file() && !paths::is_link(&md) => {}
            _ => return Ok(None),
        }
        match hash_file(&abs, &self.control, |_| {}) {
            Ok((h, _)) => Ok(Some(h)),
            Err(e @ (MfError::Cancelled | MfError::Paused)) => Err(Stop::Session(e)),
            Err(MfError::Io { source, path }) => {
                self.classify_dest_error(op, &path, source)?;
                Ok(None)
            }
            Err(_) => Ok(None),
        }
    }

    /// Resolve the final target for a copy op, handle an already-occupied
    /// target, check free space, journal and create the temp file.
    fn prepare_target(
        &mut self,
        i: usize,
        fid: i64,
        src: &Path,
        src_hash: &mut Option<Sha256Hex>,
    ) -> std::result::Result<Option<Writer>, Stop> {
        let s = |e: MfError| Stop::Session(e);
        let root = self.dest(i).root.clone();
        let size = self.files[fid as usize].size;
        let mut target_rel = self.ops[i].target_rel.clone();
        let mut attempt = 1;
        let target = loop {
            let target = match paths::resolve_within(&root, Path::new(&target_rel)) {
                Ok(t) => t,
                Err(e) => {
                    self.fail(i, &e).map_err(s)?;
                    return Ok(None);
                }
            };
            match std::fs::symlink_metadata(&target) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break target,
                Err(e) => {
                    let err = self.classify_dest_error(i, &target, e)?;
                    self.fail(i, &err).map_err(s)?;
                    return Ok(None);
                }
                Ok(md) => {
                    // Something already sits at the target. If it is a regular
                    // file with identical content (e.g. we finalized it just
                    // before a crash), accept it after verification.
                    if md.is_file() && !paths::is_link(&md) && md.len() == size {
                        if src_hash.is_none() {
                            match self.hash_source(fid, src)? {
                                Ok(h) => *src_hash = Some(h),
                                Err(e) => {
                                    self.fail(i, &e).map_err(s)?;
                                    return Ok(None);
                                }
                            }
                        }
                        if let Some(h) = self.hash_existing(i, &target_rel)? {
                            if Some(&h) == src_hash.as_ref() {
                                let st = FileStamp::of(&md);
                                self.op_to(i, OpEvent::BeginCopy, None, None, None, None, None)
                                    .map_err(s)?;
                                self.op_to(i, OpEvent::CopyWritten, None, None, None, None, None)
                                    .map_err(s)?;
                                self.op_to(
                                    i,
                                    OpEvent::Verified,
                                    None,
                                    Some(&target_rel),
                                    Some(&h),
                                    Some(&st),
                                    None,
                                )
                                .map_err(s)?;
                                self.log("info", "already_present_verified", "Identical content already present at the destination (verified); nothing was written.", Some(&target_rel));
                                self.record_library(i, &target_rel, &h);
                                return Ok(None);
                            }
                        }
                    }
                    attempt += 1;
                    let base = self.ops[i].target_rel.clone();
                    target_rel = suffixed(&base, attempt);
                    if attempt > 999 {
                        self.fail(i, &MfError::Conflict(target)).map_err(s)?;
                        return Ok(None);
                    }
                }
            }
        };
        if target_rel != self.ops[i].target_rel {
            self.log("info", "renamed_at_execution", &format!("The planned name was taken by a different file; copying as \"{target_rel}\" instead. Nothing was overwritten."), Some(&self.ops[i].target_rel.clone()));
            self.engine
                .db()
                .set_op_target(
                    &self.id,
                    self.ops[i].file_id,
                    self.ops[i].dest_idx,
                    &target_rel,
                )
                .map_err(s)?;
            self.ops[i].target_rel = target_rel.clone();
        }
        let dir = target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or(root.clone());
        if let Err(e) = paths::create_dirs_within(&root, &dir) {
            let e = match e {
                MfError::Io { source, path } => self.classify_dest_error(i, &path, source)?,
                other => other,
            };
            self.fail(i, &e).map_err(s)?;
            return Ok(None);
        }
        // Free-space check right before writing (the plan may be stale).
        if let Ok(avail) = fs4::available_space(&dir) {
            let margin = (size / 100).clamp(1 << 20, 64 << 20);
            if avail < size.saturating_add(margin) {
                self.fail(
                    i,
                    &MfError::InsufficientSpace {
                        path: root.clone(),
                        needed: size,
                        available: avail,
                    },
                )
                .map_err(s)?;
                return Ok(None);
            }
        }
        let short = &self.id[..8.min(self.id.len())];
        let temp_name = format!("{TEMP_PREFIX}{short}-{fid}-{}", self.ops[i].dest_idx);
        let temp = dir.join(&temp_name);
        // A stale temp with our exact name can only come from this session.
        remove_own_temp(&root, &target_rel, &temp_name);
        // Journal first, then create: a crash in between leaves no orphan.
        self.op_to(
            i,
            OpEvent::BeginCopy,
            Some(&temp_name),
            None,
            None,
            None,
            None,
        )
        .map_err(s)?;
        match fsops::create_new(&temp) {
            Ok(file) => Ok(Some(Writer {
                op: i,
                file: Some(file),
                temp,
                target,
                target_rel,
            })),
            Err(e) => {
                let err = self.classify_dest_error(i, &temp, e)?;
                self.fail(i, &err).map_err(s)?;
                Ok(None)
            }
        }
    }

    fn verify_parallel(&mut self, writers: &[Writer]) -> Vec<Result<Sha256Hex>> {
        let base = self.progress.bytes_verified;
        let control = self.control.clone();
        // One destination: verify on this thread, no polling latency.
        if let [w] = writers {
            let mut done = 0u64;
            let r = hash_file(&w.temp, &control, |n| {
                done += n;
                self.progress.bytes_verified = base + done;
                if let Some(c) = self.progress.current.as_mut() {
                    c.done = done;
                }
                self.emit(false);
            })
            .map(|(h, _)| h);
            return vec![r];
        }
        // Several destinations (usually different drives): verify in parallel.
        let counter = Arc::new(AtomicU64::new(0));
        let paths: Vec<PathBuf> = writers.iter().map(|w| w.temp.clone()).collect();
        std::thread::scope(|scope| {
            let handles: Vec<_> = paths
                .iter()
                .map(|p| {
                    let c = counter.clone();
                    let control = control.clone();
                    scope.spawn(move || {
                        hash_file(p, &control, |n| {
                            c.fetch_add(n, Ordering::Relaxed);
                        })
                        .map(|(h, _)| h)
                    })
                })
                .collect();
            let mut wait = Duration::from_millis(1);
            while !handles.iter().all(|h| h.is_finished()) {
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_millis(50));
                self.progress.bytes_verified = base + counter.load(Ordering::Relaxed);
                if let Some(c) = self.progress.current.as_mut() {
                    c.done = counter.load(Ordering::Relaxed) / paths.len().max(1) as u64;
                }
                self.emit(false);
            }
            let out = handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(MfError::InvalidInput("verification thread panicked".into()))
                    })
                })
                .collect();
            self.progress.bytes_verified = base + counter.load(Ordering::Relaxed);
            out
        })
    }

    fn finalize(
        &mut self,
        mut w: Writer,
        file: &FileRow,
        src_stamp: &FileStamp,
        digest: &Sha256Hex,
    ) -> std::result::Result<(), Stop> {
        let s = |e: MfError| Stop::Session(e);
        let dest_idx = self.ops[w.op].dest_idx as usize;
        if self.profile.preserve_mtime {
            if let Some(m) = src_stamp.modified() {
                let ft =
                    filetime::FileTime::from_unix_time(m.timestamp(), m.timestamp_subsec_nanos());
                if filetime::set_file_mtime(&w.temp, ft).is_err() {
                    self.log(
                        "warning",
                        "mtime_not_preserved",
                        "The original modification date could not be applied to the copy.",
                        Some(&w.target_rel),
                    );
                }
            }
        }
        if let Err(Fault::Crash) = self.opts.hooks.before_finalize(&file.rel_path, dest_idx) {
            return Err(Stop::Session(MfError::SimulatedCrash));
        }
        let root = self.dest(w.op).root.clone();
        let mut attempt = 1;
        loop {
            match fsops::rename_no_replace(&w.temp, &w.target) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 999 => {
                    // Someone created the target after our check. Never replace it.
                    attempt += 1;
                    let base = self.ops[w.op].target_rel.clone();
                    let new_rel = suffixed(&base, attempt);
                    let new_target = match paths::resolve_within(&root, Path::new(&new_rel)) {
                        Ok(t) => t,
                        Err(e) => return self.drop_writer(w, Some(&e)),
                    };
                    self.log("warning", "renamed_at_finalize", &format!("A file appeared at the target during the copy; saved as \"{new_rel}\" instead. Nothing was overwritten."), Some(&base));
                    w.target = new_target;
                    w.target_rel = new_rel;
                }
                Err(e) => {
                    let err = self.classify_dest_error(w.op, &w.target, e)?;
                    return self.drop_writer(w, Some(&err));
                }
            }
        }
        if let Some(parent) = w.target.parent() {
            let _ = fsops::sync_dir(parent);
        }
        if let Err(Fault::Crash) = self.opts.hooks.after_finalize(&file.rel_path, dest_idx) {
            return Err(Stop::Session(MfError::SimulatedCrash));
        }
        let st = match FileStamp::read(&w.target) {
            Ok(st) if st.size == file.size => st,
            Ok(_) => {
                let e = MfError::VerificationFailed {
                    path: w.target.clone(),
                    expected: format!("{} bytes", file.size),
                    actual: "different size after finalize".into(),
                };
                return self.fail(w.op, &e).map_err(s);
            }
            Err(e) => return self.fail(w.op, &e).map_err(s),
        };
        if w.target_rel != self.ops[w.op].target_rel {
            self.engine
                .db()
                .set_op_target(
                    &self.id,
                    self.ops[w.op].file_id,
                    self.ops[w.op].dest_idx,
                    &w.target_rel,
                )
                .map_err(s)?;
            self.ops[w.op].target_rel = w.target_rel.clone();
        }
        self.op_to(
            w.op,
            OpEvent::Verified,
            None,
            Some(&w.target_rel),
            Some(digest),
            Some(&st),
            None,
        )
        .map_err(s)?;
        self.record_library(w.op, &w.target_rel, digest);
        Ok(())
    }

    /// Close and delete a writer's temp file; mark the op failed with `err`.
    fn drop_writer(&mut self, w: Writer, err: Option<&MfError>) -> std::result::Result<(), Stop> {
        let op = w.op;
        drop(w.file);
        let name = w
            .temp
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let root = self.dest(op).root.clone();
        remove_own_temp(&root, &self.ops[op].target_rel.clone(), &name);
        if let Some(e) = err {
            self.fail(op, e).map_err(Stop::Session)?;
        }
        Ok(())
    }

    /// Fail every writer for a per-file reason (e.g. source changed).
    fn abort_writers(
        &mut self,
        writers: Vec<Writer>,
        err: &MfError,
    ) -> std::result::Result<(), Stop> {
        for w in writers {
            self.drop_writer(w, Some(err))?;
        }
        Ok(())
    }

    /// Session is stopping (pause/cancel/volume loss): remove temps and put the
    /// ops back to pending so a resume starts this file again.
    fn discard_writers(
        &mut self,
        writers: Vec<Writer>,
        why: &MfError,
    ) -> std::result::Result<(), Stop> {
        if matches!(why, MfError::SimulatedCrash) {
            return Ok(());
        }
        for w in writers {
            let op = w.op;
            self.drop_writer(w, None)?;
            self.op_to(op, OpEvent::Reset, None, None, None, None, None)
                .map_err(Stop::Session)?;
        }
        Ok(())
    }

    fn record_library(&self, op: usize, rel: &str, h: &Sha256Hex) {
        let f = self.file(op);
        let row = LibraryRow {
            dest_root: self.dest(op).root.to_string_lossy().into_owned(),
            rel_path: rel.to_string(),
            sha256: h.to_string(),
            size: f.size,
            category: f.category.clone(),
            project: self.project.clone(),
            source_label: self.source_label.clone(),
            source_rel: f.rel_path.clone(),
            session_id: self.id.clone(),
            verified_at: crate::util::now(),
        };
        let _ = self.engine.db().upsert_library(&row);
    }

    /// Session-level verification pass: every finalized copy must still be
    /// there, unchanged; with `Paranoid`, every copy is re-read and re-hashed.
    fn final_verification(&mut self) -> Result<()> {
        self.require_volumes()?;
        let paranoid = self.profile.verify == VerifyLevel::Paranoid;
        if paranoid {
            self.progress.bytes_to_verify += self
                .ops
                .iter()
                .filter(|o| o.state == OpState::Done)
                .map(|o| o.bytes)
                .sum::<u64>();
        }
        for i in 0..self.ops.len() {
            self.control.checkpoint()?;
            let o = self.ops[i].clone();
            if !o.state.is_success() {
                continue;
            }
            let root = self.dest(i).root.clone();
            let rel = o.final_rel.clone().unwrap_or(o.target_rel.clone());
            let abs = paths::resolve_within(&root, Path::new(&rel))?;
            let md = match std::fs::symlink_metadata(&abs) {
                Ok(md) if md.is_file() && !paths::is_link(&md) => md,
                Ok(_) | Err(_) => {
                    if !root.is_dir() {
                        return Err(MfError::VolumeUnavailable(root));
                    }
                    self.fail(
                        i,
                        &MfError::VerificationFailed {
                            path: abs,
                            expected: "file present".into(),
                            actual: "missing".into(),
                        },
                    )?;
                    continue;
                }
            };
            if o.state == OpState::Done {
                let recorded: Option<FileStamp> = o
                    .final_stamp_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());
                let now = FileStamp::of(&md);
                if recorded.is_some_and(|r| r.size != now.size || r.mtime_ns != now.mtime_ns) {
                    self.fail(
                        i,
                        &MfError::VerificationFailed {
                            path: abs,
                            expected: "unchanged copy".into(),
                            actual: "modified".into(),
                        },
                    )?;
                    continue;
                }
                if paranoid {
                    let fid = o.file_id;
                    self.set_current(fid, FilePhase::Verifying, 0);
                    let before = self.progress.bytes_verified;
                    let mut acc = 0u64;
                    let res = hash_file(&abs, &self.control.clone(), |n| acc += n);
                    self.progress.bytes_verified = before + acc;
                    self.emit(false);
                    match res {
                        Ok((h, _)) if o.sha256.as_deref() == Some(h.as_str()) => {}
                        Ok((h, _)) => {
                            self.fail(
                                i,
                                &MfError::VerificationFailed {
                                    path: abs,
                                    expected: o.sha256.clone().unwrap_or_default(),
                                    actual: h.to_string(),
                                },
                            )?;
                        }
                        Err(e @ (MfError::Cancelled | MfError::Paused)) => return Err(e),
                        Err(e) => {
                            if !root.is_dir() {
                                return Err(MfError::VolumeUnavailable(root));
                            }
                            self.fail(i, &e)?;
                        }
                    }
                }
            }
        }
        self.progress.current = None;
        self.emit(true);
        Ok(())
    }
}
