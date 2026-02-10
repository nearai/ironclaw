//! Error types for NEAR key management.

use crate::secrets::SecretError;

/// Errors from NEAR key operations.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error("Key not found: {label}")]
    NotFound { label: String },

    #[error("Key already exists: {label}")]
    AlreadyExists { label: String },

    #[error("Invalid key format: {reason}")]
    InvalidKeyFormat { reason: String },

    #[error("Invalid account ID: {reason}")]
    InvalidAccountId { reason: String },

    #[error("Signing failed: {reason}")]
    SigningFailed { reason: String },

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Approval required: {operation}")]
    ApprovalRequired { operation: String },

    #[error("Policy denied: {reason}")]
    PolicyDenied { reason: String },

    #[error("RPC error: {reason}")]
    RpcError { reason: String },

    #[error("Stale nonce: cached {cached}, chain {chain}")]
    StaleNonce { cached: u64, chain: u64 },

    #[error("Insufficient allowance: needed {needed}, available {available}")]
    InsufficientAllowance { needed: u128, available: u128 },

    #[error("Permission denied: {reason}")]
    PermissionDenied { reason: String },

    #[error("Chain signature error: {reason}")]
    ChainSignatureError { reason: String },

    #[error("Backup error: {reason}")]
    BackupError { reason: String },

    #[error("Secret store error: {0}")]
    SecretStore(#[from] SecretError),

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
