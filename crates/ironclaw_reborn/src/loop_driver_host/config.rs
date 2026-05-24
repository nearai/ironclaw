use std::{error::Error, fmt};

use ironclaw_turns::{run_profile::LoopRunContext, runner::ClaimedTurnRun};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOnlyLoopHostConfig {
    pub max_messages: usize,
    pub require_model_route_snapshot: bool,
}

impl Default for TextOnlyLoopHostConfig {
    fn default() -> Self {
        Self {
            max_messages: 16,
            require_model_route_snapshot: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornLoopDriverHostError {
    ScopeMismatch { reason: String },
    InvalidRequest { reason: String },
}

impl fmt::Display for RebornLoopDriverHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScopeMismatch { reason } => {
                write!(formatter, "loop driver host scope mismatch: {reason}")
            }
            Self::InvalidRequest { reason } => {
                write!(formatter, "invalid loop driver host request: {reason}")
            }
        }
    }
}

impl Error for RebornLoopDriverHostError {}

#[derive(Debug, Clone)]
pub struct RebornLoopDriverHostRequest {
    pub claimed_run: ClaimedTurnRun,
    pub loop_run_context: LoopRunContext,
}
