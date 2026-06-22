use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GithubIssueWorkflowError {
    #[error("invalid {kind} id `{value}`: {reason}")]
    InvalidId {
        kind: &'static str,
        value: String,
        reason: &'static str,
    },

    #[error("invalid GitHub issue workflow config: {reason}")]
    InvalidConfig { reason: String },

    #[error("GitHub issue workflow policy denied the operation: {reason}")]
    PolicyDenied { reason: String },

    #[error("GitHub issue workflow policy error: {reason}")]
    Policy { reason: String },

    #[error("GitHub issue workflow repository error: {reason}")]
    Repository { reason: String },
}
