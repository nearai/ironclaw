use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use ironclaw_approvals::{
    AutoApproveSettingInput, AutoApproveSettingKey, AutoApproveSettingRecord,
    AutoApproveSettingStore, CapabilityPermissionOverrideInput, CapabilityPermissionOverrideKey,
    CapabilityPermissionOverrideRecord, CapabilityPermissionOverrideStore,
    CapabilityPermissionStoreError, InMemoryAutoApproveSettingStore,
    InMemoryPersistentApprovalPolicyStore, InMemoryToolPermissionOverrideStore,
    PersistentApprovalPolicy, PersistentApprovalPolicyError, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey,
};
use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityId, EffectKind, ExecutionContext, ExtensionId, MountView,
    PermissionMode, Principal, ResourceEstimate, RuntimeKind, Timestamp, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, PROFILE_SET_CAPABILITY_ID, TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::json;

use super::*;
use crate::local_dev_capability_policy::local_dev_capability_policy;

struct ErroringToolPermissionOverrideStore;

struct CountingAutoApproveSettingStore {
    enabled: bool,
    gets: AtomicUsize,
    delay: Duration,
}

struct CountingToolPermissionOverrideStore {
    inner: InMemoryToolPermissionOverrideStore,
    gets: AtomicUsize,
    lists: AtomicUsize,
    delay: Duration,
}

struct CountingPersistentApprovalPolicyStore {
    inner: InMemoryPersistentApprovalPolicyStore,
    lookups: AtomicUsize,
    lists: AtomicUsize,
    delay: Duration,
}

impl CountingAutoApproveSettingStore {
    fn enabled() -> Self {
        Self {
            enabled: true,
            gets: AtomicUsize::new(0),
            delay: Duration::ZERO,
        }
    }

    fn enabled_with_delay(delay: Duration) -> Self {
        Self {
            enabled: true,
            gets: AtomicUsize::new(0),
            delay,
        }
    }

