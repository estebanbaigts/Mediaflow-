//! Compact, read-only status snapshot for system surfaces (menu bar, widgets).
//!
//! The desktop app writes it to a small JSON file that a sandboxed macOS
//! widget can read. It contains no file contents, only what the overview
//! already shows. Format version 1 is documented in `docs/WIDGET.md`.

use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::Engine;
use crate::error::{MfError, Result};
use crate::progress::Progress;
use crate::state::SessionState;

pub const STATUS_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurrentImport {
    pub session_id: String,
    pub project: String,
    pub source_label: String,
    pub state: SessionState,
    pub fraction: f64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub eta_seconds: Option<f64>,
    pub current_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastImport {
    pub session_id: String,
    pub project: String,
    pub source_label: String,
    pub state: SessionState,
    /// True only for `completed`.
    pub verified_ok: bool,
    pub finished_at: Option<DateTime<Utc>>,
    pub files_done: usize,
    pub bytes_done: u64,
    pub problems: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DestinationStatus {
    pub label: String,
    pub available: bool,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusSnapshot {
    pub version: u32,
    pub updated_at: DateTime<Utc>,
    pub current: Option<CurrentImport>,
    pub last: Option<LastImport>,
    /// Imports that are interrupted, paused, failed or finished with problems.
    pub attention: usize,
    pub destinations: Vec<DestinationStatus>,
}

fn needs_attention(s: SessionState) -> bool {
    matches!(
        s,
        SessionState::Interrupted
            | SessionState::Paused
            | SessionState::Failed
            | SessionState::CompletedWithErrors
    )
}

impl Engine {
    /// Build a snapshot. `live` is the latest progress of a running import.
    pub fn status_snapshot(&self, live: Option<&Progress>) -> Result<StatusSnapshot> {
        let sessions = self.sessions(200)?;
        let current = live.and_then(|p| {
            let s = sessions.iter().find(|s| s.id == p.session_id)?;
            Some(CurrentImport {
                session_id: s.id.clone(),
                project: s.project.clone(),
                source_label: s.source_label.clone(),
                state: p.state.unwrap_or(s.state),
                fraction: p.fraction.clamp(0.0, 1.0),
                bytes_done: p.bytes_copied + p.bytes_verified,
                bytes_total: p.bytes_to_copy + p.bytes_to_verify,
                eta_seconds: p.eta_seconds,
                current_file: p.current.as_ref().map(|c| c.rel_path.clone()),
            })
        });
        let last = sessions
            .iter()
            .filter(|s| current.as_ref().is_none_or(|c| c.session_id != s.id))
            .find(|s| s.state != SessionState::Planned)
            .map(|s| LastImport {
                session_id: s.id.clone(),
                project: s.project.clone(),
                source_label: s.source_label.clone(),
                state: s.state,
                verified_ok: s.state == SessionState::Completed,
                finished_at: s.finished_at,
                files_done: s.counts.done + s.counts.skipped_duplicate,
                bytes_done: s.counts.bytes_done,
                problems: s.counts.failed + s.counts.conflict + s.counts.pending,
            });
        let attention = sessions
            .iter()
            .filter(|s| {
                needs_attention(s.state) && current.as_ref().is_none_or(|c| c.session_id != s.id)
            })
            .count();
        let destinations = self
            .grants()?
            .into_iter()
            .filter(|g| g.kind == crate::db::GrantKind::Destination)
            .map(|g| {
                let p = Path::new(&g.path);
                DestinationStatus {
                    label: g.label,
                    available: p.is_dir(),
                    free_bytes: crate::volumes::available_space(p),
                }
            })
            .collect();
        Ok(StatusSnapshot {
            version: STATUS_VERSION,
            updated_at: crate::util::now(),
            current,
            last,
            attention,
            destinations,
        })
    }
}

impl StatusSnapshot {
    /// Write atomically (temp file in the same folder, then rename over the
    /// previous snapshot). This file belongs to MediaFlow; replacing it is
    /// the only overwrite the app performs, and only of its own file.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        let dir = path
            .parent()
            .ok_or_else(|| MfError::InvalidInput("status path has no folder".into()))?;
        std::fs::create_dir_all(dir).map_err(|e| MfError::io(dir, e))?;
        let tmp = dir.join(format!(".status-{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp).map_err(|e| MfError::io(&tmp, e))?;
            f.write_all(serde_json::to_string_pretty(self)?.as_bytes())
                .map_err(|e| MfError::io(&tmp, e))?;
            f.sync_all().map_err(|e| MfError::io(&tmp, e))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| MfError::io(path, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_engine_snapshot_roundtrips() {
        let e = Engine::open_in_memory().unwrap();
        let s = e.status_snapshot(None).unwrap();
        assert_eq!(s.version, STATUS_VERSION);
        assert!(s.current.is_none() && s.last.is_none());
        assert_eq!(s.attention, 0);
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nested").join("status.json");
        s.write_atomic(&p).unwrap();
        s.write_atomic(&p).unwrap(); // replacing our own snapshot works
        let back: StatusSnapshot =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(
            std::fs::read_dir(p.parent().unwrap()).unwrap().count(),
            1,
            "no temp file left"
        );
    }
}

#[cfg(test)]
mod contract {
    use super::*;

    /// The same fixture is decoded by the Swift widget's tests
    /// (app/macos-widget/StatusKit). Changing the format means bumping
    /// STATUS_VERSION and updating both sides.
    #[test]
    fn shared_fixture_matches_rust_model() {
        let raw = include_str!("../../../app/macos-widget/Fixtures/status-v1.json");
        let s: StatusSnapshot =
            serde_json::from_str(raw).expect("fixture must match StatusSnapshot");
        assert_eq!(s.version, STATUS_VERSION);
        let c = s.current.as_ref().unwrap();
        assert_eq!(c.state, SessionState::Copying);
        assert!(s.last.as_ref().unwrap().verified_ok);
        // And what Rust writes round-trips to the same structure.
        let again: StatusSnapshot =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(again, s);
    }
}
