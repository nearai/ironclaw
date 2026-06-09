use std::sync::Arc;

use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RecordKind, RootFilesystem};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId, VirtualPath};
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_reborn_openai_compat::{
    OpenAiCompatActorScope, OpenAiCompatBindInternalRefs, OpenAiCompatIdempotencyKey,
    OpenAiCompatInternalRefs, OpenAiCompatProductActionRef, OpenAiCompatProjectionRef,
    OpenAiCompatPublicId, OpenAiCompatRecordAcceptedAck, OpenAiCompatRefLookup,
    OpenAiCompatRefOperation, OpenAiCompatRefReservation, OpenAiCompatRefReservationOutcome,
    OpenAiCompatRefStore, OpenAiCompatRequestFingerprint, OpenAiCompatResourceBinding,
    OpenAiCompatRouteSurface, OpenAiCompatTurnRunRef, OpenAiResponseId,
};
use ironclaw_reborn_openai_compat_storage::FilesystemOpenAiCompatRefStore;
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};
use serde_json::json;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn durable_store_replays_same_idempotency_key_after_reopen() {
    let (filesystem, root, store) = test_store("replay");
    let request = reservation("tenant-a", "alice", "same-key", b"same body");
    let created = expect_created(store.reserve(request.clone()).await);

    let reopened = FilesystemOpenAiCompatRefStore::with_root(filesystem, root);
    let replayed = expect_replayed(reopened.reserve(request).await);

    assert_eq!(replayed.public_id, created.public_id);
    assert_eq!(replayed.request_fingerprint, created.request_fingerprint);
}

#[tokio::test]
async fn durable_store_persists_accepted_ack_for_idempotency_replay_after_reopen() {
    let (filesystem, root, store) = test_store("accepted-ack");
    let request = reservation("tenant-a", "alice", "same-key", b"same body");
    let created = expect_created(store.reserve(request.clone()).await);
    let ack = accepted_ack("msg:accepted");

    let updated = store
        .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
            actor("tenant-a", "alice"),
            created.public_id.clone(),
            ack.clone(),
        ))
        .await
        .expect("record accepted ack")
        .expect("mapping should exist");

    assert_eq!(updated.accepted_ack, Some(ack.clone()));

    let reopened = FilesystemOpenAiCompatRefStore::with_root(filesystem, root);
    let replayed = expect_replayed(reopened.reserve(request).await);

    assert_eq!(replayed.public_id, created.public_id);
    assert_eq!(replayed.accepted_ack, Some(ack));
}

#[tokio::test]
async fn durable_store_same_key_distinct_actors_create_distinct_refs() {
    let (_, _, store) = test_store("same-key-distinct-actors");
    let alice_request = reservation("tenant-a", "alice", "same-key", b"same body");
    let bob_request = reservation("tenant-a", "bob", "same-key", b"same body");

    let alice = expect_created(store.reserve(alice_request.clone()).await);
    let bob = expect_created(store.reserve(bob_request.clone()).await);
    let alice_replay = expect_replayed(store.reserve(alice_request).await);
    let bob_replay = expect_replayed(store.reserve(bob_request).await);

    assert_ne!(alice.public_id, bob.public_id);
    assert_eq!(alice_replay.public_id, alice.public_id);
    assert_eq!(bob_replay.public_id, bob.public_id);
}

#[tokio::test]
async fn durable_store_record_accepted_ack_wrong_owner_is_missing() {
    let (_, _, store) = test_store("accepted-ack-auth");
    let created = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "same-key", b"same body"))
            .await,
    );

    let denied = store
        .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
            actor("tenant-a", "bob"),
            created.public_id.clone(),
            accepted_ack("msg:denied"),
        ))
        .await
        .expect("record accepted ack");

    assert!(denied.is_none());
    let loaded = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            created.public_id,
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect("lookup")
        .expect("mapping exists");
    assert!(loaded.accepted_ack.is_none());
}

#[tokio::test]
async fn durable_store_conflicts_same_key_different_body_after_reopen() {
    let (filesystem, root, store) = test_store("conflict");
    let request = reservation("tenant-a", "alice", "same-key", b"first body");
    let created = expect_created(store.reserve(request).await);

    let reopened = FilesystemOpenAiCompatRefStore::with_root(filesystem, root);
    let conflict = reopened
        .reserve(reservation(
            "tenant-a",
            "alice",
            "same-key",
            b"different body",
        ))
        .await
        .expect("reserve should not fail");

    assert!(matches!(
        conflict,
        OpenAiCompatRefReservationOutcome::Conflict(_)
    ));
    let replayed = expect_replayed(
        reopened
            .reserve(reservation("tenant-a", "alice", "same-key", b"first body"))
            .await,
    );
    assert_eq!(replayed.public_id, created.public_id);
}

