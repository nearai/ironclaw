//! Adaptive learning system for IronClaw.
//!
//! Enables the agent to autonomously synthesize reusable skills from
//! successful complex interactions, with safety guarantees enforced
//! through safety layer scanning and skill validation.

pub mod candidate;
pub mod detector;
pub mod error;
pub mod synthesizer;
pub mod validator;
pub mod worker;

pub use candidate::{DetectionReason, SynthesisCandidate};
pub use error::LearningError;

/// Event sent to the learning background worker after each qualifying turn.
#[derive(Debug, Clone)]
pub struct LearningEvent {
    pub user_id: String,
    pub agent_id: String,
    pub conversation_id: uuid::Uuid,
    pub tools_used: Vec<String>,
    pub turn_count: usize,
    pub quality_score: u32,
    pub user_messages: Vec<String>,
    pub user_requested_synthesis: bool,
}
