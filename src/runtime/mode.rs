//! Mode — Runtime operating modes
//!
//! Leader mode (default): LLM autonomously decides the path — free ReAct for open
//! tasks, plan-first for complex tasks (discipline lives in the L2 constitution,
//! no per-message prompt injection).
//!
//! Workflow mode: Leader operates in workflow design mode — reads workflow-design skill,
//! analyzes user goals, decomposes into executable step sequences, outputs workflow.json.
//! Tool set switches to work_agent profile (file ops + desktop + skills + request_user_input).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Runtime operating mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Leader mode — LLM autonomously decides tool calls, default
    #[default]
    Leader,
    /// Workflow mode — Leader operates as workflow designer
    Workflow,
    /// Custom mode — user's own agent (L2 fully user-defined, L0/L1 locked)
    Custom,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Leader => "leader",
            Mode::Workflow => "workflow",
            Mode::Custom => "custom",
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "leader" | "l" => Ok(Mode::Leader),
            "workflow" | "wf" => Ok(Mode::Workflow),
            "custom" | "c" => Ok(Mode::Custom),
            other => Err(format!("unknown mode: {other}")),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_mode_from_str() {
        assert_eq!(Mode::from_str("leader").ok(), Some(Mode::Leader));
        assert_eq!(Mode::from_str("l").ok(), Some(Mode::Leader));
        assert_eq!(Mode::from_str("workflow").ok(), Some(Mode::Workflow));
        assert_eq!(Mode::from_str("wf").ok(), Some(Mode::Workflow));
        assert_eq!(Mode::from_str("unknown").ok(), None);
    }

    #[test]
    fn test_legacy_mode_strings_rejected() {
        // Legacy persisted "free"/"plan" values must parse to None so callers
        // fall back to the Leader default instead of crashing.
        assert_eq!(Mode::from_str("free").ok(), None);
        assert_eq!(Mode::from_str("plan").ok(), None);
    }

    #[test]
    fn test_mode_default_is_leader() {
        assert_eq!(Mode::default(), Mode::Leader);
        assert_eq!(Mode::default().as_str(), "leader");
    }
}
