use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FirstPartySkillsExtensionError {
    #[error(
        "invalid first-party skills extension handle {handle}: expected {expected}, got {actual}"
    )]
    InvalidHandle {
        handle: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("invalid first-party skills extension root path: {0}")]
    InvalidRootPath(String),
    #[error("invalid first-party skills extension bundle source: {0}")]
    InvalidBundleSource(String),
}
