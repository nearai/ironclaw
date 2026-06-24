//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.
//!
//! The local backend resolves every operation **fd-relative** to a mount-root
//! directory fd that is opened once during trusted setup. On Linux it uses a
//! single `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` syscall; on other
//! Unix platforms it performs an `openat(O_NOFOLLOW)` per-component walk. No
//! operation re-resolves an absolute host path or trusts `canonicalize`, so
//! containment within the mount root holds **by construction** — the
//! time-of-check/time-of-use window against a concurrently mutated, hostile
//! mount root is closed on every platform, not merely narrowed.
#![warn(unreachable_pub)]

mod backend;
mod catalog;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod db;
mod hsm;
mod in_memory;
mod index;
#[cfg(feature = "libsql")]
mod libsql;
mod local;
#[cfg(feature = "postgres")]
mod postgres;
mod record;
mod root;
mod scoped;
mod types;
mod vector;

pub use backend::{EventRecord, StorageTxn};
pub use catalog::{CompositeRootFilesystem, FilesystemCatalog, MountDescriptor, PathPlacement};
pub use hsm::HsmBackend;
pub use in_memory::InMemoryBackend;
pub use index::{Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, Page};
#[cfg(feature = "libsql")]
pub use libsql::LibSqlRootFilesystem;
pub use local::LocalFilesystem;
#[cfg(feature = "postgres")]
pub use postgres::PostgresRootFilesystem;
pub use record::{
    CasExpectation, ContentType, Entry, RecordKind, RecordVersion, SeqNo, VersionedEntry,
};
pub use root::RootFilesystem;
pub use scoped::{MountViewResolver, ScopedFilesystem};
pub use types::{
    BackendCapabilities, BackendId, BackendKind, Capability, ContentKind, DirEntry, FileStat,
    FileType, FilesystemError, FilesystemOperation, IndexConflictReason, IndexPolicy, StorageClass,
    TxnCapability,
};

fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    std::path::Path::new(path).starts_with(std::path::Path::new(prefix))
}

#[cfg(test)]
mod tests {
    use super::path_prefix_matches;

    #[test]
    fn path_prefix_matches_root_and_component_boundaries() {
        assert!(path_prefix_matches("/", "/projects"));
        assert!(path_prefix_matches("/projects", "/projects"));
        assert!(path_prefix_matches("/projects", "/projects/readme.md"));
        assert!(!path_prefix_matches("/projects", "/projects-private"));
    }
}
