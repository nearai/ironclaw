//! Discovery-level coverage for the pre-read manifest size bound.
//!
//! `ExtensionDiscovery::discover_with_manifest_contracts` must stat the
//! manifest and refuse to materialize it when it exceeds `MAX_MANIFEST_BYTES`,
//! BEFORE reading the body (DoS pre-read bound). This is proven with a fake
//! filesystem whose `get` (the body read) PANICS — discovery must reject the
//! oversized manifest via `stat` alone and never call `get`.

use async_trait::async_trait;
use ironclaw_extensions::{ExtensionDiscovery, ExtensionError, MAX_MANIFEST_BYTES};
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, RootFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::VirtualPath;

/// Reports one extension dir with a manifest that `stat`s as far larger than
/// `MAX_MANIFEST_BYTES`, and PANICS if anything attempts to read the body.
struct OversizedManifestFs;

#[async_trait]
impl RootFilesystem for OversizedManifestFs {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path.as_str() == "/system/extensions" {
            return Ok(vec![DirEntry {
                name: "huge".to_string(),
                path: VirtualPath::new("/system/extensions/huge").expect("child"),
                file_type: FileType::Directory,
            }]);
        }
        Ok(Vec::new())
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        // The manifest "exists" but is gigantic.
        Ok(FileStat {
            path: path.clone(),
            file_type: FileType::File,
            len: (MAX_MANIFEST_BYTES as u64) * 1024,
            modified: None,
            sensitive: false,
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        panic!(
            "discovery must reject the oversized manifest via the pre-read size bound \
             (stat) and must NOT read its body; get() called on {}",
            path.as_str()
        );
    }
}

#[tokio::test]
async fn discovery_rejects_oversized_manifest_before_reading_the_body() {
    let fs = OversizedManifestFs;
    let root = VirtualPath::new("/system/extensions").expect("root");

    let err = ExtensionDiscovery::discover(&fs, &root)
        .await
        .expect_err("oversized manifest must be rejected");

    match err {
        ExtensionError::InvalidManifest { reason } => {
            assert!(
                reason.contains("exceeds") && reason.contains("ceiling"),
                "rejection must cite the size ceiling, got: {reason}"
            );
        }
        other => panic!("expected InvalidManifest size rejection, got: {other:?}"),
    }
}

/// A manifest whose stat is within the bound but whose body read returns
/// NotFound is surfaced as a filesystem error (sanity: the bounded read path is
/// actually exercised when the file is within the ceiling).
#[tokio::test]
async fn discovery_within_bound_proceeds_to_read() {
    struct WithinBoundFs;

    #[async_trait]
    impl RootFilesystem for WithinBoundFs {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            if path.as_str() == "/system/extensions" {
                return Ok(vec![DirEntry {
                    name: "small".to_string(),
                    path: VirtualPath::new("/system/extensions/small").expect("child"),
                    file_type: FileType::Directory,
                }]);
            }
            Ok(Vec::new())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Ok(FileStat {
                path: path.clone(),
                file_type: FileType::File,
                len: 16,
                modified: None,
                sensitive: false,
            })
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            // Body read IS reached for a within-bound manifest.
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            })
        }
    }

    let err = ExtensionDiscovery::discover(
        &WithinBoundFs,
        &VirtualPath::new("/system/extensions").unwrap(),
    )
    .await
    .expect_err("within-bound manifest reaches the body read (which errors here)");
    // The error must come from the read path, not the size bound.
    assert!(
        matches!(err, ExtensionError::Filesystem(_)),
        "within-bound manifest must proceed to the body read, got: {err:?}"
    );
}
