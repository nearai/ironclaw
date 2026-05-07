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

async fn libsql_store() -> LibSqlCapabilityLeaseStore {
    let dir = tempfile::tempdir().unwrap().keep();
    let db = Arc::new(
        libsql::Builder::new_local(dir.join("capability-leases.db"))
            .build()
            .await
            .unwrap(),
    );
    let store = LibSqlCapabilityLeaseStore::new(db);
    store.run_migrations().await.unwrap();
    store
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