    fn get_count(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

impl CountingToolPermissionOverrideStore {
    fn with_delay(delay: Duration) -> Self {
        Self {
            inner: InMemoryToolPermissionOverrideStore::new(),
            gets: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            delay,
        }
    }

    fn get_count(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }

    fn list_count(&self) -> usize {
        self.lists.load(Ordering::SeqCst)
    }
}

impl CountingPersistentApprovalPolicyStore {
    fn with_delay(delay: Duration) -> Self {
        Self {
            inner: InMemoryPersistentApprovalPolicyStore::new(),
            lookups: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            delay,
        }
    }

    fn lookup_count(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }

    fn list_count(&self) -> usize {
        self.lists.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl AutoApproveSettingStore for CountingAutoApproveSettingStore {
    async fn set(
        &self,
        input: AutoApproveSettingInput,
    ) -> Result<AutoApproveSettingRecord, CapabilityPermissionStoreError> {
        let key = AutoApproveSettingKey::from_resource_scope(&input.scope);
        let now: Timestamp = chrono::Utc::now();
        Ok(AutoApproveSettingRecord {
            key,
            enabled: input.enabled,
            updated_by: input.updated_by,
            created_at: now,
            updated_at: now,
        })
    }

    async fn get(
        &self,
        key: &AutoApproveSettingKey,
    ) -> Result<Option<AutoApproveSettingRecord>, CapabilityPermissionStoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        let now: Timestamp = chrono::Utc::now();
        Ok(Some(AutoApproveSettingRecord {
            key: key.clone(),
            enabled: self.enabled,
            updated_by: Principal::HostRuntime,
            created_at: now,
            updated_at: now,
        }))
    }
}

#[async_trait::async_trait]
impl CapabilityPermissionOverrideStore for CountingToolPermissionOverrideStore {
    async fn set(
        &self,
        input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError> {
        self.inner.set(input).await
    }

    async fn get(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(key).await
    }

    fn supports_scope_listing(&self) -> bool {
        true
    }

    async fn list_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.inner.list_for_scope(scope).await
    }

    async fn clear(
        &self,
        key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError> {
        self.inner.clear(key).await
    }
}

#[async_trait::async_trait]
impl ironclaw_approvals::PersistentApprovalPolicyStore for CountingPersistentApprovalPolicyStore {
    async fn allow(
        &self,
        input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        self.inner.allow(input).await
    }

    async fn lookup(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.lookup(key).await
    }

    fn supports_scope_listing(&self) -> bool {
        true
    }

    async fn list_for_scope_action(
        &self,
        scope: &ResourceScope,
        action: PersistentApprovalAction,
    ) -> Result<Vec<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.inner.list_for_scope_action(scope, action).await
    }

    async fn revoke(
        &self,
        key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        self.inner.revoke(key).await
    }

    async fn revoke_if_source_approval_request(
        &self,
        key: &PersistentApprovalPolicyKey,
        source_approval_request_id: ironclaw_host_api::ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        self.inner
            .revoke_if_source_approval_request(key, source_approval_request_id)
            .await
    }
}

#[async_trait::async_trait]
impl CapabilityPermissionOverrideStore for ErroringToolPermissionOverrideStore {
    async fn set(
        &self,
        _input: CapabilityPermissionOverrideInput,
    ) -> Result<CapabilityPermissionOverrideRecord, CapabilityPermissionStoreError> {
        Err(CapabilityPermissionStoreError::Filesystem(
            "injected override store failure".to_string(),
        ))
    }

    async fn get(
        &self,
        _key: &CapabilityPermissionOverrideKey,
    ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError> {
        Err(CapabilityPermissionStoreError::Filesystem(
            "injected override store failure".to_string(),
        ))
    }

    async fn clear(
        &self,
        _key: &CapabilityPermissionOverrideKey,
    ) -> Result<(), CapabilityPermissionStoreError> {
        Err(CapabilityPermissionStoreError::Filesystem(
            "injected override store failure".to_string(),
        ))
    }
}

async fn local_dev_shell_decision_with_authorizer(
    authorizer: &dyn TrustAwareCapabilityDispatchAuthorizer,
    scope_user: &UserId,
) -> ironclaw_host_api::Decision {
    let (descriptor, context, trust_decision) = local_dev_shell_authorization_inputs(scope_user);
    authorizer
        .authorize_dispatch_with_trust(
            &context,
            &descriptor,
            &ResourceEstimate::default(),
            &trust_decision,
        )
        .await
}

/// `local_dev_shell_authorization_inputs` with the descriptor's manifest
/// `default_permission` overridden, so a test can drive a manifest-ineligible
/// tool (`PermissionMode::Deny`) through the real store-backed gate.
fn local_dev_shell_authorization_inputs_with_permission(
    scope_user: &UserId,
    permission: PermissionMode,
) -> (CapabilityDescriptor, ExecutionContext, TrustDecision) {
    let (mut descriptor, context, trust_decision) =
        local_dev_shell_authorization_inputs(scope_user);
    descriptor.default_permission = permission;
    (descriptor, context, trust_decision)
}

async fn enable_global_auto_approve(store: &InMemoryAutoApproveSettingStore, user_id: &UserId) {
    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id.clone(),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    store
        .set(AutoApproveSettingInput {
            scope,
            enabled: true,
            updated_by: Principal::User(user_id.clone()),
        })
        .await
        .expect("auto-approve setting update");
}

async fn seed_shell_tool_override(
    store: &InMemoryToolPermissionOverrideStore,
    user_id: &UserId,
    state: ToolPermissionOverride,
) {
    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id.clone(),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    // Mirror the production lookup, which keys the override on
    // `operator_tool_permission_scope` (agent/project stripped). Seeding
    // through the raw scope would produce a non-matching key.
    store
        .set(CapabilityPermissionOverrideInput {
            scope: operator_tool_permission_scope(&scope),
            capability_id: CapabilityId::new("builtin.shell").expect("capability id"),
            state,
            updated_by: Principal::User(user_id.clone()),
        })
        .await
        .expect("tool override set");
}

fn local_dev_shell_authorization_inputs(
    scope_user: &UserId,
) -> (CapabilityDescriptor, ExecutionContext, TrustDecision) {
    let capability_id = CapabilityId::new("builtin.shell").expect("capability id");
    let provider_id = ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id");
    let effects = vec![EffectKind::SpawnProcess];
    let descriptor = CapabilityDescriptor {
        id: capability_id,
        provider: provider_id.clone(),
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::UserTrusted,
        description: "test".to_string(),
        parameters_schema: json!({}),
        effects: effects.clone(),
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    };
    let policy = local_dev_capability_policy().expect("capability policy");
    let grants = policy.builtin_grants(
        &provider_id,
        &MountView::default(),
        &MountView::default(),
        &MountView::default(),
        &MountView::default(),
    );
    let context = ironclaw_host_api::ExecutionContext::local_default(
        scope_user.clone(),
        provider_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .expect("execution context");
    let trust_decision = TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: chrono::Utc::now(),
    };
    (descriptor, context, trust_decision)
}

/// Run the local-dev authorizer for a Trace Commons capability with the
/// given descriptor `effects` and return its decision. Asserts up front that
/// the effects WOULD require an approval gate without an exemption, so a
/// "skips gate" assertion can't pass via a non-gating default policy.
async fn trace_commons_authorize_decision(
    capability_id: &str,
    effects: Vec<EffectKind>,
) -> ironclaw_host_api::Decision {
    let capability_id = CapabilityId::new(capability_id).expect("capability id");
    let descriptor = CapabilityDescriptor {
        id: capability_id,
        provider: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id"),
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::UserTrusted,
        description: "test".to_string(),
        parameters_schema: json!({}),
        effects: effects.clone(),
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    };
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let provider_id = ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id");
    let grants = policy.builtin_grants(
        &provider_id,
        &MountView::default(),
        &MountView::default(),
        &MountView::default(),
        &MountView::default(),
    );
    let context = ironclaw_host_api::ExecutionContext::local_default(
        ironclaw_host_api::UserId::new("test-user").expect("user id"),
        provider_id,
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .expect("execution context");
    // These effects must be gate-worthy without an exemption, so the
    // skips-gate vs requires-gate distinction is driven by the exemption
    // list, not by a non-gating default policy.
    assert!(
        local_dev_effects_require_approval(None, policy.as_ref(), &effects),
        "test must use effects that require approval without the capability exemption"
    );
    let trust_decision = TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: chrono::Utc::now(),
    };
    let authorizer = local_dev_authorizer(
        None,
        policy,
        Arc::new(crate::profile_approval_authorization::EmptyApprovalSettingsProvider),
    );
    authorizer
        .authorize_dispatch_with_trust(
            &context,
            &descriptor,
            &ResourceEstimate::default(),
            &trust_decision,
        )
        .await
}

#[tokio::test]
async fn local_dev_trace_commons_profile_set_requires_approval_gate() {
    // profile_set publishes a PUBLIC community profile and is deliberately
    // NOT on the approval-gate exemption list: a model-controlled
    // `confirmed=true` is not sufficient consent for a public external
    // write, so it must hit the runtime approval gate.
    let decision = trace_commons_authorize_decision(
        TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
    )
    .await;
    assert!(
        matches!(
            decision,
            ironclaw_host_api::Decision::RequireApproval { .. }
        ),
        "profile_set (public external write, not exempt) must require an approval gate, got {decision:?}"
    );
}

/// Surface-visibility regression test: `builtin.profile_set` must be
/// Available (Allow) in the local-dev authorizer, not RequireApproval.
///
/// This exercises the FULL authorizer path (grant lookup + effect-set
/// check + exemption list) to guard against the MissingGrant regression
/// that caused the capability to vanish from the model-visible surface.
/// The effects used (ReadFilesystem + WriteFilesystem) are gate-worthy
/// without an exemption (write_filesystem is in ask_writes), so the Allow
/// decision can only come from the exemption list, not from a non-gating
/// default policy.
#[tokio::test]
async fn local_dev_builtin_profile_set_skips_approval_gate() {
    let decision = trace_commons_authorize_decision(
        PROFILE_SET_CAPABILITY_ID,
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
    )
    .await;
    assert!(
        matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
        "builtin.profile_set is a private local write (no network/external_write) and is \
             exempt from the approval gate; got {decision:?}"
    );
}

#[tokio::test]
async fn local_dev_trace_commons_onboard_skips_approval_gate() {
    // onboard IS exempt (it runs its own in-turn confirmed=true consent
    // before the network POST). Cover it with its real
    // network + external_write + filesystem-write effects so dropping the
    // TOML exemption fails here.
    let decision = trace_commons_authorize_decision(
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID,
        vec![
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::ExternalWrite,
        ],
    )
    .await;
    assert!(
        matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
        "onboard is consented in-turn and exempt, so it should not require a REPL approval gate, got {decision:?}"
    );
}

#[tokio::test]
async fn local_dev_authorizer_refreshes_approval_settings_on_next_invocation() {
    let user_id = UserId::new("test-user").expect("user id");
    let overrides = Arc::new(InMemoryToolPermissionOverrideStore::new());
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        overrides,
        auto_approve.clone(),
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    // Global auto-approve now defaults ON, so explicitly disable it first to
    // establish the gating baseline this test reads back across dispatches.
    {
        let scope = ironclaw_host_api::ResourceScope::local_default(
            user_id.clone(),
            ironclaw_host_api::InvocationId::new(),
        )
        .expect("local resource scope");
        auto_approve
            .set(AutoApproveSettingInput {
                scope,
                enabled: false,
                updated_by: Principal::User(user_id.clone()),
            })
            .await
            .expect("auto-approve setting update");
    }

    let before = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(before, ironclaw_host_api::Decision::RequireApproval { .. }),
        "local-dev shell dispatch should gate when global auto-approve is off, got {before:?}"
    );

    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id.clone(),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    auto_approve
        .set(AutoApproveSettingInput {
            scope,
            enabled: true,
            updated_by: Principal::User(user_id.clone()),
        })
        .await
        .expect("auto-approve setting update");

    let after = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(after, ironclaw_host_api::Decision::Allow { .. }),
        "same authorizer should observe the store update on the next invocation, got {after:?}"
    );
}

#[tokio::test]
async fn local_dev_authorizer_observes_global_auto_approve_revocation_on_next_invocation() {
    let user_id = UserId::new("test-user").expect("user id");
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    enable_global_auto_approve(&auto_approve, &user_id).await;
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(InMemoryToolPermissionOverrideStore::new()),
        auto_approve.clone(),
        Arc::new(InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    let before = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(before, ironclaw_host_api::Decision::Allow { .. }),
        "global auto-approve should initially allow the gated shell capability, got {before:?}"
    );

    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id.clone(),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    auto_approve
        .set(AutoApproveSettingInput {
            scope,
            enabled: false,
            updated_by: Principal::User(user_id.clone()),
        })
        .await
        .expect("auto-approve setting update");

    let after = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(after, ironclaw_host_api::Decision::RequireApproval { .. }),
        "revoked global auto-approve must gate the next invocation, got {after:?}"
    );
}

#[tokio::test]
async fn local_dev_authorizer_caches_global_auto_approve_within_one_invocation() {
    let user_id = UserId::new("test-user").expect("user id");
    let auto_approve = Arc::new(CountingAutoApproveSettingStore::enabled());
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(InMemoryToolPermissionOverrideStore::new()),
        auto_approve.clone(),
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);
    let (descriptor, context, trust_decision) = local_dev_shell_authorization_inputs(&user_id);