#[tokio::test]
async fn durable_store_without_idempotency_key_always_creates_new_public_ref() {
    let (_, _, store) = test_store("no-key");
    let owner = actor("tenant-a", "alice");
    let fingerprint = OpenAiCompatRequestFingerprint::from_body_bytes(b"same body");

    let first = expect_created(
        store
            .reserve(OpenAiCompatRefReservation::new(
                owner.clone(),
                OpenAiCompatRouteSurface::ResponsesApi,
                fingerprint.clone(),
                None,
            ))
            .await,
    );
    let second = expect_created(
        store
            .reserve(OpenAiCompatRefReservation::new(
                owner,
                OpenAiCompatRouteSurface::ResponsesApi,
                fingerprint,
                None,
            ))
            .await,
    );

    assert_ne!(first.public_id, second.public_id);
}

#[tokio::test]
async fn durable_store_writes_per_public_id_records_without_global_state() {
    let (filesystem, root, store) = test_store("per-ref-records");
    let first = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "first-key", b"first body"))
            .await,
    );
    let second = expect_created(
        store
            .reserve(reservation(
                "tenant-a",
                "alice",
                "second-key",
                b"second body",
            ))
            .await,
    );

    let state_path =
        VirtualPath::new(format!("{}/state.json", root.as_str())).expect("valid legacy state path");
    assert!(
        filesystem
            .get(&state_path)
            .await
            .expect("legacy state lookup")
            .is_none()
    );

    let first_path = public_id_record_path(&root, &first.public_id);
    let second_path = public_id_record_path(&root, &second.public_id);
    assert_ne!(first_path, second_path);
    assert!(
        filesystem
            .get(&first_path)
            .await
            .expect("first record lookup")
            .is_some()
    );
    assert!(
        filesystem
            .get(&second_path)
            .await
            .expect("second record lookup")
            .is_some()
    );
}

#[tokio::test]
async fn durable_store_does_not_leak_unauthorized_refs() {
    let (_, _, store) = test_store("auth");
    let created = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "key", b"body"))
            .await,
    );

    let bob_lookup = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "bob"),
            created.public_id.clone(),
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect("lookup");

    assert!(bob_lookup.is_none());
}

#[tokio::test]
async fn durable_store_persists_bound_internal_refs() {
    let (filesystem, root, store) = test_store("bind");
    let created = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "key", b"body"))
            .await,
    );
    let internal_refs = OpenAiCompatInternalRefs::new(
        OpenAiCompatProductActionRef::new("product-action:1").expect("valid product action ref"),
    )
    .with_turn_run_ref(OpenAiCompatTurnRunRef::new("turn-run:1").expect("valid turn run ref"))
    .with_projection_ref(OpenAiCompatProjectionRef::new("projection:1").expect("valid projection"));

    let bound = store
        .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
            actor("tenant-a", "alice"),
            created.public_id.clone(),
            internal_refs.clone(),
        ))
        .await
        .expect("bind")
        .expect("mapping should exist");

    assert_eq!(
        bound.binding,
        OpenAiCompatResourceBinding::Bound {
            internal_refs: internal_refs.clone()
        }
    );

    let reopened = FilesystemOpenAiCompatRefStore::with_root(filesystem, root);
    let loaded = reopened
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            created.public_id,
            OpenAiCompatRefOperation::StreamResume,
        ))
        .await
        .expect("lookup")
        .expect("mapping should survive reopen");

    assert_eq!(
        loaded.binding,
        OpenAiCompatResourceBinding::Bound { internal_refs }
    );
}

#[tokio::test]
async fn durable_store_cross_actor_bind_is_indistinguishable_from_missing() {
    let (_, _, store) = test_store("bind-auth");
    let created = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "key", b"body"))
            .await,
    );
    let internal_refs = OpenAiCompatInternalRefs::new(
        OpenAiCompatProductActionRef::new("product-action:1").expect("valid product action ref"),
    );

    let denied = store
        .bind_internal_refs(OpenAiCompatBindInternalRefs::new(
            actor("tenant-a", "bob"),
            created.public_id.clone(),
            internal_refs,
        ))
        .await
        .expect("bind");

    assert!(denied.is_none());
    let loaded = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            created.public_id,
            OpenAiCompatRefOperation::Cancel,
        ))
        .await
        .expect("lookup")
        .expect("mapping exists");
    assert!(matches!(
        loaded.binding,
        OpenAiCompatResourceBinding::Pending
    ));
}

#[tokio::test]
async fn durable_store_retries_concurrent_same_key_to_single_mapping() {
    let (_, _, store) = test_store("concurrent");
    let left_request = reservation("tenant-a", "alice", "same-key", b"same body");
    let right_request = left_request.clone();

    let (left, right) = tokio::join!(store.reserve(left_request), store.reserve(right_request));
    let mappings = [left.expect("left"), right.expect("right")]
        .into_iter()
        .filter_map(|outcome| outcome.mapping().cloned())
        .collect::<Vec<_>>();

    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].public_id, mappings[1].public_id);
}

