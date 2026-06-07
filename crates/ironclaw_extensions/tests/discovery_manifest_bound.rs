//! Discovery-level coverage for the pre-read manifest size bound.
//!
//! `ExtensionDiscovery::discover_with_manifest_contracts` must stat the
//! manifest and refuse to materialize it when it exceeds `MAX_MANIFEST_BYTES`,
//! BEFORE reading the body (DoS pre-read bound). This is proven with a fake
//! filesystem whose `get` (the body read) PANICS — discovery must reject the
//! oversized manifest via `stat` alone and never call `get`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_extensions::{
    CapabilityProviderHostApiContract, ExtensionDiscovery, ExtensionError, HostApiContractRegistry,
    MAX_MANIFEST_BYTES, ManifestSource,
};
use ironclaw_filesystem::{
    DirEntry, Entry, FileStat, FileType, FilesystemError, FilesystemOperation, InMemoryBackend,
    RootFilesystem, VersionedEntry,
};
use ironclaw_host_api::{HostPortCatalog, VirtualPath};

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

// ─────────────────────────────────────────────────────────────────────────────
// Tolerant + bounded discovery (Critical-1 DoS bound + Critical-2 fail-open fix)
// ─────────────────────────────────────────────────────────────────────────────

/// Contract registry carrying the capability_provider host_api contract that the
/// discovery-valid manifests reference.
fn capability_provider_contracts() -> HostApiContractRegistry {
    let mut contracts = HostApiContractRegistry::new();
    contracts
        .register(std::sync::Arc::new(
            CapabilityProviderHostApiContract::new().expect("capability provider contract"),
        ))
        .expect("register capability provider contract");
    contracts
}

/// A discovery-valid `InstalledLocal` v2 manifest (host_api capability_provider
/// form — the legacy top-level `[[capabilities]]` is rejected for installed
/// sources by the discovery contracts).
fn valid_manifest_toml(id: &str) -> String {
    format!(
        r#"schema_version = "reborn.extension_manifest.v2"
id = "{id}"
name = "{id}"
version = "0.1.0"
description = "{id} extension"
trust = "third_party"

[runtime]
kind = "wasm"
module = "wasm/{id}.wasm"

[[host_api]]
id = "ironclaw.capability_provider/v1"
section = "capability_provider.tools"

[capability_provider.tools]

[[capability_provider.tools.capabilities]]
id = "{id}.run"
description = "Run {id}"
effects = ["dispatch_capability"]
default_permission = "allow"
visibility = "model"
input_schema_ref = "schemas/{id}/run.input.v1.json"
output_schema_ref = "schemas/{id}/run.output.v1.json"
prompt_doc_ref = "prompts/{id}/run.md"
"#
    )
}

/// A read-counting filesystem over a fixed in-memory manifest map. Directory
/// entries are derived from the configured ids. `read_file_bounded` increments a
/// counter and, for ids listed in `panic_on_read`, PANICS — proving the bounded
/// discovery never reads a surplus manifest.
struct CountingManifestFs {
    /// id -> manifest body (sorted set drives `list_dir` order).
    manifests: BTreeMap<String, String>,
    /// ids whose body read must NEVER happen (surplus beyond the bound).
    panic_on_read: Vec<String>,
    reads: AtomicUsize,
}

impl CountingManifestFs {
    fn manifest_id_for(path: &VirtualPath) -> Option<String> {
        let rest = path.as_str().strip_prefix("/system/extensions/")?;
        let id = rest.strip_suffix("/manifest.toml")?;
        Some(id.to_string())
    }
}