    for _ in 0..2 {
        let decision = authorizer
            .authorize_dispatch_with_trust(
                &context,
                &descriptor,
                &ResourceEstimate::default(),
                &trust_decision,
            )
            .await;
        assert!(
            matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
            "global auto-approve should allow the gated shell capability, got {decision:?}"
        );
    }

    assert_eq!(
        auto_approve.get_count(),
        1,
        "same invocation should reuse the global auto-approve lookup"
    );

    let (next_descriptor, next_context, next_trust_decision) =
        local_dev_shell_authorization_inputs(&user_id);
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &next_context,
            &next_descriptor,
            &ResourceEstimate::default(),
            &next_trust_decision,
        )
        .await;
    assert!(
        matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
        "later invocation inside the short cache ttl should still allow from cached auto-approve, got {decision:?}"
    );
    assert_eq!(
        auto_approve.get_count(),
        2,
        "a later invocation must reread so settings changes apply immediately"
    );

    let (expired_descriptor, expired_context, expired_trust_decision) =
        local_dev_shell_authorization_inputs(&user_id);
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &expired_context,
            &expired_descriptor,
            &ResourceEstimate::default(),
            &expired_trust_decision,
        )
        .await;
    assert!(
        matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
        "later invocation after the short cache ttl should still allow from store, got {decision:?}"
    );
    assert_eq!(
        auto_approve.get_count(),
        3,
        "each new invocation should get a fresh store read"
    );
}