#[tokio::test]
async fn durable_store_rejects_corrupt_persisted_state() {
    let (filesystem, root, store) = test_store("corrupt");
    let public_id = OpenAiCompatPublicId::Response(
        OpenAiResponseId::new("resp_corrupt").expect("valid response id"),
    );
    let record_path = public_id_record_path(&root, &public_id);
    filesystem
        .put(
            &record_path,
            Entry::bytes(b"{ malformed json".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("write malformed state");

    let error = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            public_id,
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect_err("malformed state should fail closed");

    assert!(matches!(
        error,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError::CorruptMapping
    ));
}

#[tokio::test]
async fn durable_store_rejects_inconsistent_persisted_mapping() {
    let (filesystem, root, store) = test_store("inconsistent");
    let public_id =
        OpenAiCompatPublicId::Response(OpenAiResponseId::new("resp_unused").expect("id"));
    let record_path = public_id_record_path(&root, &public_id);
    let state = json!({
        "public_id": {"kind": "chat_completion", "id": "chatcmpl-valid"},
        "owner": {
            "tenant_id": "tenant-a",
            "user_id": "alice",
            "agent_id": "agent-a",
            "project_id": "project-a"
        },
        "surface": "responses_api",
        "request_fingerprint": OpenAiCompatRequestFingerprint::from_body_bytes(b"body"),
        "binding": {
            "state": "pending"
        }
    });
    let entry = Entry::record(
        RecordKind::new("openai_compat_ref_mapping").expect("valid record kind"),
        &state,
    )
    .expect("valid mapping record");
    filesystem
        .put(&record_path, entry, CasExpectation::Absent)
        .await
        .expect("write inconsistent mapping");

    let error = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            public_id,
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect_err("inconsistent state should fail closed");

    assert!(matches!(
        error,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError::CorruptMapping
    ));
}

#[tokio::test]
async fn durable_store_rejects_wrong_record_kind_for_persisted_mapping() {
    let (filesystem, root, store) = test_store("wrong-mapping-kind");
    let created = expect_created(
        store
            .reserve(reservation("tenant-a", "alice", "key", b"body"))
            .await,
    );
    let record_path = public_id_record_path(&root, &created.public_id);
    let payload = serde_json::to_value(&created).expect("mapping payload");
    let entry = Entry::record(
        RecordKind::new("wrong_openai_compat_mapping").expect("valid record kind"),
        &payload,
    )
    .expect("wrong-kind mapping record");
    filesystem
        .put(&record_path, entry, CasExpectation::Any)
        .await
        .expect("overwrite mapping with wrong kind");

    let error = store
        .lookup_authorized(OpenAiCompatRefLookup::new(
            actor("tenant-a", "alice"),
            created.public_id,
            OpenAiCompatRefOperation::Retrieve,
        ))
        .await
        .expect_err("wrong mapping kind should fail closed");

    assert!(matches!(
        error,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError::CorruptMapping
    ));
}

#[tokio::test]
async fn durable_store_rejects_wrong_record_kind_for_idempotency_index() {
    let (filesystem, root, store) = test_store("wrong-index-kind");
    let request = reservation("tenant-a", "alice", "same-key", b"same body");
    let created = expect_created(store.reserve(request.clone()).await);
    let owner = actor("tenant-a", "alice");
    let key = OpenAiCompatIdempotencyKey::new("same-key").expect("valid key");
    let index_path =
        idempotency_index_record_path(&root, &owner, OpenAiCompatRouteSurface::ResponsesApi, &key);
    let payload = json!({
        "owner": owner,
        "surface": "responses_api",
        "key": key,
        "public_id": created.public_id,
    });
    let entry = Entry::record(
        RecordKind::new("wrong_openai_compat_idempotency_index").expect("valid record kind"),
        &payload,
    )
    .expect("wrong-kind idempotency index");
    filesystem
        .put(&index_path, entry, CasExpectation::Any)
        .await
        .expect("overwrite index with wrong kind");

    let error = store
        .reserve(request)
        .await
        .expect_err("wrong idempotency index kind should fail closed");

    assert!(matches!(
        error,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError::CorruptMapping
    ));
}

#[tokio::test]
async fn durable_store_rejects_idempotency_index_pointing_to_other_actor_mapping() {
    let (filesystem, root, store) = test_store("stale-index");
    let alice_request = reservation("tenant-a", "alice", "same-key", b"same body");
    let _alice = expect_created(store.reserve(alice_request.clone()).await);
    let bob = expect_created(
        store
            .reserve(reservation("tenant-a", "bob", "bob-key", b"same body"))
            .await,
    );
    let alice = actor("tenant-a", "alice");
    let alice_key = OpenAiCompatIdempotencyKey::new("same-key").expect("valid key");
    let index_path = idempotency_index_record_path(
        &root,
        &alice,
        OpenAiCompatRouteSurface::ResponsesApi,
        &alice_key,
    );
    let payload = json!({
        "owner": alice,
        "surface": "responses_api",
        "key": alice_key,
        "public_id": bob.public_id,
    });
    let entry = Entry::record(
        RecordKind::new("openai_compat_idempotency_index").expect("valid record kind"),
        &payload,
    )
    .expect("stale idempotency index");
    filesystem
        .put(&index_path, entry, CasExpectation::Any)
        .await
        .expect("overwrite index with stale pointer");

    let error = store
        .reserve(alice_request)
        .await
        .expect_err("stale idempotency index should fail closed");

    assert!(matches!(
        error,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError::CorruptMapping
    ));
}

fn test_store(
    suffix: &str,
) -> (
    Arc<InMemoryBackend>,
    VirtualPath,
    FilesystemOpenAiCompatRefStore,
) {
    let filesystem = Arc::new(InMemoryBackend::new());
    let root = root_for(suffix);
    let store = FilesystemOpenAiCompatRefStore::with_root(filesystem.clone(), root.clone());
    (filesystem, root, store)
}

fn root_for(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!("/engine/openai_compat/test/{suffix}")).expect("valid test root")
}

fn public_id_record_path(root: &VirtualPath, public_id: &OpenAiCompatPublicId) -> VirtualPath {
    let (kind_dir, id) = match public_id {
        OpenAiCompatPublicId::ChatCompletion(id) => ("chat_completions", id.as_str()),
        OpenAiCompatPublicId::Response(id) => ("responses", id.as_str()),
    };
    VirtualPath::new(format!(
        "{}/by_public_id/{kind_dir}/{id}.json",
        root.as_str()
    ))
    .expect("valid public id record path")
}

fn idempotency_index_record_path(
    root: &VirtualPath,
    owner: &OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    key: &OpenAiCompatIdempotencyKey,
) -> VirtualPath {
    #[derive(serde::Serialize)]
    struct DigestInput {
        owner: OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        key: OpenAiCompatIdempotencyKey,
    }

    let digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&DigestInput {
            owner: owner.clone(),
            surface,
            key: key.clone(),
        })
        .expect("idempotency index digest payload"),
    ));
    let surface_dir = match surface {
        OpenAiCompatRouteSurface::ChatCompletions => "chat_completions",
        OpenAiCompatRouteSurface::ResponsesApi => "responses_api",
        OpenAiCompatRouteSurface::ResponsesV1 => "responses_v1",
    };
    VirtualPath::new(format!(
        "{}/by_idempotency/{surface_dir}/{digest}.json",
        root.as_str()
    ))
    .expect("valid idempotency index record path")
}

