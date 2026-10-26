//! Session reports (JSON / CSV). Built entirely from the local journal.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::EventRow;
use crate::engine::Engine;
use crate::error::{MfError, Result};
use crate::scan::ScanIssue;
use crate::state::{OpState, SessionState};

pub const REPORT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportFile {
    pub source_rel: String,
    pub size: u64,
    pub category: String,
    pub type_confidence: String,
    pub destination: String,
    pub target_rel: Option<String>,
    pub planned_action: String,
    pub state: OpState,
    pub sha256: Option<String>,
    pub error_code: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportExcluded {
    pub source_rel: String,
    pub size: u64,
    pub pattern: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReportTotals {
    pub files_scanned: usize,
    pub files_excluded: usize,
    pub operations: usize,
    pub copied_and_verified: usize,
    pub duplicates_skipped_verified: usize,
    pub conflicts_not_copied: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub pending: usize,
    pub bytes_scanned: u64,
    pub bytes_copied: u64,
    pub scan_issues: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionReport {
    pub report_version: u32,
    pub generated_at: DateTime<Utc>,
    pub generator: String,
    pub session_id: String,
    pub state: SessionState,
    /// True only for `completed`. Anything else needs attention.
    pub success: bool,
    pub project: String,
    pub profile: String,
    pub source_root: String,
    pub source_label: String,
    pub destinations: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub active_seconds: f64,
    pub totals: ReportTotals,
    pub limitations: Vec<String>,
    pub files: Vec<ReportFile>,
    pub excluded: Vec<ReportExcluded>,
    pub scan_issues: Vec<ScanIssue>,
    pub events: Vec<EventRow>,
}

impl Engine {
    pub fn report(&self, id: &str) -> Result<SessionReport> {
        let db = self.db();
        let s = db.get_session(id)?;
        let dests = db.destinations(id)?;
        let files = db.files(id)?;
        let ops = db.ops(id)?;
        let events = db.events(id)?;
        drop(db);
        let scan_issues: Vec<ScanIssue> =
            serde_json::from_str(&s.scan_issues_json).unwrap_or_default();
        let profile = serde_json::from_str::<crate::profile::Profile>(&s.profile_json).ok();

        let mut t = ReportTotals {
            files_scanned: files.len(),
            operations: ops.len(),
            scan_issues: scan_issues.len(),
            bytes_scanned: files.iter().map(|f| f.size).sum(),
            ..Default::default()
        };
        let mut out = Vec::new();
        for o in &ops {
            let f = &files[o.file_id as usize];
            match o.state {
                OpState::Done => {
                    t.copied_and_verified += 1;
                    t.bytes_copied += o.bytes;
                }
                OpState::SkippedDuplicate => t.duplicates_skipped_verified += 1,
                OpState::Conflict => t.conflicts_not_copied += 1,
                OpState::Failed => t.failed += 1,
                OpState::Cancelled => t.cancelled += 1,
                OpState::Pending | OpState::Copying | OpState::Verifying => t.pending += 1,
            }
            out.push(ReportFile {
                source_rel: f.rel_path.clone(),
                size: f.size,
                category: f.category.clone(),
                type_confidence: f.confidence.clone(),
                destination: dests
                    .iter()
                    .find(|d| d.idx == o.dest_idx)
                    .map(|d| d.root.clone())
                    .unwrap_or_default(),
                target_rel: o.final_rel.clone().or(Some(o.target_rel.clone())),
                planned_action: o.action.clone(),
                state: o.state,
                sha256: o.sha256.clone(),
                error_code: o.error_code.clone(),
                error: o.error_message.clone(),
            });
        }
        let excluded: Vec<ReportExcluded> = files
            .iter()
            .filter_map(|f| {
                Some(ReportExcluded {
                    source_rel: f.rel_path.clone(),
                    size: f.size,
                    pattern: f.excluded_pattern.clone()?,
                    reason: f.excluded_reason.clone().unwrap_or_default(),
                })
            })
            .collect();
        t.files_excluded = excluded.len();

        let mut limitations = vec![
            "Verification re-reads each copy after writing it; the operating system may serve that read from its cache, so it proves what the OS returns, not necessarily the physical medium. Use the Paranoid level or a later completeness check after reconnecting the drive for an independent read.".to_string(),
        ];
        if !s.source_scan_complete {
            limitations
                .push("Some source folders could not be read: the inventory is incomplete.".into());
        }
        if !scan_issues.is_empty() {
            limitations.push(format!("{} source item(s) were not imported (symbolic links, special or unreadable files); see scan_issues.", scan_issues.len()));
        }
        if profile
            .as_ref()
            .is_some_and(|p| p.verify == crate::profile::VerifyLevel::Full)
        {
            limitations.push(
                "Verify level Full: copies were verified when written; no final re-read pass."
                    .into(),
            );
        }

        Ok(SessionReport {
            report_version: REPORT_VERSION,
            generated_at: crate::util::now(),
            generator: format!("MediaFlow {}", env!("CARGO_PKG_VERSION")),
            session_id: s.id,
            success: s.state == SessionState::Completed,
            state: s.state,
            project: s.project,
            profile: profile.map(|p| p.name).unwrap_or_default(),
            source_root: s.source_root,
            source_label: s.source_label,
            destinations: dests.into_iter().map(|d| d.root).collect(),
            created_at: s.created_at,
            started_at: s.started_at,
            finished_at: s.finished_at,
            active_seconds: s.active_seconds,
            totals: t,
            limitations,
            files: out,
            excluded,
            scan_issues,
            events,
        })
    }
}

impl SessionReport {
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// One row per (file, destination) operation, then excluded files.
    pub fn to_csv(&self) -> Result<String> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record([
            "source_path",
            "size_bytes",
            "category",
            "type_confidence",
            "destination",
            "target_path",
            "planned_action",
            "result",
            "sha256",
            "error_code",
            "error",
        ])?;
        for f in &self.files {
            w.write_record([
                csv_safe(&f.source_rel),
                f.size.to_string(),
                f.category.clone(),
                f.type_confidence.clone(),
                csv_safe(&f.destination),
                csv_safe(f.target_rel.as_deref().unwrap_or("")),
                f.planned_action.clone(),
                f.state.as_str().to_string(),
                f.sha256.clone().unwrap_or_default(),
                f.error_code.clone().unwrap_or_default(),
                csv_safe(f.error.as_deref().unwrap_or("")),
            ])?;
        }
        for e in &self.excluded {
            w.write_record([
                csv_safe(&e.source_rel),
                e.size.to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                "excluded".into(),
                "excluded".into(),
                String::new(),
                "excluded".into(),
                csv_safe(&format!("{} ({})", e.reason, e.pattern)),
            ])?;
        }
        let bytes = w
            .into_inner()
            .map_err(|e| MfError::InvalidInput(e.to_string()))?;
        String::from_utf8(bytes).map_err(|e| MfError::InvalidInput(e.to_string()))
    }

    pub fn write(&self, path: &Path, format: ReportFormat) -> Result<()> {
        let body = match format {
            ReportFormat::Json => self.to_json()?,
            ReportFormat::Csv => self.to_csv()?,
        };
        // Exclusive creation: an export never overwrites an existing file.
        let mut f = crate::fsops::create_new(path).map_err(|e| MfError::io(path, e))?;
        std::io::Write::write_all(&mut f, body.as_bytes()).map_err(|e| MfError::io(path, e))?;
        f.sync_all().map_err(|e| MfError::io(path, e))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Json,
    Csv,
}

/// Neutralize spreadsheet formula injection: file names are attacker-controlled
/// (anything on a card), and a CSV opened in a spreadsheet may evaluate cells
/// starting with = + - @.
pub fn csv_safe(s: &str) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_injection_neutralized() {
        assert_eq!(csv_safe("=HYPERLINK(\"x\")"), "'=HYPERLINK(\"x\")");
        assert_eq!(csv_safe("clip.mov"), "clip.mov");
        assert_eq!(csv_safe("-x"), "'-x");
    }
}