#[tokio::test]
async fn local_dev_authorizer_coalesces_concurrent_global_auto_approve_misses() {
    let user_id = UserId::new("test-user").expect("user id");
    let auto_approve = Arc::new(CountingAutoApproveSettingStore::enabled_with_delay(
        Duration::from_millis(25),
    ));
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(InMemoryToolPermissionOverrideStore::new()),
        auto_approve.clone(),
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);
    let (descriptor, context, trust_decision) = local_dev_shell_authorization_inputs(&user_id);

    let mut handles = Vec::new();
    for _ in 0..32 {
        let authorizer = authorizer.clone();
        let descriptor = descriptor.clone();
        let context = context.clone();
        let trust_decision = trust_decision.clone();
        handles.push(tokio::spawn(async move {
            authorizer
                .authorize_dispatch_with_trust(
                    &context,
                    &descriptor,
                    &ResourceEstimate::default(),
                    &trust_decision,
                )
                .await
        }));
    }

    for handle in handles {
        let decision = handle.await.expect("authorization task should finish");
        assert!(
            matches!(decision, ironclaw_host_api::Decision::Allow { .. }),
            "global auto-approve should allow the gated shell capability, got {decision:?}"
        );
    }

    assert_eq!(
        auto_approve.get_count(),
        1,
        "concurrent cold misses in one invocation should share one durable settings read"
    );
}

