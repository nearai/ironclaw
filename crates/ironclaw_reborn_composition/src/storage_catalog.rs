use ironclaw_filesystem::{Capability, CompositeRootFilesystem, FilesystemError, TxnCapability};
use ironclaw_host_api::VirtualPath;

use crate::RebornBuildError;

#[derive(Debug, Clone, Copy)]
struct RuntimeStoragePlane {
    name: &'static str,
    path: &'static str,
    capabilities: &'static [Capability],
    txn: TxnCapability,
}

const BYTE_STORE_CAPABILITIES: &[Capability] = &[
    Capability::Read,
    Capability::Write,
    Capability::List,
    Capability::Stat,
    Capability::Delete,
];

const RECORD_STORE_CAPABILITIES: &[Capability] = &[
    Capability::Read,
    Capability::Write,
    Capability::List,
    Capability::Stat,
    Capability::Delete,
    Capability::Records,
    Capability::Query,
];

const MEMORY_STORE_CAPABILITIES: &[Capability] = &[
    Capability::Read,
    Capability::Write,
    Capability::List,
    Capability::Stat,
    Capability::Delete,
    Capability::Records,
    Capability::Query,
    Capability::IndexFts,
    Capability::IndexVector,
];

const EVENT_STORE_CAPABILITIES: &[Capability] = &[
    Capability::Read,
    Capability::Write,
    Capability::Append,
    Capability::List,
    Capability::Stat,
    Capability::Events,
];

const REQUIRED_RUNTIME_STORAGE_PLANES: &[RuntimeStoragePlane] = &[
    RuntimeStoragePlane {
        name: "tenant scoped state",
        path: "/tenants",
        capabilities: RECORD_STORE_CAPABILITIES,
        txn: TxnCapability::Cas,
    },
    RuntimeStoragePlane {
        name: "event log",
        path: "/events",
        capabilities: EVENT_STORE_CAPABILITIES,
        txn: TxnCapability::None,
    },
    RuntimeStoragePlane {
        name: "persistent memory",
        path: "/memory",
        capabilities: MEMORY_STORE_CAPABILITIES,
        txn: TxnCapability::Cas,
    },
    RuntimeStoragePlane {
        name: "project workspace",
        path: "/projects",
        capabilities: BYTE_STORE_CAPABILITIES,
        txn: TxnCapability::None,
    },
    RuntimeStoragePlane {
        name: "extension packages",
        path: "/system/extensions",
        capabilities: BYTE_STORE_CAPABILITIES,
        txn: TxnCapability::None,
    },
    RuntimeStoragePlane {
        name: "extension lifecycle state",
        path: "/system/extensions/.installations",
        capabilities: RECORD_STORE_CAPABILITIES,
        txn: TxnCapability::Cas,
    },
    RuntimeStoragePlane {
        name: "system settings",
        path: "/system/settings",
        capabilities: RECORD_STORE_CAPABILITIES,
        txn: TxnCapability::Cas,
    },
    RuntimeStoragePlane {
        name: "system skills",
        path: "/system/skills",
        capabilities: BYTE_STORE_CAPABILITIES,
        txn: TxnCapability::None,
    },
];

pub(crate) async fn validate_reborn_runtime_storage(
    filesystem: &CompositeRootFilesystem,
) -> Result<(), RebornBuildError> {
    for plane in REQUIRED_RUNTIME_STORAGE_PLANES {
        validate_runtime_storage_plane(filesystem, *plane).await?;
    }
    Ok(())
}

