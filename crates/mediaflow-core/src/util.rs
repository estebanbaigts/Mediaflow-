//! Small shared helpers: byte formatting, time, cooperative control.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::error::{MfError, Result};

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const CANCEL: u8 = 2;

/// Cooperative control shared between a running job and whoever drives it
/// (UI, CLI, tests). Workers call [`Control::checkpoint`] between chunks.
#[derive(Clone, Default)]
pub struct Control {
    inner: Arc<ControlInner>,
}

#[derive(Default)]
struct ControlInner {
    state: AtomicU8,
    /// When set, a pause request stops the session (state `paused`) instead of
    /// blocking the worker thread. Used by the engine so pause is durable.
    stop_on_pause: AtomicBool,
    lock: Mutex<()>,
    cv: Condvar,
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the worker to stop at the next checkpoint and leave the session
    /// resumable.
    pub fn pause(&self) {
        self.inner.state.store(PAUSE, Ordering::SeqCst);
        self.inner.cv.notify_all();
    }

    pub fn resume(&self) {
        let _ = self
            .inner
            .state
            .compare_exchange(PAUSE, RUN, Ordering::SeqCst, Ordering::SeqCst);
        self.inner.cv.notify_all();
    }

    pub fn cancel(&self) {
        self.inner.state.store(CANCEL, Ordering::SeqCst);
        self.inner.cv.notify_all();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.state.load(Ordering::SeqCst) == CANCEL
    }

    pub fn is_paused(&self) -> bool {
        self.inner.state.load(Ordering::SeqCst) == PAUSE
    }

    pub fn set_stop_on_pause(&self, v: bool) {
        self.inner.stop_on_pause.store(v, Ordering::SeqCst);
    }

    /// Returns `Err(Cancelled)` / `Err(Paused)` when the job must stop.
    /// If `stop_on_pause` is false, a pause blocks here until resumed.
    pub fn checkpoint(&self) -> Result<()> {
        loop {
            match self.inner.state.load(Ordering::SeqCst) {
                CANCEL => return Err(MfError::Cancelled),
                PAUSE => {
                    if self.inner.stop_on_pause.load(Ordering::SeqCst) {
                        return Err(MfError::Paused);
                    }
                    let g = self.inner.lock.lock().unwrap_or_else(|e| e.into_inner());
                    let _ = self
                        .inner
                        .cv
                        .wait_timeout(g, Duration::from_millis(200))
                        .unwrap_or_else(|e| e.into_inner());
                }
                _ => return Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_format() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500_000), "1.5 MB");
        assert_eq!(human_bytes(2_000_000_000), "2.0 GB");
    }

    #[test]
    fn control_cancel_and_pause() {
        let c = Control::new();
        assert!(c.checkpoint().is_ok());
        c.set_stop_on_pause(true);
        c.pause();
        assert!(matches!(c.checkpoint(), Err(MfError::Paused)));
        c.resume();
        assert!(c.checkpoint().is_ok());
        c.cancel();
        assert!(matches!(c.checkpoint(), Err(MfError::Cancelled)));
        c.resume(); // resume does not undo cancel
        assert!(c.is_cancelled());
    }
}