#[tokio::test]
async fn local_dev_authorizer_coalesces_concurrent_scope_listing_settings_misses() {
    let user_id = UserId::new("test-user").expect("user id");
    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id,
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    let capability_id = CapabilityId::new("builtin.shell").expect("capability id");
    let grantee =
        Principal::Extension(ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).expect("provider id"));
    let overrides = Arc::new(CountingToolPermissionOverrideStore::with_delay(
        Duration::from_millis(25),
    ));
    let persistent_policies = Arc::new(CountingPersistentApprovalPolicyStore::with_delay(
        Duration::from_millis(25),
    ));
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        overrides.clone(),
        Arc::new(InMemoryAutoApproveSettingStore::new()),
        persistent_policies.clone(),
    ));

    let mut override_handles = Vec::new();
    for _ in 0..32 {
        let settings = settings.clone();
        let scope = scope.clone();
        let capability_id = capability_id.clone();
        override_handles.push(tokio::spawn(async move {
            settings.tool_override(&scope, &capability_id).await
        }));
    }
    for handle in override_handles {
        let state = handle.await.expect("override lookup should finish");
        assert_eq!(state, None);
    }
    assert_eq!(
        overrides.list_count(),
        1,
        "concurrent tool_override cold misses should share one scope-list read"
    );
    assert_eq!(
        overrides.get_count(),
        0,
        "an authoritative scope-list miss must not fall through to an exact read"
    );
    assert_eq!(settings.tool_override(&scope, &capability_id).await, None);
    assert_eq!(
        overrides.get_count(),
        0,
        "a warm cached miss must not fall through to an exact read"
    );

    let mut always_allow_handles = Vec::new();
    for _ in 0..32 {
        let settings = settings.clone();
        let scope = scope.clone();
        let capability_id = capability_id.clone();
        let grantee = grantee.clone();
        always_allow_handles.push(tokio::spawn(async move {
            settings
                .tool_always_allow(&scope, &capability_id, &grantee)
                .await
        }));
    }
    for handle in always_allow_handles {
        let allowed = handle.await.expect("always-allow lookup should finish");
        assert!(!allowed);
    }
    assert_eq!(
        persistent_policies.list_count(),
        1,
        "concurrent tool_always_allow cold misses should share one scope-list read"
    );
    assert_eq!(
        persistent_policies.lookup_count(),
        0,
        "an authoritative scope-list miss must not fall through to an exact lookup"
    );
    assert!(
        !settings
            .tool_always_allow(&scope, &capability_id, &grantee)
            .await
    );
    assert_eq!(
        persistent_policies.lookup_count(),
        0,
        "a warm cached miss must not fall through to an exact lookup"
    );
}

