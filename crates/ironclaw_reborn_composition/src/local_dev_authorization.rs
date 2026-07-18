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
    CapabilityId, EffectKind, InvocationId, Principal, ResourceScope,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy, RuntimeProfile},
};
use tokio::sync::Notify;

use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::{
    profile_approval_authorization::{
        ApprovalSettingsProvider, ProfileApprovalGatePolicy, profile_approval_authorizer,
    },
    runtime_profile_approval_policy::RuntimeProfileApprovalGatePolicy,
};

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<BuiltinCapabilityPolicy>,
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
/// stores. Durable settings are operator-scoped, but the short-lived in-process
/// cache is also invocation-scoped so settings edits take effect on the next
/// dispatch while prompt construction does not reread the same settings once
/// per visible capability.
pub(crate) struct StoreApprovalSettingsProvider {
    overrides: Arc<dyn ToolPermissionOverrideStore>,
    auto_approve: Arc<dyn AutoApproveSettingStore>,
    persistent_policies: Arc<dyn PersistentApprovalPolicyStore>,
    auto_approve_cache: Mutex<AutoApproveSettingsCache>,
    override_cache:
        Mutex<ApprovalSettingsScopeCache<HashMap<CapabilityId, ToolPermissionOverride>>>,
    always_allow_cache: Mutex<ApprovalSettingsScopeCache<HashSet<AlwaysAllowPolicyCacheKey>>>,
    auto_approve_inflight: Arc<Mutex<HashMap<AutoApproveSettingsCacheKey, Arc<Notify>>>>,
    override_inflight: Arc<Mutex<HashMap<ApprovalSettingsScopeCacheKey, Arc<Notify>>>>,
    always_allow_inflight: Arc<Mutex<HashMap<ApprovalSettingsScopeCacheKey, Arc<Notify>>>>,
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
            auto_approve_inflight: Arc::new(Mutex::new(HashMap::new())),
            override_inflight: Arc::new(Mutex::new(HashMap::new())),
            always_allow_inflight: Arc::new(Mutex::new(HashMap::new())),
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
    invocation_id: InvocationId,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ApprovalSettingsScopeCacheKey {
    scope: PersistentApprovalScope,
    invocation_id: InvocationId,
}

impl ApprovalSettingsScopeCacheKey {
    fn from_scope(scope: &ResourceScope) -> Self {
        Self {
            scope: PersistentApprovalScope::from_resource_scope(scope),
            invocation_id: scope.invocation_id,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AlwaysAllowPolicyCacheKey {
    capability_id: CapabilityId,
    grantee: Principal,
}

enum InflightSettingsLoad<K>
where
    K: Eq + Hash,
{
    Leader(InflightSettingsLeader<K>),
    Follower(Pin<Box<dyn Future<Output = ()> + Send + 'static>>),
}

struct InflightSettingsLeader<K>
where
    K: Eq + Hash,
{
    inflight: Arc<Mutex<HashMap<K, Arc<Notify>>>>,
    key: K,
    notify: Arc<Notify>,
    finished: bool,
}

impl<K> InflightSettingsLeader<K>
where
    K: Eq + Hash,
{
    fn key(&self) -> &K {
        &self.key
    }

    fn finish(mut self) {
        self.cleanup();
        self.finished = true;
    }

    fn cleanup(&self) {
        self.inflight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.key);
        self.notify.notify_waiters();
    }
}

impl<K> Drop for InflightSettingsLeader<K>
where
    K: Eq + Hash,
{
    fn drop(&mut self) {
        if !self.finished {
            self.cleanup();
        }
    }
}

fn begin_inflight_settings_load<K>(
    inflight: &Arc<Mutex<HashMap<K, Arc<Notify>>>>,
    key: &K,
) -> InflightSettingsLoad<K>
where
    K: Clone + Eq + Hash,
{
    let mut inflight_guard = inflight
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(notify) = inflight_guard.get(key) {
        return InflightSettingsLoad::Follower(Box::pin(notify.clone().notified_owned()));
    }
    let notify = Arc::new(Notify::new());
    inflight_guard.insert(key.clone(), notify.clone());
    InflightSettingsLoad::Leader(InflightSettingsLeader {
        inflight: Arc::clone(inflight),
        key: key.clone(),
        notify,
        finished: false,
    })
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
                    if let Some(state) = overrides.get(capability_id).copied() {
                        return Some(state);
                    }
                    return None;
                }
                match begin_inflight_settings_load(&self.override_inflight, &cache_key) {
                    InflightSettingsLoad::Follower(wait_for_leader) => {
                        wait_for_leader.await;
                    }
                    InflightSettingsLoad::Leader(leader) => {
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
                                let key = leader.key().clone();
                                self.override_cache
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .insert(key.clone(), overrides);
                                leader.finish();
                                return result;
                            }
                            Err(error) => {
                                leader.finish();
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
            invocation_id: scope.invocation_id,
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
                InflightSettingsLoad::Leader(leader) => {
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
                        .insert(leader.key().clone(), enabled);
                    leader.finish();
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
                    InflightSettingsLoad::Leader(leader) => {
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
                                let key = leader.key().clone();
                                self.always_allow_cache
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                                    .insert(key.clone(), policies);
                                leader.finish();
                                return result;
                            }
                            Err(error) => {
                                leader.finish();
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
    capability_policy: &BuiltinCapabilityPolicy,
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
mod tests;