fn reservation(
    tenant_id: &str,
    user_id: &str,
    idempotency_key: &str,
    body: &[u8],
) -> OpenAiCompatRefReservation {
    OpenAiCompatRefReservation::new(
        actor(tenant_id, user_id),
        OpenAiCompatRouteSurface::ResponsesApi,
        OpenAiCompatRequestFingerprint::from_body_bytes(body),
        Some(OpenAiCompatIdempotencyKey::new(idempotency_key).expect("valid idempotency key")),
    )
}

fn actor(tenant_id: &str, user_id: &str) -> OpenAiCompatActorScope {
    OpenAiCompatActorScope::new(
        TenantId::new(tenant_id).expect("valid tenant"),
        UserId::new(user_id).expect("valid user"),
        Some(AgentId::new("agent-a").expect("valid agent")),
        Some(ProjectId::new("project-a").expect("valid project")),
    )
}

fn accepted_ack(message_ref: &str) -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new(message_ref).expect("valid accepted ref"),
        submitted_run_id: TurnRunId::new(),
    }
}

fn expect_created(
    result: Result<
        OpenAiCompatRefReservationOutcome,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError,
    >,
) -> ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping {
    match result.expect("reserve should not fail") {
        OpenAiCompatRefReservationOutcome::Created(mapping) => mapping,
        other => panic!("expected created mapping, got {other:?}"),
    }
}

fn expect_replayed(
    result: Result<
        OpenAiCompatRefReservationOutcome,
        ironclaw_reborn_openai_compat::OpenAiCompatRefError,
    >,
) -> ironclaw_reborn_openai_compat::OpenAiCompatResourceMapping {
    match result.expect("reserve should not fail") {
        OpenAiCompatRefReservationOutcome::Replayed(mapping) => mapping,
        other => panic!("expected replayed mapping, got {other:?}"),
    }
}
