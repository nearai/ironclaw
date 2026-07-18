//! Contract tests for [`GateRecordStore`] / [`FilesystemGateRecordStore`].
//!
//! The gate record is the model-visible content a pending gate renders from
//! (arch-simplification §5.2.9); it must survive from the turn that blocks to a
//! LATER resume turn, so it is persisted keyed by [`GateRef`]. These tests pin
//! the seam that result-side wiring depends on: save/load round-trips for every
//! [`GateRecord`] variant, a missing ref reads as `None`, a duplicate ref is
//! rejected write-once, and a record saved under one scope is not loadable under
//! another (the cross-tenant regression `database.md` / `safety-and-sandbox.md`
//! require).

use std::sync::Arc;

use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::*;
use ironclaw_run_state::*;

#[tokio::test]
async fn gate_record_store_round_trips_every_variant() {
    let store = in_mem_gate_record_store();
    let scope = sample_scope("tenant1", "user1");

    for record in every_gate_record_variant() {
        let gate_ref = GateRef::new();
        store
            .save(scope.clone(), gate_ref, record.clone())
            .await
            .unwrap();
        let loaded = store.load(&scope, gate_ref).await.unwrap();
        assert_eq!(
            loaded,
            Some(record.clone()),
            "{}: save then load must reconstruct the record",
            record.kind()
        );
    }
}

#[tokio::test]
async fn gate_record_auth_variant_round_trips_credential_requirements() {
    let store = in_mem_gate_record_store();
    let scope = sample_scope("tenant1", "user1");
    let gate_ref = GateRef::new();
    let record = GateRecord::Auth {
        summary: summary(),
        credential_requirements: vec![credential_requirement()],
    };

    store
        .save(scope.clone(), gate_ref, record.clone())
        .await
        .unwrap();

    // The host-owned credential requirement is rendered FROM the record on a
    // later resume turn, never reconstructed from model-visible data.
    match store.load(&scope, gate_ref).await.unwrap() {
        Some(GateRecord::Auth {
            credential_requirements,
            ..
        }) => {
            assert_eq!(credential_requirements, vec![credential_requirement()]);
        }
        other => panic!("expected Auth gate record, got {other:?}"),
    }
}

/// The reason this store exists: a gate blocks on one turn and is rendered on a
/// LATER resume turn, which carries a fresh `invocation_id`. The scope check
/// must compare only the owner axes (tenant/user/agent/project/mission/thread)
/// — if it ever included `invocation_id`, every legitimate resume load would
/// look unknown and this test would fail.
#[tokio::test]
async fn gate_record_loads_on_a_later_resume_turn_with_a_new_invocation_id() {
    let store = in_mem_gate_record_store();
    let blocking_turn = sample_scope("tenant1", "user1");
    let gate_ref = GateRef::new();
    store
        .save(
            blocking_turn.clone(),
            gate_ref,
            GateRecord::Approval { summary: summary() },
        )
        .await
        .unwrap();

    let resume_turn = ResourceScope {
        invocation_id: InvocationId::new(),
        ..blocking_turn
    };
    assert_eq!(
        store.load(&resume_turn, gate_ref).await.unwrap(),
        Some(GateRecord::Approval { summary: summary() }),
        "same owner on a later turn (new invocation_id) must still load the record"
    );
}

#[tokio::test]
async fn gate_record_load_of_missing_ref_returns_none() {
    let store = in_mem_gate_record_store();
    let scope = sample_scope("tenant1", "user1");

    let loaded = store.load(&scope, GateRef::new()).await.unwrap();

    assert_eq!(loaded, None);
}

#[tokio::test]
async fn gate_record_save_is_write_once() {
    let store = in_mem_gate_record_store();
    let scope = sample_scope("tenant1", "user1");
    let gate_ref = GateRef::new();
    store
        .save(
            scope.clone(),
            gate_ref,
            GateRecord::Approval { summary: summary() },
        )
        .await
        .unwrap();

    let err = store
        .save(
            scope.clone(),
            gate_ref,
            GateRecord::Resource { summary: summary() },
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        RunStateError::GateRecordAlreadyExists { gate_ref: g } if g == gate_ref
    ));
    // The original write-once record is intact — the rejected save did not clobber it.
    assert_eq!(
        store.load(&scope, gate_ref).await.unwrap(),
        Some(GateRecord::Approval { summary: summary() })
    );
}

#[tokio::test]
async fn gate_record_load_is_scoped_to_tenant_and_user() {
    let store = in_mem_gate_record_store();
    let gate_ref = GateRef::new();
    let tenant_a = sample_scope("tenant1", "user1");
    let tenant_b = sample_scope("tenant2", "user1");

    store
        .save(
            tenant_a.clone(),
            gate_ref,
            GateRecord::Approval { summary: summary() },
        )
        .await
        .unwrap();

    // A record saved under tenant A must not be loadable under tenant B.
    assert_eq!(store.load(&tenant_b, gate_ref).await.unwrap(), None);
    // The owner still sees it.
    assert_eq!(
        store.load(&tenant_a, gate_ref).await.unwrap(),
        Some(GateRecord::Approval { summary: summary() })
    );
}

#[tokio::test]
async fn gate_record_load_is_scoped_to_within_tenant_axes() {
    // Same tenant/user, different project — path-level within-tenant isolation.
    let store = in_mem_gate_record_store();
    let gate_ref = GateRef::new();
    let project_a = scope_with_project("tenant1", "user1", "project-a");
    let project_b = scope_with_project("tenant1", "user1", "project-b");

    store
        .save(
            project_a.clone(),
            gate_ref,
            GateRecord::Approval { summary: summary() },
        )
        .await
        .unwrap();

    assert_eq!(store.load(&project_b, gate_ref).await.unwrap(), None);
    assert_eq!(
        store.load(&project_a, gate_ref).await.unwrap(),
        Some(GateRecord::Approval { summary: summary() })
    );
}

fn every_gate_record_variant() -> Vec<GateRecord> {
    let result = ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap();
    vec![
        GateRecord::Approval { summary: summary() },
        GateRecord::Auth {
            summary: summary(),
            credential_requirements: vec![credential_requirement()],
        },
        GateRecord::Resource { summary: summary() },
        GateRecord::DependentRun {
            summary: summary(),
            result,
            byte_len: 2048,
        },
        GateRecord::ExternalTool { summary: summary() },
    ]
}

fn summary() -> SafeSummary {
    SafeSummary::new("awaiting decision").unwrap()
}

fn credential_requirement() -> RuntimeCredentialAuthRequirement {
    RuntimeCredentialAuthRequirement {
        provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
        setup: RuntimeCredentialAccountSetup::ManualToken,
        requester_extension: ExtensionId::new("github").unwrap(),
        provider_scopes: vec!["repo".to_string()],
    }
}

fn engine_filesystem() -> ironclaw_filesystem::InMemoryBackend {
    ironclaw_filesystem::InMemoryBackend::new()
}

/// The production gate-record store over a fresh in-memory backend.
fn in_mem_gate_record_store() -> FilesystemGateRecordStore<ironclaw_filesystem::InMemoryBackend> {
    FilesystemGateRecordStore::new(scoped_gate_record_fs(Arc::new(engine_filesystem())))
}

/// Build a [`ScopedFilesystem`] exposing the `/gate-records` alias under a single
/// tenant/user subtree of the underlying mount — mirrors the production shape
/// where one `MountView` covers a consumer alias for a given tenant/user.
fn scoped_gate_record_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/gate-records").expect("alias"),
        VirtualPath::new("/engine/tenants/test-tenant/users/test-user/gate-records")
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