async fn validate_runtime_storage_plane(
    filesystem: &CompositeRootFilesystem,
    plane: RuntimeStoragePlane,
) -> Result<(), RebornBuildError> {
    let path = VirtualPath::new(plane.path).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!(
            "runtime storage plane `{}` has invalid validation path `{}`: {error}",
            plane.name, plane.path
        ),
    })?;
    let placement = filesystem
        .describe_path(&path)
        .await
        .map_err(|error| match error {
            FilesystemError::MountNotFound { .. } => RebornBuildError::InvalidConfig {
                reason: format!(
                    "runtime storage plane `{}` requires `{}`, but no configured mount covers it",
                    plane.name, plane.path
                ),
            },
            error => RebornBuildError::Filesystem(error),
        })?;
    let missing: Vec<_> = plane
        .capabilities
        .iter()
        .copied()
        .filter(|capability| !placement.capabilities.has(*capability))
        .collect();
    if !missing.is_empty() {
        return Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "runtime storage plane `{}` at `{}` is backed by `{}` but is missing capabilities: {}",
                plane.name,
                plane.path,
                placement.matched_root.as_str(),
                format_capabilities(&missing)
            ),
        });
    }
    if txn_rank(placement.capabilities.txn()) < txn_rank(plane.txn) {
        return Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "runtime storage plane `{}` at `{}` is backed by `{}` with {:?} transactions, but {:?} is required",
                plane.name,
                plane.path,
                placement.matched_root.as_str(),
                placement.capabilities.txn(),
                plane.txn
            ),
        });
    }
    Ok(())
}

fn format_capabilities(capabilities: &[Capability]) -> String {
    capabilities
        .iter()
        .map(|capability| format!("{capability:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn txn_rank(txn: TxnCapability) -> u8 {
    match txn {
        TxnCapability::None => 0,
        TxnCapability::Cas => 1,
        TxnCapability::MultiKey => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{
        BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
        InMemoryBackend, IndexPolicy, MountDescriptor, StorageClass,
    };

    use super::*;

    fn descriptor(path: &str, capabilities: BackendCapabilities) -> MountDescriptor {
        MountDescriptor {
            virtual_root: VirtualPath::new(path).expect("valid virtual root"),
            backend_id: BackendId::new(format!("test-{}", path.replace('/', "-")))
                .expect("valid backend id"),
            backend_kind: BackendKind::DatabaseFilesystem,
            storage_class: StorageClass::StructuredRecords,
            content_kind: ContentKind::StructuredRecord,
            index_policy: IndexPolicy::BackendDefined,
            capabilities,
        }
    }

    #[tokio::test]
    async fn runtime_storage_validation_fails_when_required_plane_is_unmounted() {
        let mut filesystem = CompositeRootFilesystem::new();
        let backend = Arc::new(InMemoryBackend::new());
        filesystem
            .mount(
                descriptor("/tenants", BackendCapabilities::in_memory_full()),
                backend,
            )
            .expect("mount tenants");

        let error = validate_reborn_runtime_storage(&filesystem)
            .await
            .expect_err("missing planes should fail validation");

        assert!(
            matches!(
                error,
                RebornBuildError::InvalidConfig { ref reason }
                    if reason.contains("runtime storage plane `event log` requires `/events`")
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn runtime_storage_validation_accepts_complete_composite() {
        let mut filesystem = CompositeRootFilesystem::new();
        let backend = Arc::new(InMemoryBackend::new());
        for path in [
            "/tenants",
            "/events",
            "/memory",
            "/projects",
            "/system/extensions",
            "/system/settings",
            "/system/skills",
        ] {
            filesystem
                .mount(
                    descriptor(path, BackendCapabilities::in_memory_full()),
                    Arc::clone(&backend),
                )
                .expect("mount runtime plane");
        }

        validate_reborn_runtime_storage(&filesystem)
            .await
            .expect("complete composite should validate");
    }

    #[tokio::test]
    async fn runtime_storage_validation_rejects_record_plane_without_cas() {
        let mut filesystem = CompositeRootFilesystem::new();
        let backend = Arc::new(InMemoryBackend::new());
        for path in [
            "/tenants",
            "/events",
            "/memory",
            "/projects",
            "/system/extensions",
            "/system/settings",
            "/system/skills",
        ] {
            let capabilities = if path == "/tenants" {
                BackendCapabilities::in_memory_full().with_txn(TxnCapability::None)
            } else {
                BackendCapabilities::in_memory_full()
            };
            filesystem
                .mount(descriptor(path, capabilities), Arc::clone(&backend))
                .expect("mount runtime plane");
        }

        let error = validate_reborn_runtime_storage(&filesystem)
            .await
            .expect_err("record plane without CAS should fail validation");

        assert!(
            matches!(
                error,
                RebornBuildError::InvalidConfig { ref reason }
                    if reason.contains("runtime storage plane `tenant scoped state`")
                        && reason.contains("Cas is required")
            ),
            "{error}"
        );
    }
}