#[async_trait]
impl RootFilesystem for CountingManifestFs {
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if path.as_str() == "/system/extensions" {
            return Ok(self
                .manifests
                .keys()
                .map(|id| DirEntry {
                    name: id.clone(),
                    path: VirtualPath::new(format!("/system/extensions/{id}")).expect("child"),
                    file_type: FileType::Directory,
                })
                .collect());
        }
        Ok(Vec::new())
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let len = Self::manifest_id_for(path)
            .and_then(|id| self.manifests.get(&id).map(|body| body.len() as u64))
            .unwrap_or(0);
        Ok(FileStat {
            path: path.clone(),
            file_type: FileType::File,
            len,
            modified: None,
            sensitive: false,
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let Some(id) = Self::manifest_id_for(path) else {
            return Ok(None);
        };
        assert!(
            !self.panic_on_read.contains(&id),
            "bounded discovery read a surplus manifest it must have skipped: {id}"
        );
        match self.manifests.get(&id) {
            Some(body) => Ok(Some(VersionedEntry {
                path: path.clone(),
                entry: Entry::bytes(body.clone().into_bytes()),
                version: ironclaw_filesystem::RecordVersion::from_backend(1),
            })),
            None => Ok(None),
        }
    }
}

/// Critical-1: with more extension directories than the bound, the bounded
/// discovery reads/parses at most `max_extensions` manifests and records the
/// surplus as quarantines WITHOUT reading them. The surplus `get` PANICS if
/// reached, so a passing test proves the read storm is capped.
#[tokio::test]
async fn bounded_discovery_stops_reading_surplus_extensions() {
    const BOUND: usize = 3;
    let mut manifests = BTreeMap::new();
    // ext-000 .. ext-005 — first BOUND (by sorted name) are read; the rest must
    // never be read.
    let mut panic_on_read = Vec::new();
    for i in 0..6usize {
        let id = format!("ext-{i:03}");
        manifests.insert(id.clone(), valid_manifest_toml(&id));
        if i >= BOUND {
            panic_on_read.push(id);
        }
    }
    let fs = CountingManifestFs {
        manifests,
        panic_on_read,
        reads: AtomicUsize::new(0),
    };
    let root = VirtualPath::new("/system/extensions").expect("root");

    let result = ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded(
        &fs,
        &root,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
        BOUND,
    )
    .await
    .expect("tolerant bounded discovery never fails on per-package issues");

    assert_eq!(
        fs.reads.load(Ordering::SeqCst),
        BOUND,
        "exactly the bound number of manifests may be read"
    );
    assert_eq!(
        result.registry.extensions().count(),
        BOUND,
        "only the bounded prefix of extensions is loaded"
    );
    assert_eq!(
        result.quarantined.len(),
        3,
        "the 3 surplus extensions must be quarantined (not read)"
    );
    for q in &result.quarantined {
        assert!(
            q.reason.contains("exceeded discovery bound"),
            "surplus quarantine reason must cite the bound, got: {}",
            q.reason
        );
    }
}

/// Critical-2: ONE malformed manifest among several valid ones quarantines only
/// the bad package; valid siblings still load and the call does NOT fail.
#[tokio::test]
async fn tolerant_discovery_quarantines_only_the_malformed_package() {
    let fs = InMemoryBackend::new();
    fs.write_file(
        &VirtualPath::new("/system/extensions/good-a/manifest.toml").unwrap(),
        valid_manifest_toml("good-a").as_bytes(),
    )
    .await
    .expect("write good-a");
    fs.write_file(
        &VirtualPath::new("/system/extensions/bad/manifest.toml").unwrap(),
        b"this is not valid toml {{{",
    )
    .await
    .expect("write bad");
    fs.write_file(
        &VirtualPath::new("/system/extensions/good-b/manifest.toml").unwrap(),
        valid_manifest_toml("good-b").as_bytes(),
    )
    .await
    .expect("write good-b");

    let root = VirtualPath::new("/system/extensions").expect("root");
    let result = ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded(
        &fs,
        &root,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
        64,
    )
    .await
    .expect("a malformed sibling must not fail the whole discovery");

    let loaded: Vec<String> = result
        .registry
        .extensions()
        .map(|p| p.manifest.id.as_str().to_string())
        .collect();
    assert!(
        loaded.contains(&"good-a".to_string()),
        "good-a must load; quarantines: {:?}",
        result.quarantined
    );
    assert!(loaded.contains(&"good-b".to_string()), "good-b must load");
    assert!(
        !loaded.contains(&"bad".to_string()),
        "the malformed package must be quarantined"
    );
    assert_eq!(
        result.quarantined.len(),
        1,
        "exactly the malformed package is quarantined"
    );
    assert_eq!(result.quarantined[0].extension_id, "bad");
}

/// Root-unreadable is the ONLY case that surfaces as an outer `Err` (the caller
/// uses it to fall back to builtin-only).
#[tokio::test]
async fn tolerant_discovery_propagates_root_list_failure() {
    struct UnreadableRootFs;

    #[async_trait]
    impl RootFilesystem for UnreadableRootFs {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "extensions root unreadable".to_string(),
            })
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            })
        }
    }

    let root = VirtualPath::new("/system/extensions").expect("root");
    let err = ExtensionDiscovery::discover_with_manifest_contracts_tolerant_bounded(
        &UnreadableRootFs,
        &root,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
        &capability_provider_contracts(),
        64,
    )
    .await
    .expect_err("an unreadable root must surface as an outer Err");
    assert!(
        matches!(err, ExtensionError::Filesystem(_)),
        "root list failure must surface as a filesystem error, got: {err:?}"
    );
}
