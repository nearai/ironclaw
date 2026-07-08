use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    hash::Hash,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use ironclaw_approvals::{
    AutoApproveSettingKey, AutoApproveSettingStore, PersistentApprovalAction,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore, PersistentApprovalScope,
    ToolPermissionOverride, ToolPermissionOverrideKey, ToolPermissionOverrideStore,
};
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    CapabilityId, EffectKind, Principal, ResourceScope,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile},
};
use tokio::sync::Notify;

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;
use crate::{
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
const APPROVAL_SETTINGS_CACHE_TTL: Duration = Duration::from_millis(500);

/// Live [`ApprovalSettingsProvider`] backed by the durable per-user approval
/// stores. Settings are operator-scoped, so repeated checks for the same
/// `(tenant, user)` share short-lived bounded cache entries. The cache is
/// deliberately small-TTL rather than process-permanent so WebUI changes take
/// effect without a restart while prompt construction does not reread the same
/// settings once per visible capability.
pub(crate) struct StoreApprovalSettingsProvider {
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    auto_approve_cache: Mutex<AutoApproveSettingsCache>,
    override_cache:
        Mutex<ApprovalSettingsScopeCache<HashMap<CapabilityId, ToolPermissionOverride>>>,
    always_allow_cache: Mutex<ApprovalSettingsScopeCache<HashSet<AlwaysAllowPolicyCacheKey>>>,
    auto_approve_inflight: Mutex<HashMap<AutoApproveSettingsCacheKey, Arc<Notify>>>,
    override_inflight: Mutex<HashMap<ApprovalSettingsScopeCacheKey, Arc<Notify>>>,
    always_allow_inflight: Mutex<HashMap<ApprovalSettingsScopeCacheKey, Arc<Notify>>>,
}

impl StoreApprovalSettingsProvider {
    pub(crate) fn new(
        overrides: Arc<dyn ToolPermissionOverrideStore>,
        auto_approve: Arc<dyn AutoApproveSettingStore>,
        persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    ) -> Self {
        Self {
            overrides,
            auto_approve,
            persistent_policies,
            auto_approve_cache: Mutex::new(AutoApproveSettingsCache::default()),
            override_cache: Mutex::new(ApprovalSettingsScopeCache::default()),
            always_allow_cache: Mutex::new(ApprovalSettingsScopeCache::default()),
            auto_approve_inflight: Mutex::new(HashMap::new()),
            override_inflight: Mutex::new(HashMap::new()),
            always_allow_inflight: Mutex::new(HashMap::new()),
        }
    }
}

fn trace_approval_settings_latency_ok(
    operation: &'static str,
    scope: &ResourceScope,
    capability_id: Option<&CapabilityId>,
    started_at: Option<std::time::Instant>,
) {
    ironclaw_observability::live_latency_trace_ok!(
        "approval_settings_provider",
        operation,
        started_at,
        tenant_id = %scope.tenant_id,
        user_id = %scope.user_id,
        invocation_id = %scope.invocation_id,
        capability_id = capability_id.map(|id| id.as_str()).unwrap_or(""),
        "approval settings lookup completed",
    );
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AutoApproveSettingsCacheKey {
    setting: AutoApproveSettingKey,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ApprovalSettingsScopeCacheKey {
    scope: PersistentApprovalScope,
}

impl ApprovalSettingsScopeCacheKey {
    fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            scope: PersistentApprovalScope::from_resource_scope(scope),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AlwaysAllowPolicyCacheKey {
    capability_id: CapabilityId,
    grantee: Principal,
}

enum InflightSettingsLoad<K> {
    Leader { key: K, notify: Arc<Notify> },
    Follower(Pin<Box<dyn Future<Output = ()> + Send + 'static>>),
}

fn begin_inflight_settings_load<K>(
    inflight: &Mutex<HashMap<K, Arc<Notify>>>,
    key: &K,
) -> InflightSettingsLoad<K>
where
    K: Clone + Eq + Hash,
{
    let mut inflight = inflight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(notify) = inflight.get(key) {
        return InflightSettingsLoad::Follower(Box::pin(notify.clone().notified_owned()));
    }
    let notify = Arc::new(Notify::new());
    inflight.insert(key.clone(), notify.clone());
    InflightSettingsLoad::Leader {
        key: key.clone(),
        notify,
    }
}

fn finish_inflight_settings_load<K>(
    inflight: &Mutex<HashMap<K, Arc<Notify>>>,
    key: &K,
    notify: Arc<Notify>,
) where
    K: Eq + Hash,
{
    inflight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(key);
    notify.notify_waiters();
}

#[derive(Default)]
struct AutoApproveSettingsCache {
    entries: HashMap<AutoApproveSettingsCacheKey, TimedCacheEntry<bool>>,
    recency: VecDeque<AutoApproveSettingsCacheKey>,
}

impl AutoApproveSettingsCache {
    fn get(&mut self, key: &AutoApproveSettingsCacheKey) -> Option<bool> {
        self.get_fresh(key).copied()
    }

    fn insert(&mut self, key: AutoApproveSettingsCacheKey, value: bool) {
        if self
            .entries
            .insert(key.clone(), TimedCacheEntry::new(value))
            .is_some()
        {
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

    fn get_fresh(&mut self, key: &AutoApproveSettingsCacheKey) -> Option<&bool> {
        let fresh = self
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_expired());
        if !fresh {
            self.entries.remove(key);
            self.recency.retain(|candidate| candidate != key);
            return None;
        }
        self.touch(key);
        self.entries.get(key).map(|entry| &entry.value)
    }

    fn touch(&mut self, key: &AutoApproveSettingsCacheKey) {
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.clone());
    }
}

struct TimedCacheEntry<T> {
    value: T,
    cached_at: Instant,
}

impl<T> TimedCacheEntry<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            cached_at: Instant::now(),
        }
    }

    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() >= APPROVAL_SETTINGS_CACHE_TTL
    }
}

struct ApprovalSettingsScopeCache<T> {
    entries: HashMap<ApprovalSettingsScopeCacheKey, TimedCacheEntry<T>>,
    recency: VecDeque<ApprovalSettingsScopeCacheKey>,
}

impl<T> Default for ApprovalSettingsScopeCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            recency: VecDeque::new(),
        }
    }
}

