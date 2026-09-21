//! Platform-independent semantic desktop automation core.
//!
//! This module intentionally contains no platform handles or coordinates.
//! Platform adapters produce observations and execute locally validated
//! candidates; decision providers may only return a candidate id.

mod jev;
mod runner;
mod types;
mod windows_uia;

pub use jev::{
    JevClient, JevError, ReqwestSystemOneTransport, SystemOneRequest, SystemOneResponse,
    SystemOneTransport,
};
pub use runner::{
    AutomationRunner, CandidateBuilder, ComputerExecutor, ComputerObserver, Verifier,
};
pub use types::*;
pub use windows_uia::WindowsUiaAdapter;
