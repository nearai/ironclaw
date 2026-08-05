use ironclaw_filesystem::FilesystemError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReleasePairMigrationError {
    #[error("filesystem error: {0}")]
    Filesystem(#[from] FilesystemError),
    #[error("release-pair migration record is malformed: {0}")]
    Malformed(String),
    #[error("release-pair migration is already running in another process")]
    ConcurrentStartup,
    #[error("release-pair migration source/target does not match this binary")]
    UnsupportedReleasePair,
    #[error("release-pair migration lost its database-wide lease")]
    LostLease,
    #[error("{domain} startup migration failed: {reason}")]
    Domain {
        domain: &'static str,
        reason: String,
    },
}
