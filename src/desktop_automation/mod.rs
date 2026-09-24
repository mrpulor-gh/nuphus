//! Platform-independent semantic desktop automation core.
//!
//! This module intentionally contains no platform handles or coordinates.
//! Platform adapters produce observations and execute locally validated
//! candidates; decision providers may only return a candidate id.

mod jev;
mod laya;
pub(crate) mod macos_accessibility;
mod outcome;
#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod risk;
mod runner;
mod types;
mod windows_uia;

pub use jev::{
    JevClient, JevError, ReqwestSystemOneTransport, SystemOneRequest, SystemOneResponse,
    SystemOneTransport,
};
pub use laya::{
    LayaChoiceAnswer, LayaChoiceQuestion, LayaClient, LayaConfig, LayaError, LayaRequest,
    LayaResponse, LayaTransport, LayaUsage, ReqwestLayaTransport,
};
pub use macos_accessibility::MacosAccessibilityAdapter;
pub use outcome::{ActionEffect, DesktopActionError, DispatchState};
#[cfg(any(target_os = "windows", target_os = "macos", test))]
pub(crate) use risk::classify_desktop_risk;
pub use runner::{
    AutomationRunner, CandidateBuilder, ComputerExecutor, ComputerObserver, Verifier,
};
pub use types::*;
pub use windows_uia::WindowsUiaAdapter;
