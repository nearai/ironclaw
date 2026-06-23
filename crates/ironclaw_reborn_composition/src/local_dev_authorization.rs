use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingKey, AutoApproveSettingStore, ToolPermissionOverride,
    ToolPermissionOverrideKey, ToolPermissionOverrideStore,
};
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    CapabilityId, EffectKind, InvocationId, ResourceScope,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile},
};

use crate::{
    local_dev_capability_policy::LocalDevCapabilityPolicy,
    profile_approval_authorization::{
        ApprovalSettingsProvider, ProfileApprovalGatePolicy, profile_approval_authorizer,
    },
    runtime_profile_approval_policy::RuntimeProfileApprovalGatePolicy,
};

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    let gate_effects = capability_policy.approval_gate_effects();
    let exempt_capabilities = capability_policy.approval_gate_exempt_capabilities();
    let gate_policy: Arc<dyn ProfileApprovalGatePolicy> = Arc::new(
        RuntimeProfileApprovalGatePolicy::new(resolved_profile, gate_effects)
            .with_exempt_capabilities(exempt_capabilities),
    );
    profile_approval_authorizer(approval_policy, gate_policy, settings)
}

const AUTO_APPROVE_INVOCATION_CACHE_MAX_ENTRIES: usize = 512;

/// Live [`ApprovalSettingsProvider`] backed by the durable per-user approval
/// stores. Per-tool overrides are read at each eligible dispatch decision. The
/// global auto-approve toggle is scoped to `(tenant, user)`, so repeated checks
/// inside one invocation share a bounded cache entry; a later invocation reads
/// the store again so WebUI changes take effect without a process restart
/// (#4959).
pub(crate) struct StoreApprovalSettingsProvider {
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    auto_approve_cache: Mutex<AutoApproveInvocationCache>,
}

