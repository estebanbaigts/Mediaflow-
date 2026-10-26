//! Explicit state machines for import sessions and per-file operations.
//! Every state change in the engine goes through [`SessionState::apply`] /
//! [`OpState::apply`], so illegal transitions are impossible by construction.

use serde::{Deserialize, Serialize};

use crate::error::{MfError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Queued,
    Scanning,
    Planned,
    Copying,
    Verifying,
    /// Every planned operation finished and was verified (or verified-skipped).
    Completed,
    /// Finished, but at least one file failed, conflicted or was not copied.
    /// Never presented as a success.
    CompletedWithErrors,
    Paused,
    /// Stopped unexpectedly (crash, volume loss). Resumable.
    Interrupted,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEvent {
    StartScan,
    Planned,
    Start,
    CopyFinished,
    VerifyOk,
    VerifyWithErrors,
    Pause,
    Resume,
    Interrupt,
    Fail,
    Cancel,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Queued => "queued",
            SessionState::Scanning => "scanning",
            SessionState::Planned => "planned",
            SessionState::Copying => "copying",
            SessionState::Verifying => "verifying",
            SessionState::Completed => "completed",
            SessionState::CompletedWithErrors => "completed_with_errors",
            SessionState::Paused => "paused",
            SessionState::Interrupted => "interrupted",
            SessionState::Failed => "failed",
            SessionState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        use SessionState::*;
        [
            Queued,
            Scanning,
            Planned,
            Copying,
            Verifying,
            Completed,
            CompletedWithErrors,
            Paused,
            Interrupted,
            Failed,
            Cancelled,
        ]
        .into_iter()
        .find(|x| x.as_str() == s)
        .ok_or_else(|| MfError::InvalidInput(format!("unknown session state {s:?}")))
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SessionState::Completed | SessionState::CompletedWithErrors | SessionState::Cancelled
        )
    }

    /// States in which a worker may be running.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            SessionState::Scanning | SessionState::Copying | SessionState::Verifying
        )
    }

    /// `completed_with_errors` is resumable too: resuming retries the failed
    /// operations (conflicts stay flagged).
    pub fn is_resumable(self) -> bool {
        matches!(
            self,
            SessionState::Paused
                | SessionState::Interrupted
                | SessionState::Failed
                | SessionState::CompletedWithErrors
        )
    }

    pub fn apply(self, ev: SessionEvent) -> Result<SessionState> {
        use SessionEvent as E;
        use SessionState as S;
        let next = match (self, ev) {
            (S::Queued, E::StartScan) => S::Scanning,
            (S::Queued | S::Scanning, E::Planned) => S::Planned,
            (S::Planned, E::Start) => S::Copying,
            (S::Copying, E::CopyFinished) => S::Verifying,
            (S::Verifying, E::VerifyOk) => S::Completed,
            (S::Verifying, E::VerifyWithErrors) => S::CompletedWithErrors,
            (S::Copying | S::Verifying, E::Pause) => S::Paused,
            (S::Paused | S::Interrupted | S::Failed | S::CompletedWithErrors, E::Resume) => {
                S::Copying
            }
            (S::Scanning | S::Copying | S::Verifying, E::Interrupt) => S::Interrupted,
            (s, E::Fail) if !s.is_terminal() => S::Failed,
            (
                S::Queued
                | S::Planned
                | S::Copying
                | S::Verifying
                | S::Paused
                | S::Interrupted
                | S::Failed,
                E::Cancel,
            ) => S::Cancelled,
            _ => {
                return Err(MfError::InvalidTransition {
                    from: self.as_str().into(),
                    event: format!("{ev:?}"),
                })
            }
        };
        Ok(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpState {
    Pending,
    /// Data is being written to the temporary file.
    Copying,
    /// Temporary file complete; re-reading to verify.
    Verifying,
    /// Verified and finalized under its final name.
    Done,
    /// Identical content verified at destination; nothing written.
    SkippedDuplicate,
    /// Not copied because the target holds different content (profile: flag).
    Conflict,
    Failed,
    /// Session cancelled before this file finished.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpEvent {
    BeginCopy,
    CopyWritten,
    Verified,
    SkipVerifiedDuplicate,
    FlagConflict,
    Fail,
    Cancel,
    /// Back to pending (resume after interruption, retry after failure).
    Reset,
}

impl OpState {
    pub fn as_str(self) -> &'static str {
        match self {
            OpState::Pending => "pending",
            OpState::Copying => "copying",
            OpState::Verifying => "verifying",
            OpState::Done => "done",
            OpState::SkippedDuplicate => "skipped_duplicate",
            OpState::Conflict => "conflict",
            OpState::Failed => "failed",
            OpState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        use OpState::*;
        [
            Pending,
            Copying,
            Verifying,
            Done,
            SkippedDuplicate,
            Conflict,
            Failed,
            Cancelled,
        ]
        .into_iter()
        .find(|x| x.as_str() == s)
        .ok_or_else(|| MfError::InvalidInput(format!("unknown op state {s:?}")))
    }

    /// Finished successfully (file is safe at destination).
    pub fn is_success(self) -> bool {
        matches!(self, OpState::Done | OpState::SkippedDuplicate)
    }

    pub fn apply(self, ev: OpEvent) -> Result<OpState> {
        use OpEvent as E;
        use OpState as S;
        let next = match (self, ev) {
            (S::Pending, E::BeginCopy) => S::Copying,
            (S::Copying, E::CopyWritten) => S::Verifying,
            (S::Verifying, E::Verified) => S::Done,
            (S::Pending, E::SkipVerifiedDuplicate) => S::SkippedDuplicate,
            (S::Pending, E::FlagConflict) => S::Conflict,
            (S::Pending | S::Copying | S::Verifying, E::Fail) => S::Failed,
            // A finalized copy that fails re-verification becomes failed.
            (S::Done | S::SkippedDuplicate, E::Fail) => S::Failed,
            (S::Pending | S::Copying | S::Verifying, E::Cancel) => S::Cancelled,
            (
                S::Copying
                | S::Verifying
                | S::Failed
                | S::Cancelled
                | S::Conflict
                | S::Done
                | S::SkippedDuplicate,
                E::Reset,
            ) => S::Pending,
            _ => {
                return Err(MfError::InvalidTransition {
                    from: self.as_str().into(),
                    event: format!("{ev:?}"),
                })
            }
        };
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionEvent as E;
    use SessionState as S;

    #[test]
    fn happy_path() {
        let s = S::Queued
            .apply(E::StartScan)
            .and_then(|s| s.apply(E::Planned))
            .and_then(|s| s.apply(E::Start))
            .and_then(|s| s.apply(E::CopyFinished))
            .and_then(|s| s.apply(E::VerifyOk))
            .unwrap();
        assert_eq!(s, S::Completed);
        assert!(s.is_terminal());
    }

    #[test]
    fn terminal_states_are_final() {
        for t in [S::Completed, S::CompletedWithErrors, S::Cancelled] {
            for e in [E::Start, E::Pause, E::Fail, E::Cancel, E::Interrupt] {
                assert!(t.apply(e).is_err(), "{t:?} {e:?}");
            }
            assert_eq!(t.apply(E::Resume).is_ok(), t == S::CompletedWithErrors);
        }
    }

    #[test]
    fn cannot_complete_without_verifying() {
        assert!(S::Copying.apply(E::VerifyOk).is_err());
        assert!(S::Planned.apply(E::VerifyOk).is_err());
    }

    #[test]
    fn interruption_and_resume() {
        assert_eq!(S::Copying.apply(E::Interrupt).unwrap(), S::Interrupted);
        assert_eq!(S::Interrupted.apply(E::Resume).unwrap(), S::Copying);
        assert_eq!(S::Paused.apply(E::Resume).unwrap(), S::Copying);
        assert_eq!(S::Failed.apply(E::Resume).unwrap(), S::Copying);
        assert!(S::Planned.apply(E::Resume).is_err());
    }

    #[test]
    fn op_cannot_be_done_without_verification() {
        assert!(OpState::Pending.apply(OpEvent::Verified).is_err());
        assert!(OpState::Copying.apply(OpEvent::Verified).is_err());
        let s = OpState::Pending
            .apply(OpEvent::BeginCopy)
            .and_then(|s| s.apply(OpEvent::CopyWritten))
            .and_then(|s| s.apply(OpEvent::Verified))
            .unwrap();
        assert!(s.is_success());
    }

    #[test]
    fn string_roundtrip() {
        for s in [
            S::Queued,
            S::Completed,
            S::CompletedWithErrors,
            S::Interrupted,
        ] {
            assert_eq!(S::parse(s.as_str()).unwrap(), s);
        }
        for s in [OpState::Pending, OpState::SkippedDuplicate, OpState::Failed] {
            assert_eq!(OpState::parse(s.as_str()).unwrap(), s);
        }
    }
}
