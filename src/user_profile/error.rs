use thiserror::Error;

#[derive(Debug, Error)]
pub enum UserProfileError {
    #[error("Encryption error: {reason}")]
    EncryptionError { reason: String },

    #[error("Decryption error: {reason}")]
    DecryptionError { reason: String },

    #[error("Profile fact rejected by safety scan: {reason}")]
    SafetyRejected { reason: String },

    #[error("Database error: {0}")]
    DatabaseError(#[from] crate::error::DatabaseError),

    #[error("LLM error during distillation: {0}")]
    LlmError(String),
}
