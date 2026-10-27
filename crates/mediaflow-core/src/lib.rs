//! MediaFlow engine.
//!
//! UI-independent core: path safety, scanning, classification, planning,
//! verified copy with a durable journal, completeness checks and reports.
//! Nothing in this crate deletes or overwrites user files: the only files it
//! ever removes are its own `.mfpart-*` temporary files.

pub mod classify;
pub mod compare;
pub mod db;
pub mod engine;
pub mod error;
pub mod executor;
pub mod fsops;
pub mod hash;
pub mod matching;
pub mod paths;
pub mod plan;
pub mod profile;
pub mod progress;
pub mod report;
pub mod scan;
pub mod state;
pub mod status;
pub mod template;
pub mod util;
pub mod volumes;

pub use engine::{Engine, PlanRequest, SessionInfo};
pub use error::{MfError, Result};
pub use executor::RunOptions;
pub use util::Control;
