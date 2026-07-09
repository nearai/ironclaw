use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use ironclaw_approvals::{
    CapabilityAuthorityCleanupError, CapabilityPermissionOverride,
    CapabilityPermissionOverrideInput, CapabilityPermissionOverrideKey,
    CapabilityPermissionOverrideRecord, CapabilityPermissionOverrideStore,
    CapabilityPermissionStoreError, InMemoryCapabilityPermissionOverrideStore,
    InMemoryPersistentApprovalPolicyStore, PersistentApprovalAction, PersistentApprovalPolicy,
    PersistentApprovalPolicyError, PersistentApprovalPolicyInput, PersistentApprovalPolicyKey,
    PersistentApprovalPolicyStore, cleanup_capability_authority,
};
use ironclaw_host_api::{
    AgentId, ApprovalRequestId, CapabilityId, EffectKind, GrantConstraints, InvocationId,
    MountView, NetworkPolicy, Principal, ProjectId, ResourceScope, TenantId, UserId,
};

#[tokio::test]
async fn exact_cleanup_revokes_only_matching_dispatch_authority_and_clears_matching_overrides() {
    let policies = InMemoryPersistentApprovalPolicyStore::new();
    let overrides = InMemoryCapabilityPermissionOverrideStore::new();
    let request_scope = ResourceScope {
        agent_id: Some(AgentId::new("agent-a").expect("valid agent")),
        project_id: Some(ProjectId::new("project-a").expect("valid project")),
        ..scope("owner-a")
    };
    let scope_a = request_scope.tenant_user_settings_scope();
    let other_scope = scope("owner-b");
    let extension_grantee = extension_grantee();
    let other_grantee = Principal::User(UserId::new("owner-a").expect("valid user"));
    let target = capability("mcp-acme.search");
    let unrelated = capability("mcp-acme.create");

    seed_policy(
        &policies,
        &scope_a,
        PersistentApprovalAction::Dispatch,
        &target,
        &extension_grantee,
    )
    .await;
    seed_policy(
        &policies,
        &scope_a,
        PersistentApprovalAction::Dispatch,
        &unrelated,
        &extension_grantee,
    )
    .await;
    seed_policy(
        &policies,
        &scope_a,
        PersistentApprovalAction::Dispatch,
        &target,
        &other_grantee,
    )
    .await;
    seed_policy(
        &policies,
        &scope_a,
        PersistentApprovalAction::SpawnCapability,
        &target,
        &extension_grantee,
    )
    .await;
    seed_policy(
        &policies,
        &other_scope,
        PersistentApprovalAction::Dispatch,
        &target,
        &extension_grantee,
    )
    .await;
    seed_override(&overrides, &scope_a, &target).await;
    seed_override(&overrides, &scope_a, &unrelated).await;
    seed_override(&overrides, &other_scope, &target).await;

    cleanup_capability_authority(
        &policies,
        &overrides,
        &request_scope,
        &extension_grantee,
        &[target.clone(), capability("mcp-acme.missing")],
    )
    .await
    .expect("exact cleanup");

    let target_policy = policies
        .lookup(&policy_key(
            &scope_a,
            PersistentApprovalAction::Dispatch,
            &target,
            &extension_grantee,
        ))
        .await
        .expect("target lookup")
        .expect("target policy");
    assert!(
        target_policy.revoked_at.is_some(),
        "target dispatch revoked"
    );
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(
                &scope_a,
                target.clone()
            ))
            .await
            .expect("target override lookup")
            .is_none(),
        "target override cleared"
    );

    for key in [
        policy_key(
            &scope_a,
            PersistentApprovalAction::Dispatch,
            &unrelated,
            &extension_grantee,
        ),
        policy_key(
            &scope_a,
            PersistentApprovalAction::Dispatch,
            &target,
            &other_grantee,
        ),
        policy_key(
            &scope_a,
            PersistentApprovalAction::SpawnCapability,
            &target,
            &extension_grantee,
        ),
        policy_key(
            &other_scope,
            PersistentApprovalAction::Dispatch,
            &target,
            &extension_grantee,
        ),
    ] {
        let policy = policies
            .lookup(&key)
            .await
            .expect("unrelated lookup")
            .expect("unrelated policy");
        assert!(policy.revoked_at.is_none(), "unrelated policy preserved");
    }
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(&scope_a, unrelated))
            .await
            .expect("unrelated override lookup")
            .is_some(),
        "unrelated capability override preserved"
    );
    assert!(
        overrides
            .get(&CapabilityPermissionOverrideKey::new(&other_scope, target))
            .await
            .expect("other owner override lookup")
            .is_some(),
        "other owner override preserved"
    );
}

