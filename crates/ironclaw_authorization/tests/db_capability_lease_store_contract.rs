#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_authorization::{
    CapabilityLease, CapabilityLeaseError, CapabilityLeaseStatus, CapabilityLeaseStore,
    LibSqlCapabilityLeaseStore,
};
use ironclaw_host_api::*;

#[tokio::test]
async fn libsql_capability_lease_store_persists_and_reloads_issued_leases() {
    let store = libsql_store().await;
    let context = execution_context(CapabilitySet::default());
    let lease = lease_for(&context);
    let lease_id = lease.grant.id;

    store.issue(lease.clone()).await.unwrap();
    assert_eq!(
        store.get(&context.resource_scope, lease_id).await,
        Some(lease)
    );
    assert_eq!(
        store.leases_for_scope(&context.resource_scope).await.len(),
        1
    );
}

#[tokio::test]
async fn libsql_capability_lease_store_persists_revoke_claim_and_consume() {
    let store = libsql_store().await;
    let context = execution_context(CapabilitySet::default());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &capability_id(),
        &ResourceEstimate::default(),
        &serde_json::json!({"message":"approved"}),
    )
    .unwrap();
    let mut lease = lease_for(&context);
    lease.invocation_fingerprint = Some(fingerprint.clone());
    lease.grant.constraints.max_invocations = Some(1);
    let lease_id = lease.grant.id;
    store.issue(lease).await.unwrap();

    let claimed = store
        .claim(&context.resource_scope, lease_id, &fingerprint)
        .await
        .unwrap();
    assert_eq!(claimed.status, CapabilityLeaseStatus::Claimed);

    let consumed = store
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap();
    assert_eq!(consumed.status, CapabilityLeaseStatus::Consumed);
    assert_eq!(consumed.grant.constraints.max_invocations, Some(0));

    let revoked = store
        .revoke(&context.resource_scope, lease_id)
        .await
        .unwrap();
    assert_eq!(revoked.status, CapabilityLeaseStatus::Revoked);
}

#[tokio::test]
async fn libsql_capability_lease_store_preserves_fingerprint_claim_guard() {
    let store = libsql_store().await;
    let context = execution_context(CapabilitySet::default());
    let fingerprint = InvocationFingerprint::for_dispatch(
        &context.resource_scope,
        &capability_id(),
        &ResourceEstimate::default(),
        &serde_json::json!({"message":"approved"}),
    )
    .unwrap();
    let mut lease = lease_for(&context);
    lease.invocation_fingerprint = Some(fingerprint);
    let lease_id = lease.grant.id;
    store.issue(lease).await.unwrap();

    let err = store
        .consume(&context.resource_scope, lease_id)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        CapabilityLeaseError::UnclaimedFingerprintLease { lease_id: id } if id == lease_id
    ));
}

#[tokio::test]
async fn libsql_capability_lease_store_ignores_non_canonical_lease_id_rows() {
    let (store, db) = libsql_store_with_db().await;
    let context = execution_context(CapabilitySet::default());
    let lease = lease_for(&context);
    let canonical_lease_id = lease.grant.id.to_string();
    let non_canonical_lease_id = canonical_lease_id.to_uppercase();
    assert_ne!(canonical_lease_id, non_canonical_lease_id);

    let conn = db.connect().unwrap();
    conn.execute(
        "INSERT INTO reborn_capability_lease_records (owner_key, invocation_id, lease_id, status, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
        libsql::params![
            owner_key(&context.resource_scope),
            context.resource_scope.invocation_id.to_string(),
            non_canonical_lease_id,
            "active",
            serde_json::to_string(&lease).unwrap(),
        ],
    )
    .await
    .unwrap();

    assert!(
        store
            .leases_for_scope(&context.resource_scope)
            .await
            .is_empty(),
        "non-canonical row keys must not produce authorizing leases that exact consume/revoke lookups cannot find"
    );
}

async fn libsql_store() -> LibSqlCapabilityLeaseStore {
    libsql_store_with_db().await.0
}

async fn libsql_store_with_db() -> (LibSqlCapabilityLeaseStore, Arc<libsql::Database>) {
    let dir = tempfile::tempdir().unwrap().keep();
    let db = Arc::new(
        libsql::Builder::new_local(dir.join("capability-leases.db"))
            .build()
            .await
            .unwrap(),
    );
    let store = LibSqlCapabilityLeaseStore::new(Arc::clone(&db));
    store.run_migrations().await.unwrap();
    (store, db)
}

fn owner_key(scope: &ResourceScope) -> String {
    #[derive(serde::Serialize)]
    struct OwnerKey<'a> {
        tenant_id: &'a str,
        user_id: &'a str,
        agent_id: Option<&'a str>,
        project_id: Option<&'a str>,
        mission_id: Option<&'a str>,
        thread_id: Option<&'a str>,
    }
    serde_json::to_string(&OwnerKey {
        tenant_id: scope.tenant_id.as_str(),
        user_id: scope.user_id.as_str(),
        agent_id: scope.agent_id.as_ref().map(|id| id.as_str()),
        project_id: scope.project_id.as_ref().map(|id| id.as_str()),
        mission_id: scope.mission_id.as_ref().map(|id| id.as_str()),
        thread_id: scope.thread_id.as_ref().map(|id| id.as_str()),
    })
    .unwrap()
}

fn lease_for(context: &ExecutionContext) -> CapabilityLease {
    CapabilityLease::new(
        context.resource_scope.clone(),
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability_id(),
            grantee: Principal::Extension(context.extension_id.clone()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        },
    )
}

fn execution_context(grants: CapabilitySet) -> ExecutionContext {
    let invocation_id = InvocationId::new();
    let resource_scope = ResourceScope {
        tenant_id: TenantId::new("tenant1").unwrap(),
        user_id: UserId::new("user1").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        extension_id: ExtensionId::new("caller").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::Sandbox,
        grants,
        mounts: MountView::default(),
        resource_scope,
    }
}

fn capability_id() -> CapabilityId {
    CapabilityId::new("echo.say").unwrap()
}
