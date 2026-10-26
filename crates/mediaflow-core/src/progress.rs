//! Progress snapshots derived from work actually performed (bytes read,
//! written and re-read), never from timers.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::state::{OpState, SessionState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentFile {
    pub file_id: i64,
    pub rel_path: String,
    pub size: u64,
    pub phase: FilePhase,
    pub done: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilePhase {
    Checking,
    Copying,
    Verifying,
    Finalizing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileOutcome {
    pub file_id: i64,
    pub dest_idx: i64,
    pub rel_path: String,
    pub state: OpState,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub session_id: String,
    pub state: Option<SessionState>,
    pub ops_total: usize,
    pub ops_done: usize,
    pub ops_failed: usize,
    pub ops_skipped: usize,
    pub bytes_to_copy: u64,
    pub bytes_copied: u64,
    pub bytes_to_verify: u64,
    pub bytes_verified: u64,
    /// 0..=1, from bytes copied + bytes verified.
    pub fraction: f64,
    pub bytes_per_second: Option<f64>,
    /// Only present when a stable rate has been measured.
    pub eta_seconds: Option<f64>,
    pub current: Option<CurrentFile>,
    pub last_outcome: Option<FileOutcome>,
}

/// Sliding-window throughput meter.
pub struct RateMeter {
    window: VecDeque<(Instant, u64)>,
    span: Duration,
    started: Instant,
}

impl Default for RateMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateMeter {
    pub fn new() -> Self {
        RateMeter {
            window: VecDeque::new(),
            span: Duration::from_secs(5),
            started: Instant::now(),
        }
    }

    pub fn record(&mut self, total_bytes: u64) {
        let now = Instant::now();
        self.window.push_back((now, total_bytes));
        while let Some((t, _)) = self.window.front() {
            if now.duration_since(*t) > self.span && self.window.len() > 2 {
                self.window.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn rate(&self) -> Option<f64> {
        if self.started.elapsed() < Duration::from_secs(2) {
            return None;
        }
        let (t0, b0) = self.window.front()?;
        let (t1, b1) = self.window.back()?;
        let dt = t1.duration_since(*t0).as_secs_f64();
        if dt < 0.5 || b1 <= b0 {
            return None;
        }
        Some((b1 - b0) as f64 / dt)
    }
}

impl Progress {
    pub fn recompute(&mut self, meter: &RateMeter) {
        let total = self.bytes_to_copy + self.bytes_to_verify;
        let done = self.bytes_copied + self.bytes_verified;
        self.fraction = if total == 0 {
            if self.ops_total == 0 {
                1.0
            } else {
                (self.ops_done + self.ops_failed + self.ops_skipped) as f64 / self.ops_total as f64
            }
        } else {
            (done as f64 / total as f64).min(1.0)
        };
        self.bytes_per_second = meter.rate();
        self.eta_seconds = self
            .bytes_per_second
            .filter(|r| *r > 0.0)
            .map(|r| total.saturating_sub(done) as f64 / r);
    }
}