impl<T> ApprovalSettingsScopeCache<T>
where
    T: Clone,
{
    fn get(&mut self, key: &ApprovalSettingsScopeCacheKey) -> Option<T> {
        let fresh = self
            .entries
            .get(key)
            .is_some_and(|entry| !entry.is_expired());
        if !fresh {
            self.entries.remove(key);
            self.recency.retain(|candidate| candidate != key);
            return None;
        }
        self.touch(key);
        self.entries.get(key).map(|entry| entry.value.clone())
    }

    fn insert(&mut self, key: ApprovalSettingsScopeCacheKey, value: T) {
        if self
            .entries
            .insert(key.clone(), TimedCacheEntry::new(value))
            .is_some()
        {
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

    fn touch(&mut self, key: &ApprovalSettingsScopeCacheKey) {
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
        let operator_scope = operator_tool_permission_scope(scope);
        if self.overrides.supports_scope_listing() {
            let cache_key = ApprovalSettingsScopeCacheKey::from_scope(&operator_scope);
            loop {
                if let Some(overrides) = self
                    .override_cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&cache_key)
                {
                    return overrides.get(capability_id).copied();
                }
                match begin_inflight_settings_load(&self.override_inflight, &cache_key) {
                    InflightSettingsLoad::Follower(wait_for_leader) => {
                        wait_for_leader.await;
                    }
                    InflightSettingsLoad::Leader { key, notify } => {
                        let started_at = ironclaw_observability::live_latency_started_at();
                        match self.overrides.list_for_scope(&operator_scope).await {
                            Ok(records) => {
                                trace_approval_settings_latency_ok(
                                    "tool_override_scope",
                                    scope,
                                    None,
                                    started_at,
                                );
                                let overrides = records
                                    .into_iter()
                                    .map(|record| (record.key.capability_id, record.state))
                                    .collect::<HashMap<_, _>>();
                                let result = overrides.get(capability_id).copied();
                                self.override_cache
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .insert(key.clone(), overrides);
                                finish_inflight_settings_load(
                                    &self.override_inflight,
                                    &key,
                                    notify,
                                );
                                return result;
                            }
                            Err(error) => {
                                finish_inflight_settings_load(
                                    &self.override_inflight,
                                    &key,
                                    notify,
                                );
                                tracing::warn!(
                                    %error,
                                    "tool permission override scope lookup failed; falling back to exact lookup"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        let key = ToolPermissionOverrideKey::new(&operator_scope, capability_id.clone());
        let started_at = ironclaw_observability::live_latency_started_at();
        let result = match self.overrides.get(&key).await {
            Ok(record) => record.map(|record| record.state),
            Err(error) => {
                // silent-ok: fail-safe to "ask" on store read error; logged for observability.
                tracing::warn!(%error, "tool permission override lookup failed; defaulting to ask");
                Some(ToolPermissionOverride::AskEachTime)
            }
        };
        trace_approval_settings_latency_ok("tool_override", scope, Some(capability_id), started_at);
        result
    }

    async fn global_auto_approve(&self, scope: &ResourceScope) -> bool {
        let key = AutoApproveSettingsCacheKey {
            setting: AutoApproveSettingKey::from_resource_scope(scope),
        };
        loop {
            if let Some(enabled) = self
                .auto_approve_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&key)
            {
                return enabled;
            }
            match begin_inflight_settings_load(&self.auto_approve_inflight, &key) {
                InflightSettingsLoad::Follower(wait_for_leader) => {
                    wait_for_leader.await;
                }
                InflightSettingsLoad::Leader { key, notify } => {
                    let started_at = ironclaw_observability::live_latency_started_at();
                    let enabled = match self.auto_approve.is_enabled(scope).await {
                        Ok(enabled) => enabled,
                        Err(error) => {
                            // silent-ok: fail-safe to "ask" by disabling global auto-approve; logged for observability.
                            tracing::warn!(%error, "auto-approve setting lookup failed; defaulting to off");
                            false
                        }
                    };
                    trace_approval_settings_latency_ok(
                        "global_auto_approve",
                        scope,
                        None,
                        started_at,
                    );
                    self.auto_approve_cache
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(key.clone(), enabled);
                    finish_inflight_settings_load(&self.auto_approve_inflight, &key, notify);
                    return enabled;
                }
            }
        }
    }

    async fn tool_always_allow(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        grantee: &Principal,
    ) -> bool {
        let operator_scope = operator_tool_permission_scope(scope);
        if self.persistent_policies.supports_scope_listing() {
            let cache_key = ApprovalSettingsScopeCacheKey::from_scope(&operator_scope);
            let policy_key = AlwaysAllowPolicyCacheKey {
                capability_id: capability_id.clone(),
                grantee: grantee.clone(),
            };
            loop {
                if let Some(policies) = self
                    .always_allow_cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .get(&cache_key)
                {
                    return policies.contains(&policy_key);
                }
                match begin_inflight_settings_load(&self.always_allow_inflight, &cache_key) {
                    InflightSettingsLoad::Follower(wait_for_leader) => {
                        wait_for_leader.await;
                    }
                    InflightSettingsLoad::Leader { key, notify } => {
                        let started_at = ironclaw_observability::live_latency_started_at();
                        match self
                            .persistent_policies
                            .list_for_scope_action(
                                &operator_scope,
                                PersistentApprovalAction::Dispatch,
                            )
                            .await
                        {
                            Ok(records) => {
                                trace_approval_settings_latency_ok(
                                    "tool_always_allow_scope",
                                    scope,
                                    None,
                                    started_at,
                                );
                                let policies = records
                                    .into_iter()
                                    .filter(|policy| policy.active_grant().is_some())
                                    .map(|policy| AlwaysAllowPolicyCacheKey {
                                        capability_id: policy.key.capability_id,
                                        grantee: policy.key.grantee,
                                    })
                                    .collect::<HashSet<_>>();
                                let result = policies.contains(&policy_key);
                                self.always_allow_cache
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .insert(key.clone(), policies);
                                finish_inflight_settings_load(
                                    &self.always_allow_inflight,
                                    &key,
                                    notify,
                                );
                                return result;
                            }
                            Err(error) => {
                                finish_inflight_settings_load(
                                    &self.always_allow_inflight,
                                    &key,
                                    notify,
                                );
                                tracing::warn!(
                                    %error,
                                    "persistent approval policy scope lookup failed; falling back to exact lookup"
                                );
                                break;
                            }
                        }
                    }
                }
            }
        }

        let key = PersistentApprovalPolicyKey::new(
            &operator_scope,
            PersistentApprovalAction::Dispatch,
            capability_id.clone(),
            grantee.clone(),
        );
        let started_at = ironclaw_observability::live_latency_started_at();
        let result = match self.persistent_policies.lookup(&key).await {
            Ok(policy) => policy.and_then(|policy| policy.active_grant()).is_some(),
            Err(error) => {
                // silent-ok: fail-safe to "ask" on store read error; logged for observability.
                tracing::debug!(
                    %error,
                    capability = %capability_id,
                    "settings always-allow lookup failed; defaulting to ask"
                );
                false
            }
        };
        trace_approval_settings_latency_ok(
            "tool_always_allow",
            scope,
            Some(capability_id),
            started_at,
        );
        result
    }
}

fn operator_tool_permission_scope(scope: &ResourceScope) -> ResourceScope {
    ResourceScope {
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: scope.invocation_id,
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
    async fn local_dev_authorizer_refreshes_approval_settings_after_short_cache_ttl() {
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

        tokio::time::sleep(APPROVAL_SETTINGS_CACHE_TTL + Duration::from_millis(50)).await;
        let after = local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
        assert!(
            matches!(after, ironclaw_host_api::Decision::Allow { .. }),
            "same authorizer should observe the store update after the short settings cache ttl, got {after:?}"
        );
    }

    #[tokio::test]
    async fn local_dev_authorizer_caches_global_auto_approve_within_short_scope_ttl() {
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
            "same scope should reuse the global auto-approve lookup inside the short cache ttl"
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
            1,
            "a later invocation inside the short cache ttl should not reread the store"
        );

        tokio::time::sleep(APPROVAL_SETTINGS_CACHE_TTL + Duration::from_millis(50)).await;
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
            2,
            "after the short cache ttl, the provider should reread so settings changes apply without restart"
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

        let mut handles = Vec::new();
        for _ in 0..32 {
            let authorizer = authorizer.clone();
            let user_id = user_id.clone();
            handles.push(tokio::spawn(async move {
                local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await
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
            "concurrent cold misses for one operator scope should share one durable settings read"
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

        let decision =
            local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
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

        let decision =
            local_dev_shell_decision_with_authorizer(authorizer.as_ref(), &user_id).await;
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
}
