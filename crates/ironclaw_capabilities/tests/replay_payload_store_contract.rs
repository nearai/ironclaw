//! Contract tests for [`ReplayPayloadStore`] / [`FilesystemReplayPayloadStore`].
//!
//! The replay payload is the **host-private** raw capability input + estimate a
//! gate/auth resume replays through (arch-simplification §5.3). It is the exact
//! opposite of a `GateRecord`: it must NEVER be model-visible — it carries the
//! raw tool `input` JSON and `ResourceEstimate` that today ride in-band through
//! the untrusted loop checkpoint. Moving it host-side (keyed by `InvocationId`)
//! retires that exposure, so these tests pin the seam a later resume-read slice
//! depends on: an all-fields round-trip (proving raw `input`/`estimate` survive),
//! the auth-without-prior-approval shape, a missing key reads as `None`, a
//! duplicate key is rejected write-once, and a payload saved under one scope is
//! not loadable under another (the cross-tenant + within-tenant regression
//! `database.md` / `safety-and-sandbox.md` require).
//!
//! Mirrors `ironclaw_run_state`'s `gate_record_store_contract.rs`.

use std::sync::Arc;

use ironclaw_capabilities::{
    FilesystemReplayPayloadStore, ReplayPayload, ReplayPayloadStore, ReplayPayloadStoreError,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ApprovalRequestId, CorrelationId, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, ResourceEstimate, ResourceScope, TenantId, UserId, VirtualPath,
};
use ironclaw_turns::run_profile::{AuthResumeApprovalIdentity, CapabilityInputRef};

#[tokio::test]
async fn replay_payload_round_trips_all_fields() {
    let store = in_mem_replay_payload_store();
    let scope = sample_scope("tenant1", "user1");
    let invocation_id = InvocationId::new();
    // A payload that populated every field: raw input JSON, a non-trivial
    // estimate, a prior-approval identity, an input ref, and a correlation id.
    let payload = payload_with_prior_approval();

    store
        .save(scope.clone(), invocation_id, payload.clone())
        .await
        .unwrap();

    let loaded = store.load(&scope, invocation_id).await.unwrap();
    assert_eq!(
        loaded,
        Some(payload),
        "save then load must reconstruct every field, including the raw input and estimate"
    );
}

#[tokio::test]
async fn replay_payload_without_prior_approval_round_trips() {
    // The auth-resume-without-approval shape: `prior_approval` is `None`. This is
    // a distinct scenario from the all-fields case (the auth gate that never
    // passed an approval gate), so it gets its own assertion rather than being
    // folded into the round-trip above.
    let store = in_mem_replay_payload_store();
    let scope = sample_scope("tenant1", "user1");
    let invocation_id = InvocationId::new();
    let payload = payload_without_prior_approval();

    store
        .save(scope.clone(), invocation_id, payload.clone())
        .await
        .unwrap();

    match store.load(&scope, invocation_id).await.unwrap() {
        Some(loaded) => {
            assert_eq!(loaded.prior_approval, None);
            assert_eq!(loaded, payload);
        }
        None => panic!("expected the saved payload to load"),
    }
}

#[tokio::test]
async fn replay_payload_load_of_missing_returns_none() {
    let store = in_mem_replay_payload_store();
    let scope = sample_scope("tenant1", "user1");

    let loaded = store.load(&scope, InvocationId::new()).await.unwrap();

    assert_eq!(loaded, None);
}

#[tokio::test]
async fn replay_payload_save_is_write_once() {
    let store = in_mem_replay_payload_store();
    let scope = sample_scope("tenant1", "user1");
    let invocation_id = InvocationId::new();
    let first = payload_with_prior_approval();
    store
        .save(scope.clone(), invocation_id, first.clone())
        .await
        .unwrap();

    let err = store
        .save(
            scope.clone(),
            invocation_id,
            payload_without_prior_approval(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ReplayPayloadStoreError::ReplayPayloadAlreadyExists { invocation_id: id } if id == invocation_id
    ));
    // The original write-once payload is intact — the rejected save did not clobber it.
    assert_eq!(
        store.load(&scope, invocation_id).await.unwrap(),
        Some(first)
    );
}

#[tokio::test]
async fn replay_payload_load_is_scoped_to_tenant_and_user() {
    let store = in_mem_replay_payload_store();
    let invocation_id = InvocationId::new();
    let tenant_a = sample_scope("tenant1", "user1");
    let tenant_b = sample_scope("tenant2", "user1");
    let payload = payload_with_prior_approval();

    store
        .save(tenant_a.clone(), invocation_id, payload.clone())
        .await
        .unwrap();

    // A payload saved under tenant A must not be loadable under tenant B.
    assert_eq!(store.load(&tenant_b, invocation_id).await.unwrap(), None);
    // The owner still sees it.
    assert_eq!(
        store.load(&tenant_a, invocation_id).await.unwrap(),
        Some(payload)
    );
}

#[tokio::test]
async fn replay_payload_load_is_scoped_to_within_tenant_axes() {
    // Same tenant/user, different project — path-level within-tenant isolation.
    let store = in_mem_replay_payload_store();
    let invocation_id = InvocationId::new();
    let project_a = scope_with_project("tenant1", "user1", "project-a");
    let project_b = scope_with_project("tenant1", "user1", "project-b");
    let payload = payload_with_prior_approval();

    store
        .save(project_a.clone(), invocation_id, payload.clone())
        .await
        .unwrap();

    assert_eq!(store.load(&project_b, invocation_id).await.unwrap(), None);
    assert_eq!(
        store.load(&project_a, invocation_id).await.unwrap(),
        Some(payload)
    );
}

fn payload_with_prior_approval() -> ReplayPayload {
    ReplayPayload {
        input: serde_json::json!({
            "path": "/etc/hosts",
            "mode": "read",
            "nested": { "count": 3, "flag": true },
        }),
        estimate: ResourceEstimate::default()
            .set_input_tokens(1_200)
            .set_wall_clock_ms(750)
            .set_output_bytes(4_096),
        prior_approval: Some(AuthResumeApprovalIdentity {
            approval_request_id: ApprovalRequestId::new(),
            correlation_id: CorrelationId::new(),
        }),
        input_ref: CapabilityInputRef::new("input:round-trip-fixture").unwrap(),
        correlation_id: CorrelationId::new(),
    }
}

fn payload_without_prior_approval() -> ReplayPayload {
    ReplayPayload {
        input: serde_json::json!({ "query": "select 1" }),
        estimate: ResourceEstimate::default().set_process_count(1),
        prior_approval: None,
        input_ref: CapabilityInputRef::new("input:no-approval-fixture").unwrap(),
        correlation_id: CorrelationId::new(),
    }
}

/// The production replay-payload store over a fresh in-memory backend.
fn in_mem_replay_payload_store() -> FilesystemReplayPayloadStore<InMemoryBackend> {
    FilesystemReplayPayloadStore::new(scoped_replay_payload_fs(Arc::new(InMemoryBackend::new())))
}

/// Build a [`ScopedFilesystem`] exposing the `/replay-payloads` alias under a
/// single tenant/user subtree of the underlying mount — mirrors the production
/// shape where one `MountView` covers a consumer alias for a given tenant/user.
fn scoped_replay_payload_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/replay-payloads").expect("alias"),
        VirtualPath::new("/engine/tenants/test-tenant/users/test-user/replay-payloads")
            .expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn scope_with_project(tenant: &str, user: &str, project: &str) -> ResourceScope {
    ResourceScope {
        project_id: Some(ProjectId::new(project).unwrap()),
        ..sample_scope(tenant, user)
    }
}