#[tokio::test]
async fn local_dev_authorizer_releases_global_auto_approve_inflight_when_leader_is_cancelled() {
    let user_id = UserId::new("test-user").expect("user id");
    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id,
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    let auto_approve = Arc::new(CountingAutoApproveSettingStore::enabled_with_delay(
        Duration::from_millis(100),
    ));
    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(InMemoryToolPermissionOverrideStore::new()),
        auto_approve.clone(),
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));

    let leader_settings = Arc::clone(&settings);
    let leader_scope = scope.clone();
    let leader =
        tokio::spawn(async move { leader_settings.global_auto_approve(&leader_scope).await });
    let deadline = std::time::Instant::now() + Duration::from_millis(250);
    while auto_approve.get_count() == 0 {
        if std::time::Instant::now() > deadline {
            panic!("leader settings lookup never started");
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    leader.abort();
    let _ = leader.await;

    let enabled = tokio::time::timeout(
        Duration::from_millis(500),
        settings.global_auto_approve(&scope),
    )
    .await
    .expect("aborted leader must release inflight waiters");

    assert!(enabled);
    assert_eq!(
        auto_approve.get_count(),
        2,
        "a cancelled leader should not poison the inflight map; the retry should reread"
    );
}

#[tokio::test]
async fn local_dev_authorizer_fails_closed_when_override_lookup_errors() {
    let user_id = UserId::new("test-user").expect("user id");
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    let scope = ironclaw_host_api::ResourceScope::local_default(
        user_id.clone(),
        ironclaw_host_api::InvocationId::new(),
    )
    .expect("local resource scope");
    auto_approve
        .set(AutoApproveSettingInput {
            scope,
            enabled: true,
            updated_by: Principal::User(user_id.clone()),
        })
        .await
        .expect("auto-approve setting update");

    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(ErroringToolPermissionOverrideStore),
        auto_approve,
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    let decision = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(
            decision,
            ironclaw_host_api::Decision::RequireApproval { .. }
        ),
        "override-store read errors must fail closed even when global auto-approve is enabled, got {decision:?}"
    );
}

#[tokio::test]
async fn per_tool_disabled_overrides_global_auto_approve_through_store() {
    let user_id = UserId::new("test-user").expect("user id");
    let overrides = Arc::new(InMemoryToolPermissionOverrideStore::new());
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    enable_global_auto_approve(&auto_approve, &user_id).await;
    seed_shell_tool_override(&overrides, &user_id, ToolPermissionOverride::Disabled).await;

    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        overrides,
        auto_approve,
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    let decision = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(decision, ironclaw_host_api::Decision::Deny { .. }),
        "a per-tool Disabled override must deny even with global auto-approve on, got {decision:?}"
    );
}

#[tokio::test]
async fn per_tool_ask_each_time_overrides_global_auto_approve_through_store() {
    let user_id = UserId::new("test-user").expect("user id");
    let overrides = Arc::new(InMemoryToolPermissionOverrideStore::new());
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    enable_global_auto_approve(&auto_approve, &user_id).await;
    seed_shell_tool_override(&overrides, &user_id, ToolPermissionOverride::AskEachTime).await;

    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        overrides,
        auto_approve,
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    let decision = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
    assert!(
        matches!(
            decision,
            ironclaw_host_api::Decision::RequireApproval { .. }
        ),
        "a per-tool AskEachTime override must gate even with global auto-approve on, got {decision:?}"
    );
}

#[tokio::test]
async fn global_auto_approve_does_not_bypass_manifest_ineligible_tool_through_store() {
    let user_id = UserId::new("test-user").expect("user id");
    let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
    enable_global_auto_approve(&auto_approve, &user_id).await;

    let settings = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::new(InMemoryToolPermissionOverrideStore::new()),
        auto_approve,
        Arc::new(ironclaw_approvals::InMemoryPersistentApprovalPolicyStore::new()),
    ));
    let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
    let authorizer = local_dev_authorizer(None, policy, settings);

    // `Deny` manifest permission is not durable-approval eligible, so the
    // global switch must not bypass the gate (the #f14b04d34 manifest-gate
    // fix, exercised here through the real store-backed provider rather than
    // the unit-level stub).
    let (descriptor, context, trust_decision) =
        local_dev_shell_authorization_inputs_with_permission(&user_id, PermissionMode::Deny);
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &context,
            &descriptor,
            &ResourceEstimate::default(),
            &trust_decision,
        )
        .await;
    assert!(
        matches!(
            decision,
            ironclaw_host_api::Decision::RequireApproval { .. }
        ),
        "global auto-approve must not bypass a manifest-ineligible tool, got {decision:?}"
    );
}