impl StoreApprovalSettingsProvider {
    pub(crate) fn new(
        overrides: Arc<dyn ToolPermissionOverrideStore>,
        auto_approve: Arc<dyn AutoApproveSettingStore>,
    ) -> Self {
        Self {
            overrides,
            auto_approve,
            auto_approve_cache: Mutex::new(AutoApproveInvocationCache::default()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AutoApproveInvocationCacheKey {
    setting: AutoApproveSettingKey,
    invocation_id: InvocationId,
}

#[derive(Default)]
struct AutoApproveInvocationCache {
    entries: HashMap<AutoApproveInvocationCacheKey, bool>,
    recency: VecDeque<AutoApproveInvocationCacheKey>,
}

impl AutoApproveInvocationCache {
    fn get(&mut self, key: &AutoApproveInvocationCacheKey) -> Option<bool> {
        let value = self.entries.get(key).copied();
        if value.is_some() {
            self.touch(key);
        }
        value
    }

    fn insert(&mut self, key: AutoApproveInvocationCacheKey, value: bool) {
        if self.entries.insert(key.clone(), value).is_some() {
            self.touch(&key);
            return;
        }
        self.recency.push_back(key);
        while self.entries.len() > AUTO_APPROVE_INVOCATION_CACHE_MAX_ENTRIES {
            let Some(evicted) = self.recency.pop_front() else {
                break;
            };
            self.entries.remove(&evicted);
        }
    }

    fn touch(&mut self, key: &AutoApproveInvocationCacheKey) {
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.clone());
    }
}

#[async_trait]
impl ApprovalSettingsProvider for StoreApprovalSettingsProvider {
    async fn tool_override(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<ToolPermissionOverride> {
        // Fail safe: a store read error resolves to "ask each time" with
        // auto-approve off so the gate falls back to asking rather than
        // silently auto-approving or denying. The error is logged, not swallowed.
        let key = ToolPermissionOverrideKey::new(scope, capability_id.clone());
        match self.overrides.get(&key).await {
            Ok(record) => record.map(|record| record.state),
            Err(error) => {
                // silent-ok: fail-safe to "ask" on store read error; logged for observability.
                tracing::warn!(%error, "tool permission override lookup failed; defaulting to ask");
                Some(ToolPermissionOverride::AskEachTime)
            }
        }
    }

    async fn global_auto_approve(&self, scope: &ResourceScope) -> bool {
        let key = AutoApproveInvocationCacheKey {
            setting: AutoApproveSettingKey::from_resource_scope(scope),
            invocation_id: scope.invocation_id,
        };
        if let Some(enabled) = self
            .auto_approve_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
        {
            return enabled;
        }
        let enabled = match self.auto_approve.is_enabled(scope).await {
            Ok(enabled) => enabled,
            Err(error) => {
                // silent-ok: fail-safe to "ask" by disabling global auto-approve; logged for observability.
                tracing::warn!(%error, "auto-approve setting lookup failed; defaulting to off");
                false
            }
        };
        self.auto_approve_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, enabled);
        enabled
    }
}

pub(crate) fn local_dev_effects_require_approval(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: &LocalDevCapabilityPolicy,
    effects: &[EffectKind],
) -> bool {
    let (approval_policy, resolved_profile) = local_dev_approval_policy(runtime_policy);
    RuntimeProfileApprovalGatePolicy::new(
        resolved_profile,
        capability_policy.approval_gate_effects(),
    )
    .effects_require_approval(approval_policy, effects)
}

fn local_dev_approval_policy(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
) -> (ApprovalPolicy, RuntimeProfile) {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    let resolved_profile = runtime_policy
        .map(|policy| policy.resolved_profile)
        .unwrap_or(RuntimeProfile::LocalDev);
    (approval_policy, resolved_profile)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use ironclaw_approvals::{
        AutoApproveSettingInput, AutoApproveSettingKey, AutoApproveSettingRecord,
        AutoApproveSettingStore, CapabilityPermissionOverrideInput,
        CapabilityPermissionOverrideKey, CapabilityPermissionOverrideRecord,
        CapabilityPermissionOverrideStore, CapabilityPermissionStoreError,
        InMemoryAutoApproveSettingStore, InMemoryToolPermissionOverrideStore,
    };
    use ironclaw_host_api::{
        CapabilityDescriptor, CapabilityId, EffectKind, ExecutionContext, ExtensionId, MountView,
        PermissionMode, Principal, ResourceEstimate, RuntimeKind, Timestamp, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        BUILTIN_FIRST_PARTY_PROVIDER, PROFILE_SET_CAPABILITY_ID,
        TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::json;

    use super::*;
    use crate::local_dev_capability_policy::local_dev_capability_policy;

    struct ErroringToolPermissionOverrideStore;

    struct CountingAutoApproveSettingStore {
        enabled: bool,
        gets: AtomicUsize,
    }

    impl CountingAutoApproveSettingStore {
        fn enabled() -> Self {
            Self {
                enabled: true,
                gets: AtomicUsize::new(0),
            }
        }

        fn get_count(&self) -> usize {
            self.gets.load(Ordering::SeqCst)
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
        ) -> Result<Option<CapabilityPermissionOverrideRecord>, CapabilityPermissionStoreError>
        {
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
        let (descriptor, context, trust_decision) =
            local_dev_shell_authorization_inputs(scope_user);
        authorizer
            .authorize_dispatch_with_trust(
                &context,
                &descriptor,
                &ResourceEstimate::default(),
                &trust_decision,
            )
            .await
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
    async fn local_dev_authorizer_reads_approval_settings_store_on_each_dispatch() {
        let user_id = UserId::new("test-user").expect("user id");
        let overrides = Arc::new(InMemoryToolPermissionOverrideStore::new());
        let auto_approve = Arc::new(InMemoryAutoApproveSettingStore::new());
        let settings = Arc::new(StoreApprovalSettingsProvider::new(
            overrides,
            auto_approve.clone(),
        ));
        let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
        let authorizer = local_dev_authorizer(None, policy, settings);

        let before = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
        assert!(
            matches!(before, ironclaw_host_api::Decision::RequireApproval { .. }),
            "default local-dev shell dispatch should gate before a settings update, got {before:?}"
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
            "same authorizer should observe the store update on the next dispatch, got {after:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_authorizer_caches_global_auto_approve_within_one_invocation() {
        let user_id = UserId::new("test-user").expect("user id");
        let auto_approve = Arc::new(CountingAutoApproveSettingStore::enabled());
        let settings = Arc::new(StoreApprovalSettingsProvider::new(
            Arc::new(InMemoryToolPermissionOverrideStore::new()),
            auto_approve.clone(),
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
            "later invocation should still resolve auto-approve from the store, got {decision:?}"
        );
        assert_eq!(
            auto_approve.get_count(),
            2,
            "a later invocation must read the store again so settings changes apply without restart"
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
        ));
        let policy = Arc::new(local_dev_capability_policy().expect("capability policy"));
        let authorizer = local_dev_authorizer(None, policy, settings);

        let decision =
            local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
        assert!(
            matches!(
                decision,
                ironclaw_host_api::Decision::RequireApproval { .. }
            ),
            "override-store read errors must fail closed even when global auto-approve is enabled, got {decision:?}"
        );
    }
}
