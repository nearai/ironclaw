use thiserror::Error;

#[derive(Debug, Error)]
pub enum LearningError {
    #[error("Skill synthesis failed: {reason}")]
    SynthesisFailed { reason: String },

    #[error("Safety validation rejected skill '{skill_name}': {reason}")]
    SafetyRejected { skill_name: String, reason: String },

    #[error("Skill parse error: {0}")]
    ParseError(#[from] crate::skills::parser::SkillParseError),

    #[error("LLM error during synthesis: {0}")]
    LlmError(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::error::DatabaseError),

    #[error("Pattern detection failed: {reason}")]
    DetectionFailed { reason: String },
}
