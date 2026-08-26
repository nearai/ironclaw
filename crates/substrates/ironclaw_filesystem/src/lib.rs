//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.
//!
//! The local backend retains a directory capability for each mounted host root.
//! `write_file`, `append_file`, `create_dir_all`, and `create_subtree_atomic`
//! create child directories and temporary material relative to that retained
//! handle, so replacing the ambient mount path or an ancestor after admission
//! cannot redirect those writes. Reads resolve through the same retained handle
//! and reject symlink components, and every error keeps the virtual-path-only
//! boundary.
#![warn(unreachable_pub)]

mod backend;
mod cas;
mod catalog;
mod db;
#[cfg(feature = "test-support")]
mod fault;
mod hsm;
mod in_memory;
mod index;
mod libsql;
mod local;
mod local_capability;
mod ordinary_tree;
mod postgres;
#[cfg(feature = "test-support")]
mod postgres_isolation;
mod record;
mod root;
mod scoped;
mod types;
mod vector;

pub use backend::{EventRecord, StorageTxn};
pub use cas::{
    CasApply, CasUpdateError, FILESYSTEM_APPLY_TIMEOUT, FILESYSTEM_CAS_BACKOFF_BASE,
    FILESYSTEM_CAS_BACKOFF_MAX, FILESYSTEM_CAS_RETRIES, cas_update,
};
pub use catalog::{CompositeRootFilesystem, MountDescriptor, PathPlacement};
#[cfg(feature = "test-support")]
pub use fault::{Fault, FaultInjecting, FaultKind, RecordedOp};
pub use hsm::HsmBackend;
pub use in_memory::InMemoryBackend;
pub use index::{
    Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue, OrderedPage, OrderedQueryCursor,
    Page, SortDirection,
};
pub use libsql::LibSqlRootFilesystem;
pub use local::DiskFilesystem;
pub use local_capability::DiskDirectoryCapability;
pub use ordinary_tree::{
    MAX_ORDINARY_HOST_TREE_DEPTH, inspect_ordinary_host_tree, read_ordinary_host_file,
};
pub use postgres::{PostgresConnectionPool, PostgresRootFilesystem};
#[cfg(feature = "test-support")]
pub use postgres_isolation::{
    IsolatedPostgresDatabase, IsolatedPostgresProvisioner, PostgresUnreachable,
};
pub use record::{
    CasExpectation, ContentType, Entry, RecordKind, RecordVersion, SeqNo, VersionedEntry,
};
pub use root::RootFilesystem;
pub use scoped::{MountViewResolver, ScopedFilesystem};
pub use types::{
    AtomicSubtreeEntry, BackendCapabilities, BackendId, BackendKind, Capability, ContentKind,
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, IndexConflictReason,
    IndexPolicy, ScopedAtomicSubtreeEntry, StorageClass, TxnCapability,
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
