//! Error type shared by the whole engine.
//!
//! Every variant carries enough context to produce a message a non-technical
//! user can act on (`user_message`) and a stable machine code (`code`) that the
//! UI and reports use. Messages never contain file *content*.

use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, MfError>;

#[derive(Debug, thiserror::Error)]
pub enum MfError {
    #[error("path is not allowed: {path} ({reason})")]
    PathRejected { path: String, reason: &'static str },

    #[error("access outside authorized folders refused: {0}")]
    NotAuthorized(PathBuf),

    #[error("source changed since it was scanned: {0}")]
    SourceChanged(PathBuf),

    #[error("destination already contains a different file: {0}")]
    Conflict(PathBuf),

    #[error("verification failed for {path}: expected {expected}, got {actual}")]
    VerificationFailed {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error(
        "not enough free space on destination {path}: need {needed} bytes, {available} available"
    )]
    InsufficientSpace {
        path: PathBuf,
        needed: u64,
        available: u64,
    },

    #[error("volume unavailable: {0}")]
    VolumeUnavailable(PathBuf),

    #[error("operation cancelled")]
    Cancelled,

    #[error("operation paused")]
    Paused,

    /// Test-only fault injection: the executor stops immediately without
    /// any cleanup or journal update, as if the process had been killed.
    #[doc(hidden)]
    #[error("simulated crash")]
    SimulatedCrash,

    #[error("another MediaFlow instance is using this library: {0}")]
    Locked(PathBuf),

    #[error("invalid state transition from {from} on {event}")]
    InvalidTransition { from: String, event: String },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
}

impl MfError {
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        MfError::Io {
            path: path.into(),
            source,
        }
    }

    /// Stable code for UI, reports and tests.
    pub fn code(&self) -> &'static str {
        match self {
            MfError::PathRejected { .. } => "path_rejected",
            MfError::NotAuthorized(_) => "not_authorized",
            MfError::SourceChanged(_) => "source_changed",
            MfError::Conflict(_) => "conflict",
            MfError::VerificationFailed { .. } => "verification_failed",
            MfError::InsufficientSpace { .. } => "insufficient_space",
            MfError::VolumeUnavailable(_) => "volume_unavailable",
            MfError::Cancelled => "cancelled",
            MfError::Paused => "paused",
            MfError::SimulatedCrash => "simulated_crash",
            MfError::Locked(_) => "locked",
            MfError::InvalidTransition { .. } => "invalid_transition",
            MfError::NotFound(_) => "not_found",
            MfError::InvalidInput(_) => "invalid_input",
            MfError::Io { source, .. } => match classify_io(source) {
                IoClass::PermissionDenied => "permission_denied",
                IoClass::StorageFull => "storage_full",
                IoClass::NotFound => "io_not_found",
                IoClass::Other => "io_error",
            },
            MfError::Db(_) => "database_error",
            MfError::Serde(_) | MfError::Csv(_) => "serialization_error",
        }
    }

    /// A sentence a non-technical user can understand and act on.
    pub fn user_message(&self) -> String {
        match self {
            MfError::PathRejected { path, reason } => {
                format!("The path \"{path}\" cannot be used: {reason}.")
            }
            MfError::NotAuthorized(p) => format!(
                "MediaFlow is not allowed to access \"{}\". Add this folder as a source or destination first.",
                p.display()
            ),
            MfError::SourceChanged(p) => format!(
                "\"{}\" changed after it was scanned. It was not copied. Scan the source again.",
                p.display()
            ),
            MfError::Conflict(p) => format!(
                "A different file already exists at \"{}\". Nothing was overwritten.",
                p.display()
            ),
            MfError::VerificationFailed { path, .. } => format!(
                "The copy of \"{}\" does not match the original. The copy was discarded; the original is untouched.",
                path.display()
            ),
            MfError::InsufficientSpace { path, needed, available } => format!(
                "Not enough space on \"{}\": {} needed, {} available.",
                path.display(),
                crate::util::human_bytes(*needed),
                crate::util::human_bytes(*available)
            ),
            MfError::VolumeUnavailable(p) => format!(
                "\"{}\" is no longer available. Reconnect the drive, then resume the import.",
                p.display()
            ),
            MfError::Cancelled => "The operation was cancelled.".into(),
            MfError::Paused => "The operation is paused.".into(),
            MfError::SimulatedCrash => "Simulated crash (test).".into(),
            MfError::Locked(_) => "MediaFlow is already open in another window or process. Close it first.".into(),
            MfError::InvalidTransition { .. } => {
                "This action is not possible in the current state of the import.".into()
            }
            MfError::NotFound(what) => format!("Not found: {what}."),
            MfError::InvalidInput(what) => format!("Invalid input: {what}."),
            MfError::Io { path, source } => match classify_io(source) {
                IoClass::PermissionDenied => format!(
                    "Permission denied for \"{}\". Check the folder permissions.",
                    path.display()
                ),
                IoClass::StorageFull => format!("The drive holding \"{}\" is full.", path.display()),
                IoClass::NotFound => format!("\"{}\" could not be found.", path.display()),
                IoClass::Other => format!(
                    "A read or write error occurred on \"{}\" ({source}).",
                    path.display()
                ),
            },
            MfError::Db(_) => "The local MediaFlow database reported an error.".into(),
            MfError::Serde(_) | MfError::Csv(_) => "A report could not be written.".into(),
        }
    }

    /// True when the error indicates the volume itself went away, which should
    /// pause/interrupt the session rather than fail individual files.
    pub fn is_volume_loss(&self) -> bool {
        matches!(self, MfError::VolumeUnavailable(_))
    }
}

pub(crate) enum IoClass {
    PermissionDenied,
    StorageFull,
    NotFound,
    Other,
}

pub(crate) fn classify_io(e: &io::Error) -> IoClass {
    match e.kind() {
        io::ErrorKind::PermissionDenied => IoClass::PermissionDenied,
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => IoClass::StorageFull,
        io::ErrorKind::NotFound => IoClass::NotFound,
        _ => {
            #[cfg(unix)]
            if e.raw_os_error() == Some(libc::ENOSPC) {
                return IoClass::StorageFull;
            }
            IoClass::Other
        }
    }
}