#[tokio::test]
async fn policy_store_failure_stops_before_override_cleanup() {
    let clear_calls = Arc::new(AtomicUsize::new(0));
    let overrides = CountingOverrideStore {
        clear_calls: Arc::clone(&clear_calls),
        clear_error: None,
    };
    let error = cleanup_capability_authority(
        &FailingPolicyStore,
        &overrides,
        &scope("owner-a"),
        &extension_grantee(),
        &[capability("mcp-acme.search")],
    )
    .await
    .expect_err("policy failure must fail closed");

    assert!(matches!(
        error,
        CapabilityAuthorityCleanupError::Policy(PersistentApprovalPolicyError::Filesystem(_))
    ));
    assert_eq!(
        clear_calls.load(Ordering::SeqCst),
        0,
        "override cleanup must not run after policy revoke fails"
    );
}

#[tokio::test]
async fn override_store_failure_is_returned_after_missing_policy_is_treated_as_success() {
    let clear_calls = Arc::new(AtomicUsize::new(0));
    let overrides = CountingOverrideStore {
        clear_calls: Arc::clone(&clear_calls),
        clear_error: Some(CapabilityPermissionStoreError::Filesystem(
            "injected override failure".to_string(),
        )),
    };
    let error = cleanup_capability_authority(
        &InMemoryPersistentApprovalPolicyStore::new(),
        &overrides,
        &scope("owner-a"),
        &extension_grantee(),
        &[capability("mcp-acme.search")],
    )
    .await
    .expect_err("override failure must fail closed");

    assert!(matches!(
        error,
        CapabilityAuthorityCleanupError::Override(CapabilityPermissionStoreError::Filesystem(_))
    ));
    assert_eq!(clear_calls.load(Ordering::SeqCst), 1);
}

fn capability(value: &str) -> CapabilityId {
    CapabilityId::new(value).expect("valid capability id")
}

fn extension_grantee() -> Principal {
    Principal::Extension(
        ironclaw_host_api::ExtensionId::new("mcp-acme").expect("valid extension id"),
    )
}

fn scope(user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("valid tenant"),
        user_id: UserId::new(user).expect("valid user"),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn policy_key(
    scope: &ResourceScope,
    action: PersistentApprovalAction,
    capability_id: &CapabilityId,
    grantee: &Principal,
) -> PersistentApprovalPolicyKey {
    PersistentApprovalPolicyKey::new(scope, action, capability_id.clone(), grantee.clone())
}

async fn seed_policy(
    store: &dyn PersistentApprovalPolicyStore,
    scope: &ResourceScope,
    action: PersistentApprovalAction,
    capability_id: &CapabilityId,
    grantee: &Principal,
) {
    store
        .allow(PersistentApprovalPolicyInput {
            scope: scope.clone(),
            action,
            capability_id: capability_id.clone(),
            grantee: grantee.clone(),
            approved_by: Principal::User(scope.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: Some(ApprovalRequestId::new()),
        })
        .await
        .expect("seed policy");
}

async fn seed_override(
    store: &dyn CapabilityPermissionOverrideStore,
    scope: &ResourceScope,
    capability_id: &CapabilityId,
) {
    store
        .set(CapabilityPermissionOverrideInput {
            scope: scope.clone(),
            capability_id: capability_id.clone(),
            state: CapabilityPermissionOverride::AskEachTime,
            updated_by: Principal::User(scope.user_id.clone()),
        })
        .await
        .expect("seed override");
}

struct FailingPolicyStore;

#[async_trait]
impl PersistentApprovalPolicyStore for FailingPolicyStore {
    async fn allow(
        &self,
        _input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        unreachable!("cleanup does not allow policies")
    }

    async fn lookup(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        unreachable!("cleanup revokes by exact key")
    }

    async fn revoke(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "injected policy failure".to_string(),
        ))
    }

    async fn revoke_if_source_approval_request(
        &self,
        _key: &PersistentApprovalPolicyKey,
        _source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        unreachable!("cleanup uses exact unconditional revocation")
    }
}

struct CountingOverrideStore {
    clear_calls: Arc<AtomicUsize>,
    clear_error: Option<CapabilityPermissionStoreError>,
}

#[async_trait]
impl CapabilityPermissionOverrideStore for CountingOverrideStore {
    async fn set(
        &self,
        _input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError> {
        unreachable!("cleanup does not set overrides")
    }

    async fn get(
        &self,
        _key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        unreachable!("cleanup clears by exact key")
    }

    async fn clear(
        &self,
        _key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        match &self.clear_error {
            Some(error) => Err(CapabilityPermissionStoreError::Filesystem(
                error.to_string(),
            )),
            None => Ok(()),
        }
    }
}
