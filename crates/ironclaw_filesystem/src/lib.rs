//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.
//!
//! The local backend canonicalizes existing paths and their nearest existing
//! ancestors before opening files, and it re-roots new leaf paths on the checked
//! canonical parent. That narrows symlink escape opportunities but does not
//! provide a kernel-enforced race-free guarantee against a writable mount root
//! being modified between containment checks and opens. Production hardening for
//! hostile local directories should use fd-relative traversal such as `openat2`
//! with `RESOLVE_BENEATH`, `O_NOFOLLOW`, or a capability filesystem crate.
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
pub use root::{BatchPut, MAX_BATCH_PUTS, RootFilesystem};
pub use scoped::{MountViewResolver, ScopedBatchPut, ScopedFilesystem};
pub use types::{
    BackendCapabilities, BackendId, BackendKind, Capability, ContentKind, DirEntry, FileStat,
    FileType, FilesystemError, FilesystemOperation, IndexConflictReason, IndexPolicy, StorageClass,
    TxnCapability,
};

fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    std::path::Path::new(path).starts_with(std::path::Path::new(prefix))
}

/// Longest common leading-component prefix of `paths`, as a [`VirtualPath`].
///
/// Component-aware (never splits inside a path segment): for two sibling leaves
/// under the same directory it returns that directory, and for an ancestor /
/// descendant pair it returns the ancestor. Returns `None` when the paths share
/// no leading component (e.g. they live under different virtual roots) or when
/// `paths` is empty.
///
/// Used by [`RootFilesystem::put_batch`](crate::RootFilesystem::put_batch) to
/// derive the prefix that scopes a multi-key transaction: every input path
/// satisfies `path_prefix_matches(result, path)`.
fn common_dir_prefix<'a>(
    mut paths: impl Iterator<Item = &'a ironclaw_host_api::VirtualPath>,
) -> Option<ironclaw_host_api::VirtualPath> {
    let first = paths.next()?;
    let mut common: Vec<&str> = first
        .as_str()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    for path in paths {
        let shared = common
            .iter()
            .zip(path.as_str().split('/').filter(|s| !s.is_empty()))
            .take_while(|(a, b)| **a == *b)
            .count();
        common.truncate(shared);
    }
    if common.is_empty() {
        return None;
    }
    ironclaw_host_api::VirtualPath::new(format!("/{}", common.join("/"))).ok()
}

#[cfg(test)]
mod tests {
    use super::{common_dir_prefix, path_prefix_matches};
    use ironclaw_host_api::VirtualPath;

    #[test]
    fn path_prefix_matches_root_and_component_boundaries() {
        assert!(path_prefix_matches("/", "/projects"));
        assert!(path_prefix_matches("/projects", "/projects"));
        assert!(path_prefix_matches("/projects", "/projects/readme.md"));
        assert!(!path_prefix_matches("/projects", "/projects-private"));
    }

    #[test]
    fn common_dir_prefix_is_component_aware() {
        let vp = |s: &str| VirtualPath::new(s).unwrap();

        // Identical directory, divergent leaves → the shared directory.
        let prefix =
            common_dir_prefix([vp("/secrets/leases/A"), vp("/secrets/leases/B")].iter()).unwrap();
        assert_eq!(prefix.as_str(), "/secrets/leases");

        // Nested: one path is an ancestor of the other → the ancestor.
        let prefix =
            common_dir_prefix([vp("/secrets/leases/x"), vp("/secrets/leases/x/y")].iter()).unwrap();
        assert_eq!(prefix.as_str(), "/secrets/leases/x");

        // Divergent siblings high up → the common single root element.
        let prefix = common_dir_prefix([vp("/secrets/a"), vp("/secrets/b")].iter()).unwrap();
        assert_eq!(prefix.as_str(), "/secrets");

        // Single element → the path itself (all components shared).
        let prefix = common_dir_prefix([vp("/secrets/only/L1")].iter()).unwrap();
        assert_eq!(prefix.as_str(), "/secrets/only/L1");

        // No shared leading component → None.
        assert!(common_dir_prefix([vp("/secrets/a"), vp("/memory/b")].iter()).is_none());

        // Divergent virtual roots (different mounts) → None.
        assert!(common_dir_prefix([vp("/memory/a"), vp("/turns/b")].iter()).is_none());

        // The computed prefix matches every input via `path_prefix_matches`.
        let paths = [vp("/secrets/leases/A"), vp("/secrets/leases/sub/B")];
        let prefix = common_dir_prefix(paths.iter()).unwrap();
        assert!(
            paths
                .iter()
                .all(|p| path_prefix_matches(prefix.as_str(), p.as_str()))
        );
    }
}
