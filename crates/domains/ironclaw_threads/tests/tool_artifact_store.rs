use std::sync::Arc;

use ironclaw_filesystem::InMemoryBackend;
use ironclaw_host_api::{
    artifact::{
        AccountedArtifactPersister, ArtifactAccessPort, ArtifactDigest, ArtifactId,
        ArtifactLineRange, ArtifactNamespaceId, ArtifactOwnerScope, ArtifactPersistencePort,
        ArtifactReadRequest, ArtifactReadTarget, ArtifactSelector, ArtifactWriteError,
        ArtifactWriteMetadata,
    },
    ids::{CapabilityId, InvocationId, RunId, UserId},
    resource::{
        ReservationStatus, ResourceEstimate, ResourceReceipt, ResourceScope, ResourceUsage,
    },
};
use ironclaw_threads::DurableToolArtifactStore;

fn owner_scope() -> ArtifactOwnerScope {
    let scope = ResourceScope::local_default(
        UserId::new("artifact-owner").expect("owner id"),
        InvocationId::new(),
    )
    .expect("resource scope");
    ArtifactOwnerScope::from_resource_scope(&scope)
}

fn metadata(namespace: ArtifactNamespaceId) -> ArtifactWriteMetadata {
    ArtifactWriteMetadata {
        write_key: None,
        owner_scope: owner_scope(),
        namespace,
        producer_capability_id: CapabilityId::new("builtin.grep").expect("capability id"),
        content_type: "text/plain".to_string(),
        expected_bytes: None,
    }
}

fn read_request(
    namespace: ArtifactNamespaceId,
    artifact_id: ArtifactId,
    selector: ArtifactSelector,
) -> ArtifactReadRequest {
    ArtifactReadRequest {
        owner_scope: owner_scope(),
        namespace,
        target: ArtifactReadTarget {
            artifact_id,
            selector,
            max_output_bytes: 24 * 1024,
        },
    }
}

/// `write_key` means "one artifact per invocation", so allocations sharing a key
/// adopt the same artifact rather than minting new ones.
///
/// That is the retry-idempotency contract the coding spill path relies on: a
/// redispatched invocation re-adopts its existing write instead of duplicating
/// it (`coding.rs` passes `write_key: Some(scope.invocation_id)`).
///
/// It also records the boundary condition. Adoption is only safe while one
/// invocation produces one result. `invoke_capability_batch` executes its
/// invocations sequentially and each mints its own `InvocationId`, so today no
/// caller shares a key across distinct results. If a batch ever dispatches
/// concurrently, or any caller reuses one invocation for several spills, those
/// results would silently land in one artifact and every `artifact://N` handed
/// to the model would resolve to the wrong bytes — such a caller needs a
/// per-result write key, not the invocation id.
#[tokio::test]
async fn allocations_sharing_a_write_key_adopt_one_artifact() {
    let store = Arc::new(
        DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("artifact store"),
    );
    let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
    let owner = owner_scope();
    let write_key = InvocationId::new();

    let mut ids = Vec::new();
    for _ in 0..4 {
        let handle = store
            .allocate(ArtifactWriteMetadata {
                write_key: Some(write_key),
                owner_scope: owner.clone(),
                namespace,
                producer_capability_id: CapabilityId::new("builtin.read").expect("capability id"),
                content_type: "text/plain".to_string(),
                expected_bytes: None,
            })
            .await
            .expect("allocate adopts the bound artifact");
        ids.push(handle.artifact_id().get());
    }
    assert_eq!(
        ids,
        vec![0, 0, 0, 0],
        "a shared write_key must re-adopt one artifact, not mint new ids"
    );

    // A different key is a different result and must get its own artifact, or
    // idempotency would degenerate into a single artifact per namespace.
    let other = store
        .allocate(ArtifactWriteMetadata {
            write_key: Some(InvocationId::new()),
            owner_scope: owner,
            namespace,
            producer_capability_id: CapabilityId::new("builtin.read").expect("capability id"),
            content_type: "text/plain".to_string(),
            expected_bytes: None,
        })
        .await
        .expect("allocate for a distinct write key");
    assert_ne!(
        other.artifact_id().get(),
        0,
        "a distinct write key must not adopt another invocation's artifact"
    );
}

#[tokio::test]
async fn finalized_artifact_supports_indexed_line_reads() {
    let store =
        DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("artifact store");
    let namespace = ArtifactNamespaceId::from_root_run(RunId::new());

    let first = store
        .allocate(metadata(namespace))
        .await
        .expect("allocate first artifact");
    let second = store
        .allocate(metadata(namespace))
        .await
        .expect("allocate second artifact");
    assert_eq!(first.artifact_id().get(), 0);
    assert_eq!(second.artifact_id().get(), 1);

    store
        .append(&first, b"one\ntwo\n")
        .await
        .expect("append first chunk");
    assert!(
        store
            .read(read_request(
                namespace,
                first.artifact_id(),
                ArtifactSelector::Full,
            ))
            .await
            .expect("read incomplete artifact")
            .is_none(),
        "incomplete artifacts must not be model-readable"
    );

    store
        .append(&first, b"three\nfour\n")
        .await
        .expect("append second chunk");
    let completed = store.finalize(first).await.expect("finalize artifact");
    assert_eq!(completed.byte_len, 19);
    assert_eq!(completed.total_lines, Some(4));

    assert_eq!(
        completed.digest,
        ArtifactDigest::from_bytes(b"one\ntwo\nthree\nfour\n")
    );
    let lines = store
        .read(read_request(
            namespace,
            completed.artifact_ref.id(),
            ArtifactSelector::Lines(ArtifactLineRange { start: 2, end: 3 }),
        ))
        .await
        .expect("read artifact")
        .expect("finalized artifact");
    assert_eq!(lines.content, b"two\nthree\n");
    assert_eq!(lines.total_bytes, 19);
    assert_eq!(lines.total_lines, Some(4));
}

#[tokio::test]
async fn accounted_persistence_rejects_bytes_not_covered_by_receipt() {
    let store =
        DurableToolArtifactStore::new(Arc::new(InMemoryBackend::new())).expect("artifact store");
    let namespace = ArtifactNamespaceId::from_root_run(RunId::new());
    let scope = ResourceScope::local_default(
        UserId::new("artifact-owner").expect("owner id"),
        InvocationId::new(),
    )
    .expect("resource scope");
    let receipt = ResourceReceipt {
        id: Default::default(),
        scope,
        status: ReservationStatus::Reconciled,
        estimate: ResourceEstimate::default(),
        actual: Some(ResourceUsage {
            output_bytes: 3,
            ..ResourceUsage::default()
        }),
    };

    let error = store
        .persist(metadata(namespace), b"four", &receipt)
        .await
        .expect_err("receipt must cover every persisted output byte");

    assert_eq!(error, ArtifactWriteError::Budget);
}
