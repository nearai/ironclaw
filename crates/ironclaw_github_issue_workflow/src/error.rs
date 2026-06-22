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
}
