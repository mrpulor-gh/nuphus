//! Platform-independent semantic desktop automation core.
//!
//! This module intentionally contains no platform handles or coordinates.
//! Platform adapters produce observations and execute locally validated
//! candidates; decision providers may only return a candidate id.

mod types;
mod windows_uia;

use async_trait::async_trait;

pub use types::*;
pub use windows_uia::WindowsUiaAdapter;

#[async_trait]
pub trait ComputerObserver: Send + Sync {
    fn capabilities(&self) -> PlatformCapabilities;
    async fn observe(&self, scope: &ObservationScope) -> Result<Observation, AutomationError>;
}

pub trait CandidateBuilder: Send + Sync {
    fn build(
        &self,
        goal: &str,
        observation: &Observation,
    ) -> Result<Vec<ActionCandidate>, AutomationError>;

    fn semantic_locator(&self, _candidate: &ActionCandidate) -> Option<SemanticLocator> {
        None
    }

    fn rebuild_semantic_candidate(
        &self,
        _locator: &SemanticLocator,
        _action: NativeAction,
        _observation: &Observation,
    ) -> Result<ActionCandidate, AutomationError> {
        Err(AutomationError::Candidates(
            "persistent semantic actions are unsupported by this adapter".into(),
        ))
    }
}

#[async_trait]
pub trait ComputerExecutor: Send + Sync {
    async fn execute(
        &self,
        fresh: &Observation,
        action: &ActionCandidate,
        input: &ExecutionInput,
    ) -> Result<ActionReceipt, AutomationError>;
}
