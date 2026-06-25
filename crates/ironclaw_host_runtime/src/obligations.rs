use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_capabilities::{
    CapabilityObligationAbortRequest, CapabilityObligationCompletionRequest,
    CapabilityObligationError, CapabilityObligationFailureKind, CapabilityObligationHandler,
    CapabilityObligationOutcome, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_events::{
    AuditSink, EventSink, RuntimeEvent, SecurityAuditEvent, SecurityAuditSink, SecurityBoundary,
    SecurityDecision,
};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityDispatchResult, CapabilityId, CredentialStageError, DecisionSummary, EffectKind,
    ExtensionId, MountView, NetworkPolicy, Obligation, ProcessId, ResourceCeiling,
    ResourceEstimate, ResourceReservation, ResourceScope, ResourceUsage,
    RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
    RuntimeCredentialAuthRequirement, RuntimeHttpEgress, SandboxQuota, SecretHandle, Timestamp,
};
use ironclaw_network::NetworkHttpEgress;
use ironclaw_processes::{ProcessError, ProcessRecord, ProcessStart, ProcessStore};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::{
    SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore, SecretStoreError,
};
use secrecy::ExposeSecret;

use crate::{
    ToolCallHttpEgress,
    http_body::{RuntimeHttpBodyStore, UnsupportedRuntimeHttpBodyStore},
};

/// Default maximum lifetime for one-shot runtime secret material staged in memory.
pub(crate) const DEFAULT_RUNTIME_SECRET_INJECTION_TTL: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct RuntimeCredentialAccountRequest<'a> {
    pub scope: &'a ResourceScope,
    pub provider: &'a RuntimeCredentialAccountProviderId,
    pub setup: &'a RuntimeCredentialAccountSetup,
    pub provider_scopes: &'a [String],
    pub requester_extension: &'a ExtensionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCredentialAccessSecret {
    pub scope: ResourceScope,
    pub handle: SecretHandle,
}

#[async_trait]
pub trait RuntimeCredentialAccountResolver: Send + Sync + fmt::Debug {
    /// Resolve the access-secret source for the requested product-auth account.
    ///
    /// Returns [`CredentialStageError::AuthRequired`] when the account is
    /// missing/unconfigured/expired/revoked (user must re-authenticate), or
    /// [`CredentialStageError::Backend`] for internal failures not attributable
    /// to user credentials. Shares its error vocabulary with the rest of the
    /// staged-credential surface (`ProductAuthCredentialStageError`,
    /// `GsuiteCredentialStageError`) so no per-layer error mapping is needed.
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError>;
}

/// Runtime secret material staged after `InjectSecretOnce` lease consumption.
///
/// The store is keyed by scoped invocation, capability, and handle. Runtime adapters
/// borrow staged material during dispatch; `complete_dispatch`/`abort` removes it
/// after the scoped capability finishes. Entries also expire after a short TTL so
/// abandoned handoffs from setup failures, cancellation, or adapter bugs cannot
/// remain usable indefinitely.
#[derive(Clone)]
pub(crate) struct RuntimeSecretInjectionStore {
    state: Arc<RuntimeSecretInjectionState>,
}

struct RuntimeSecretInjectionState {
    secrets: Mutex<HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>>,
    ttl: Duration,
}

struct RuntimeSecretInjectionEntry {
    material: SecretMaterial,
    expires_at: Instant,
}

impl RuntimeSecretInjectionStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_ttl(ttl: Duration) -> Self {
        Self {
            state: Arc::new(RuntimeSecretInjectionState {
                secrets: Mutex::new(HashMap::new()),
                ttl,
            }),
        }
    }

    pub(crate) fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let expires_at = now.checked_add(self.state.ttl).unwrap_or(now);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        secrets.insert(
            RuntimeSecretInjectionKey::new(scope, capability_id, handle),
            RuntimeSecretInjectionEntry {
                material,
                expires_at,
            },
        );
        Ok(())
    }

    pub(crate) fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        Ok(secrets
            .remove(&RuntimeSecretInjectionKey::new(
                scope,
                capability_id,
                handle,
            ))
            .map(|entry| entry.material))
    }

    pub(crate) fn clone_material(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        let now = Instant::now();
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, now);
        Ok(secrets
            .get(&RuntimeSecretInjectionKey::new(
                scope,
                capability_id,
                handle,
            ))
            .map(|entry| SecretMaterial::from(entry.material.expose_secret())))
    }

    /// Discard all staged secrets for a scoped capability before process ownership exists.
    ///
    /// Background process lifecycle cleanup is guarded by a single-active-handoff
    /// invariant for the scoped capability; this method remains the abort/inline cleanup seam.
    pub(crate) fn discard_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        let scope_key = RuntimeSecretInjectionScopeKey::new(scope, capability_id);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, Instant::now());
        secrets.retain(|key, _| !key.matches_scope(&scope_key));
        Ok(())
    }

    fn has_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Result<bool, RuntimeSecretInjectionStoreError> {
        let scope_key = RuntimeSecretInjectionScopeKey::new(scope, capability_id);
        let mut secrets = self.lock()?;
        prune_expired_entries(&mut secrets, Instant::now());
        Ok(secrets.keys().any(|key| key.matches_scope(&scope_key)))
    }

    #[cfg(test)]
    fn prune_expired(&self) -> Result<usize, RuntimeSecretInjectionStoreError> {
        let mut secrets = self.lock()?;
        Ok(prune_expired_entries(&mut secrets, Instant::now()))
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>>,
        RuntimeSecretInjectionStoreError,
    > {
        self.state
            .secrets
            .lock()
            .map_err(|_| RuntimeSecretInjectionStoreError::Unavailable)
    }
}

impl Default for RuntimeSecretInjectionStore {
    fn default() -> Self {
        Self::with_ttl(DEFAULT_RUNTIME_SECRET_INJECTION_TTL)
    }
}

impl fmt::Debug for RuntimeSecretInjectionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSecretInjectionStore")
            .field("secrets", &"[REDACTED]")
            .field("ttl", &self.state.ttl)
            .finish()
    }
}

fn prune_expired_entries(
    secrets: &mut HashMap<RuntimeSecretInjectionKey, RuntimeSecretInjectionEntry>,
    now: Instant,
) -> usize {
    let before = secrets.len();
    secrets.retain(|_, entry| entry.expires_at > now);
    before.saturating_sub(secrets.len())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeSecretInjectionStoreError {
    Unavailable,
}

impl fmt::Display for RuntimeSecretInjectionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("runtime secret injection store unavailable"),
        }
    }
}

impl std::error::Error for RuntimeSecretInjectionStoreError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeSecretInjectionKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
    handle: String,
}

impl RuntimeSecretInjectionKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId, handle: &SecretHandle) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
            handle: handle.as_str().to_string(),
        }
    }

    fn matches_scope(&self, scope: &RuntimeSecretInjectionScopeKey) -> bool {
        self.tenant_id == scope.tenant_id
            && self.user_id == scope.user_id
            && self.agent_id == scope.agent_id
            && self.project_id == scope.project_id
            && self.mission_id == scope.mission_id
            && self.thread_id == scope.thread_id
            && self.invocation_id == scope.invocation_id
            && self.capability_id == scope.capability_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeSecretInjectionScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl RuntimeSecretInjectionScopeKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

/// In-memory policy handoff from obligation handling to runtime adapters.
///
/// Policies are keyed by tenant/user/project/mission/thread/invocation scope and
/// capability id. Runtime adapters and host egress borrow the staged policy for
/// every network operation in the invocation; obligation completion/abort or
/// process lifecycle cleanup owns the final discard.
#[derive(Debug, Clone, Default)]
pub(crate) struct NetworkObligationPolicyStore {
    policies: Arc<Mutex<HashMap<NetworkPolicyKey, NetworkPolicy>>>,
}

impl NetworkObligationPolicyStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: NetworkPolicy,
    ) {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(NetworkPolicyKey::new(scope, capability_id), policy);
    }

    pub(crate) fn get(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<NetworkPolicy> {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&NetworkPolicyKey::new(scope, capability_id))
            .cloned()
    }

    pub(crate) fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> Option<NetworkPolicy> {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&NetworkPolicyKey::new(scope, capability_id))
    }

    /// Discard a staged policy for a scoped capability before process ownership exists.
    ///
    /// Background process lifecycle cleanup is guarded by a single-active-handoff
    /// invariant for the scoped capability; this method remains the abort/inline cleanup seam.
    pub(crate) fn discard_for_capability(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) {
        let _ = self.take(scope, capability_id);
    }

    fn contains(&self, scope: &ResourceScope, capability_id: &CapabilityId) -> bool {
        self.policies
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(&NetworkPolicyKey::new(scope, capability_id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NetworkPolicyKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl NetworkPolicyKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

/// Host-runtime-owned backing services for a fully configured built-in obligation handler.
///
/// This value is the production composition seam for obligation handling. It
/// keeps the in-memory network-policy and runtime-secret handoff stores alive
/// outside the handler so runtime adapters can consume the exact staged state
/// that [`BuiltinObligationHandler`] prepares before dispatch.
#[derive(Clone)]
pub struct BuiltinObligationServices {
    audit_sink: Arc<dyn AuditSink>,
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_store: Arc<dyn SecretStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    credential_account_resolver: Option<Arc<dyn RuntimeCredentialAccountResolver>>,
}

impl BuiltinObligationServices {
    pub fn new(
        audit_sink: Arc<dyn AuditSink>,
        secret_store: Arc<dyn SecretStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self::with_handoff_stores(
            audit_sink,
            Arc::new(NetworkObligationPolicyStore::new()),
            secret_store,
            Arc::new(RuntimeSecretInjectionStore::new()),
            resource_governor,
        )
    }

    pub(crate) fn with_handoff_stores(
        audit_sink: Arc<dyn AuditSink>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_store: Arc<dyn SecretStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self {
            audit_sink,
            network_policies,
            secret_store,
            secret_injections,
            resource_governor,
            credential_account_resolver: None,
        }
    }

    pub fn with_credential_account_resolver<T>(mut self, resolver: Arc<T>) -> Self
    where
        T: RuntimeCredentialAccountResolver + 'static,
    {
        self.credential_account_resolver = Some(resolver);
        self
    }

    pub fn with_credential_account_resolver_dyn(
        mut self,
        resolver: Arc<dyn RuntimeCredentialAccountResolver>,
    ) -> Self {
        self.credential_account_resolver = Some(resolver);
        self
    }

    pub fn audit_sink(&self) -> Arc<dyn AuditSink> {
        self.audit_sink.clone()
    }

    pub fn secret_store(&self) -> Arc<dyn SecretStore> {
        self.secret_store.clone()
    }

    pub fn resource_governor(&self) -> Arc<dyn ResourceGovernor> {
        self.resource_governor.clone()
    }

    /// Builds host HTTP egress over this service graph's private handoff stores.
    /// Callers can supply concrete network transport without receiving mutable
    /// access to staged policy or secret material.
    pub fn host_http_egress<N>(
        &self,
        network: N,
    ) -> impl RuntimeHttpEgress + ToolCallHttpEgress + use<N>
    where
        N: NetworkHttpEgress + 'static,
    {
        self.host_http_egress_with_body_store(network, Arc::new(UnsupportedRuntimeHttpBodyStore))
    }

    pub fn host_http_egress_with_body_store<N, T>(
        &self,
        network: N,
        body_store: Arc<T>,
    ) -> impl RuntimeHttpEgress + ToolCallHttpEgress + use<N, T>
    where
        N: NetworkHttpEgress + 'static,
        T: RuntimeHttpBodyStore + 'static,
    {
        let body_store: Arc<dyn RuntimeHttpBodyStore> = body_store;
        crate::HostHttpEgressService::production(
            network,
            SharedSecretStore(self.secret_store.clone()),
            self.network_policies.clone(),
            self.secret_injections.clone(),
            body_store,
        )
    }

    pub fn process_obligation_lifecycle_store<S>(
        &self,
        inner: Arc<S>,
    ) -> ProcessObligationLifecycleStore
    where
        S: ProcessStore + 'static,
    {
        ProcessObligationLifecycleStore::new(
            inner,
            self.network_policies.clone(),
            self.secret_injections.clone(),
            self.resource_governor.clone(),
        )
    }

    pub fn process_obligation_lifecycle_store_dyn(
        &self,
        inner: Arc<dyn ProcessStore>,
    ) -> ProcessObligationLifecycleStore {
        ProcessObligationLifecycleStore::from_dyn(
            inner,
            self.network_policies.clone(),
            self.secret_injections.clone(),
            self.resource_governor.clone(),
        )
    }

    pub fn obligation_handler(&self) -> BuiltinObligationHandler {
        let handler = BuiltinObligationHandler::new()
            .with_audit_sink_dyn(self.audit_sink.clone())
            .with_network_policy_store(self.network_policies.clone())
            .with_secret_store_dyn(self.secret_store.clone())
            .with_secret_injection_store(self.secret_injections.clone())
            .with_resource_governor_dyn(self.resource_governor.clone());
        match &self.credential_account_resolver {
            Some(resolver) => handler.with_credential_account_resolver_dyn(Arc::clone(resolver)),
            None => handler,
        }
    }
}

impl fmt::Debug for BuiltinObligationServices {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuiltinObligationServices")
            .field("audit_sink", &"<audit_sink>")
            .field("network_policies", &self.network_policies)
            .field("secret_store", &"[REDACTED]")
            .field("secret_injections", &self.secret_injections)
            .field("resource_governor", &"<resource_governor>")
            .field(
                "credential_account_resolver",
                &self
                    .credential_account_resolver
                    .as_ref()
                    .map(|_| "<resolver>"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct SharedSecretStore(pub(crate) Arc<dyn SecretStore>);

#[async_trait]
impl SecretStore for SharedSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.0.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        self.0.metadata(scope, handle).await
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        self.0.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.0.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.0.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.0.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.0.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.0.leases_for_scope(scope).await
    }
}

/// Process-store wrapper that owns spawn-phase obligation handoffs after
/// `ProcessStore::start` succeeds.
///
/// `CapabilityHost` aborts prepared effects when process start fails. Once
/// start succeeds, this wrapper becomes responsible for discarding staged
/// network/secret handoffs and reconciling or releasing a prepared resource
/// reservation when the process reaches a terminal state.
pub struct ProcessObligationLifecycleStore {
    inner: Arc<dyn ProcessStore>,
    network_policies: Arc<NetworkObligationPolicyStore>,
    secret_injections: Arc<RuntimeSecretInjectionStore>,
    resource_governor: Arc<dyn ResourceGovernor>,
    event_sink: Mutex<Option<Arc<dyn EventSink>>>,
    active_process_handoffs: Mutex<HashMap<ProcessObligationHandoffKey, ProcessId>>,
    cleaned_process_handoffs: Mutex<HashSet<ProcessObligationProcessKey>>,
}

impl ProcessObligationLifecycleStore {
    pub(crate) fn new<S>(
        inner: Arc<S>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self
    where
        S: ProcessStore + 'static,
    {
        let inner: Arc<dyn ProcessStore> = inner;
        Self::from_dyn(
            inner,
            network_policies,
            secret_injections,
            resource_governor,
        )
    }

    pub(crate) fn from_dyn(
        inner: Arc<dyn ProcessStore>,
        network_policies: Arc<NetworkObligationPolicyStore>,
        secret_injections: Arc<RuntimeSecretInjectionStore>,
        resource_governor: Arc<dyn ResourceGovernor>,
    ) -> Self {
        Self {
            inner,
            network_policies,
            secret_injections,
            resource_governor,
            event_sink: Mutex::new(None),
            active_process_handoffs: Mutex::new(HashMap::new()),
            cleaned_process_handoffs: Mutex::new(HashSet::new()),
        }
    }

    /// Attaches a best-effort event sink for process lifecycle transitions.
    pub fn set_event_sink(&self, event_sink: Arc<dyn EventSink>) {
        match self.event_sink.lock() {
            Ok(mut slot) => {
                *slot = Some(event_sink);
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "process lifecycle event sink registry unavailable"
                );
            }
        }
    }

    async fn emit_process_event(&self, event: RuntimeEvent) {
        let event_sink = match self.event_sink.lock() {
            Ok(slot) => slot.clone(),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    "process lifecycle event sink registry unavailable"
                );
                None
            }
        };
        if let Some(event_sink) = event_sink {
            let _ = event_sink.emit(event).await;
        }
    }

    /// Discards staged obligation handoffs and closes any reservation for an
    /// executor that finished but could not publish its result record.
    pub async fn cleanup_process_obligations(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if let Some(record) = self.inner.get(scope, process_id).await? {
            self.cleanup_record_obligations(&record, reconcile)?;
            self.release_active_process_handoff(&record)?;
            self.mark_process_handoff_cleaned(&record)?;
        }
        Ok(())
    }

    fn has_process_obligations(&self, start: &ProcessStart) -> Result<bool, ProcessError> {
        let has_secret_handoff = self
            .secret_injections
            .has_for_capability(&start.scope, &start.capability_id)
            .map_err(|_| ProcessError::InvalidStoredRecord {
                reason: "process obligation handoff lookup failed".to_string(),
            })?;
        Ok(start.resource_reservation_id.is_some()
            || self
                .network_policies
                .contains(&start.scope, &start.capability_id)
            || has_secret_handoff)
    }

    fn claim_active_process_handoff(&self, start: &ProcessStart) -> Result<bool, ProcessError> {
        if !self.has_process_obligations(start)? {
            return Ok(false);
        }

        let key = ProcessObligationHandoffKey::new(&start.scope, &start.capability_id);
        let mut active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        if let Some(existing_process_id) = active.get(&key) {
            return Err(ProcessError::InvalidStoredRecord {
                reason: format!(
                    "process obligation handoff already active for scoped capability: {existing_process_id}"
                ),
            });
        }
        active.insert(key, start.process_id);
        Ok(true)
    }

    fn release_claimed_process_handoff(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        process_id: ProcessId,
    ) -> Result<(), ProcessError> {
        let key = ProcessObligationHandoffKey::new(scope, capability_id);
        let mut active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        if active.get(&key) == Some(&process_id) {
            active.remove(&key);
        }
        Ok(())
    }

    fn release_active_process_handoff(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        self.release_claimed_process_handoff(
            &record.scope,
            &record.capability_id,
            record.process_id,
        )
    }

    fn has_active_process_handoff(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let key = ProcessObligationHandoffKey::new(&record.scope, &record.capability_id);
        let active =
            self.active_process_handoffs
                .lock()
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff registry unavailable".to_string(),
                })?;
        Ok(active.get(&key) == Some(&record.process_id))
    }

    fn process_handoff_cleaned(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let key = ProcessObligationProcessKey::new(&record.scope, record.process_id);
        let cleaned = self.cleaned_process_handoffs.lock().map_err(|_| {
            ProcessError::InvalidStoredRecord {
                reason: "process obligation cleanup registry unavailable".to_string(),
            }
        })?;
        Ok(cleaned.contains(&key))
    }

    fn mark_process_handoff_cleaned(&self, record: &ProcessRecord) -> Result<(), ProcessError> {
        let key = ProcessObligationProcessKey::new(&record.scope, record.process_id);
        let mut cleaned = self.cleaned_process_handoffs.lock().map_err(|_| {
            ProcessError::InvalidStoredRecord {
                reason: "process obligation cleanup registry unavailable".to_string(),
            }
        })?;
        cleaned.insert(key);
        Ok(())
    }

    fn has_staged_handoffs(&self, record: &ProcessRecord) -> Result<bool, ProcessError> {
        let has_secret_handoff = self
            .secret_injections
            .has_for_capability(&record.scope, &record.capability_id)
            .map_err(|_| ProcessError::InvalidStoredRecord {
                reason: "process obligation handoff lookup failed".to_string(),
            })?;
        Ok(self
            .network_policies
            .contains(&record.scope, &record.capability_id)
            || has_secret_handoff)
    }

    fn cleanup_terminal(
        &self,
        record: &ProcessRecord,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if let Err(error) = self.cleanup_record_obligations(record, reconcile) {
            tracing::warn!(
                process_id = %record.process_id,
                tenant_id = %record.scope.tenant_id,
                user_id = %record.scope.user_id,
                reconcile,
                error = %error,
                "process obligation cleanup failed after terminal transition"
            );
            return Err(error);
        }
        self.release_active_process_handoff(record)?;
        self.mark_process_handoff_cleaned(record)?;
        Ok(())
    }

    fn cleanup_record_obligations(
        &self,
        record: &ProcessRecord,
        reconcile: bool,
    ) -> Result<(), ProcessError> {
        if self.process_handoff_cleaned(record)? {
            return Ok(());
        }
        let should_cleanup_handoffs = self.has_active_process_handoff(record)?
            || record.resource_reservation_id.is_some()
            || self.has_staged_handoffs(record)?;
        if should_cleanup_handoffs {
            self.network_policies
                .discard_for_capability(&record.scope, &record.capability_id);
            self.secret_injections
                .discard_for_capability(&record.scope, &record.capability_id)
                .map_err(|_| ProcessError::InvalidStoredRecord {
                    reason: "process obligation handoff cleanup failed".to_string(),
                })?;
        }
        if let Some(reservation_id) = record.resource_reservation_id {
            if reconcile {
                close_reservation_once(
                    self.resource_governor
                        .reconcile(reservation_id, ResourceUsage::default()),
                )?;
            } else {
                close_reservation_once(self.resource_governor.release(reservation_id))?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessObligationHandoffKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    invocation_id: String,
    capability_id: String,
}

impl ProcessObligationHandoffKey {
    fn new(scope: &ResourceScope, capability_id: &CapabilityId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            invocation_id: scope.invocation_id.to_string(),
            capability_id: capability_id.as_str().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProcessObligationProcessKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    process_id: ProcessId,
}

impl ProcessObligationProcessKey {
    fn new(scope: &ResourceScope, process_id: ProcessId) -> Self {
        Self {
            tenant_id: scope.tenant_id.as_str().to_string(),
            user_id: scope.user_id.as_str().to_string(),
            agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
            project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
            mission_id: scope.mission_id.as_ref().map(|id| id.as_str().to_string()),
            thread_id: scope.thread_id.as_ref().map(|id| id.as_str().to_string()),
            process_id,
        }
    }
}

#[async_trait]
impl ProcessStore for ProcessObligationLifecycleStore {
    async fn start(&self, start: ProcessStart) -> Result<ProcessRecord, ProcessError> {
        let claimed = self.claim_active_process_handoff(&start)?;
        let process_id = start.process_id;
        let scope = start.scope.clone();
        let capability_id = start.capability_id.clone();
        match self.inner.start(start).await {
            Ok(record) => {
                self.emit_process_event(RuntimeEvent::process_started(
                    record.scope.clone(),
                    record.capability_id.clone(),
                    record.extension_id.clone(),
                    record.runtime,
                    record.process_id,
                ))
                .await;
                Ok(record)
            }
            Err(error) => {
                if claimed {
                    self.release_claimed_process_handoff(&scope, &capability_id, process_id)?;
                }
                Err(error)
            }
        }
    }

    async fn complete(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.complete(scope, process_id).await?;
        self.emit_process_event(RuntimeEvent::process_completed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        self.cleanup_terminal(&record, true)?;
        Ok(record)
    }

    async fn fail(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
        error_kind: String,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.fail(scope, process_id, error_kind).await?;
        self.emit_process_event(RuntimeEvent::process_failed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
            record
                .error_kind
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        ))
        .await;
        self.cleanup_terminal(&record, false)?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.kill(scope, process_id).await?;
        self.emit_process_event(RuntimeEvent::process_killed(
            record.scope.clone(),
            record.capability_id.clone(),
            record.extension_id.clone(),
            record.runtime,
            record.process_id,
        ))
        .await;
        self.cleanup_terminal(&record, false)?;
        Ok(record)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<Option<ProcessRecord>, ProcessError> {
        self.inner.get(scope, process_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ProcessRecord>, ProcessError> {
        self.inner.records_for_scope(scope).await
    }
}

fn close_reservation_once<T>(result: Result<T, ResourceError>) -> Result<(), ProcessError> {
    match result {
        Ok(_) => Ok(()),
        Err(ResourceError::ReservationClosed { .. }) => Ok(()),
        Err(ResourceError::UnknownReservation { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Built-in obligation handler for the current host-runtime slice.
#[derive(Clone, Default)]
pub struct BuiltinObligationHandler {
    audit_sink: Option<Arc<dyn AuditSink>>,
    security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    secret_injections: Option<Arc<RuntimeSecretInjectionStore>>,
    resource_governor: Option<Arc<dyn ResourceGovernor>>,
    credential_account_resolver: Option<Arc<dyn RuntimeCredentialAccountResolver>>,
}

impl BuiltinObligationHandler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_audit_sink<T>(mut self, sink: Arc<T>) -> Self
    where
        T: AuditSink + 'static,
    {
        let sink: Arc<dyn AuditSink> = sink;
        self.audit_sink = Some(sink);
        self
    }

    pub fn with_audit_sink_dyn(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = Some(sink);
        self
    }

    /// Wire in a [`SecurityAuditSink`] for boundary-decision recording.
    ///
    /// Currently consumed by the output-redaction (leak-detector) path in
    /// [`Self::complete_dispatch`]. Additional boundaries inside this handler
    /// will adopt the same sink in follow-up PRs; the wiring is intentionally
    /// optional so unconfigured callers keep working unchanged.
    pub fn with_security_audit_sink(mut self, sink: Arc<dyn SecurityAuditSink>) -> Self {
        self.security_audit_sink = Some(sink);
        self
    }

    pub(crate) fn with_network_policy_store(
        mut self,
        store: Arc<NetworkObligationPolicyStore>,
    ) -> Self {
        self.network_policies = Some(store);
        self
    }

    pub fn with_secret_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: SecretStore + 'static,
    {
        let store: Arc<dyn SecretStore> = store;
        self.secret_store = Some(store);
        self
    }

    pub fn with_secret_store_dyn(mut self, store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(store);
        self
    }

    pub(crate) fn with_secret_injection_store(
        mut self,
        store: Arc<RuntimeSecretInjectionStore>,
    ) -> Self {
        self.secret_injections = Some(store);
        self
    }

    pub fn with_resource_governor<T>(mut self, governor: Arc<T>) -> Self
    where
        T: ResourceGovernor + 'static,
    {
        let governor: Arc<dyn ResourceGovernor> = governor;
        self.resource_governor = Some(governor);
        self
    }

    pub fn with_resource_governor_dyn(mut self, governor: Arc<dyn ResourceGovernor>) -> Self {
        self.resource_governor = Some(governor);
        self
    }

    pub fn with_credential_account_resolver<T>(mut self, resolver: Arc<T>) -> Self
    where
        T: RuntimeCredentialAccountResolver + 'static,
    {
        let resolver: Arc<dyn RuntimeCredentialAccountResolver> = resolver;
        self.credential_account_resolver = Some(resolver);
        self
    }

    pub fn with_credential_account_resolver_dyn(
        mut self,
        resolver: Arc<dyn RuntimeCredentialAccountResolver>,
    ) -> Self {
        self.credential_account_resolver = Some(resolver);
        self
    }

    async fn emit_audit_before(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let Some(audit_sink) = &self.audit_sink else {
            return Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            });
        };

        audit_sink
            .emit_audit(audit_before_record(request))
            .await
            .map_err(|_| CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            })
    }

    async fn preflight_secret_injection(
        &self,
        request: &CapabilityObligationRequest<'_>,
        handles: &[SecretHandle],
    ) -> Result<(), CapabilityObligationError> {
        if handles.is_empty() {
            return Ok(());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        if self.secret_injections.is_none() {
            return Err(secret_obligation_failed());
        }
        for handle in handles {
            // Fail closed on a store error: the dispatch-time backstop must never
            // let an uncredentialed call through on a transient failure. Preserve the
            // cause as a server-side trail (`SecretStoreError` Display carries no raw
            // secret material — handles/reasons only); the caller still receives the
            // opaque, sanitized secret-obligation failure.
            let exists = match secret_present(
                secret_store.as_ref(),
                &request.context.resource_scope,
                handle,
            )
            .await
            {
                Ok(exists) => exists,
                Err(error) => {
                    tracing::debug!(
                        secret_handle = handle.as_str(),
                        error = %error,
                        "dispatch-time secret presence probe failed; failing closed at the obligation backstop"
                    );
                    return Err(secret_obligation_failed());
                }
            };
            if !exists {
                return Err(CapabilityObligationError::AuthRequired {
                    credential_requirements: Vec::new(),
                });
            }
        }
        Ok(())
    }

    async fn inject_secrets(
        &self,
        request: &CapabilityObligationRequest<'_>,
        handles: &[SecretHandle],
    ) -> Result<(), CapabilityObligationError> {
        if handles.is_empty() {
            return Ok(());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        let Some(secret_injections) = &self.secret_injections else {
            return Err(secret_obligation_failed());
        };

        let mut material = Vec::with_capacity(handles.len());
        for handle in handles {
            let lease = secret_store
                .lease_once(&request.context.resource_scope, handle)
                .await
                .map_err(|_| secret_obligation_failed())?;
            let secret = secret_store
                .consume(&request.context.resource_scope, lease.id)
                .await
                .map_err(|_| secret_obligation_failed())?;
            material.push((handle.clone(), secret));
        }

        for (handle, secret) in material {
            secret_injections
                .insert(
                    &request.context.resource_scope,
                    request.capability_id,
                    &handle,
                    secret,
                )
                .map_err(|_| secret_obligation_failed())?;
        }
        Ok(())
    }

    async fn inject_credential_accounts(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let account_obligations = credential_account_injection_obligations(request.obligations);
        if account_obligations.is_empty() {
            return Ok(());
        }
        let Some(resolver) = &self.credential_account_resolver else {
            return Err(secret_obligation_failed());
        };
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        let Some(secret_injections) = &self.secret_injections else {
            return Err(secret_obligation_failed());
        };

        for obligation in account_obligations {
            let access_secret = resolver
                .resolve_access_secret(RuntimeCredentialAccountRequest {
                    scope: &request.context.resource_scope,
                    provider: obligation.provider,
                    setup: obligation.setup,
                    provider_scopes: obligation.provider_scopes,
                    requester_extension: obligation.requester_extension,
                })
                .await
                .map_err(|error| {
                    credential_stage_error_to_obligation_error(error, Some(&obligation))
                })?;
            // Retrieve and stage the resolved credential under the obligation's injection handle.
            // The access_secret names the material in the secret store; obligation.handle is
            // the slot name the WASM guest expects.
            stage_credential_material(
                secret_store.as_ref(),
                secret_injections,
                &access_secret.scope,
                &request.context.resource_scope,
                request.capability_id,
                &access_secret.handle,
                obligation.handle,
            )
            .await
            .map_err(|error| {
                credential_stage_error_to_obligation_error(error, Some(&obligation))
            })?;
        }

        Ok(())
    }

    fn reserve_resource_obligation(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<Option<ResourceReservation>, CapabilityObligationError> {
        let mut reservation_id = None;
        for obligation in request.obligations {
            if let Obligation::ReserveResources { reservation_id: id } = obligation {
                if reservation_id.is_some() {
                    return Err(resource_obligation_failed());
                }
                reservation_id = Some(*id);
            }
        }
        let Some(reservation_id) = reservation_id else {
            return Ok(None);
        };
        let Some(governor) = &self.resource_governor else {
            return Err(resource_obligation_failed());
        };
        governor
            .reserve_with_id(
                request.context.resource_scope.clone(),
                request.estimate.clone(),
                reservation_id,
            )
            .map(Some)
            .map_err(|_| resource_obligation_failed())
    }

    fn preflight_resource_ceiling(
        &self,
        request: &CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        let Some(ceiling) = resource_ceiling_obligation(request.obligations)? else {
            return Ok(());
        };
        validate_supported_resource_ceiling(ceiling)?;
        validate_estimate_within_ceiling(request.estimate, ceiling)
    }

    async fn finish_prepare(
        &self,
        request: &CapabilityObligationRequest<'_>,
        secret_handles: &[SecretHandle],
        network_policy: Option<NetworkPolicy>,
    ) -> Result<(), CapabilityObligationError> {
        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            self.emit_audit_before(request).await?;
        }

        self.inject_secrets(request, secret_handles).await?;
        self.inject_credential_accounts(request).await?;

        if let Some(policy) = network_policy {
            let Some(store) = &self.network_policies else {
                return Err(network_obligation_failed());
            };
            store.insert(
                &request.context.resource_scope,
                request.capability_id,
                policy,
            );
        }

        Ok(())
    }

    async fn emit_audit_after(
        &self,
        request: &CapabilityObligationCompletionRequest<'_>,
        output_bytes: u64,
    ) -> Result<(), CapabilityObligationError> {
        let Some(audit_sink) = &self.audit_sink else {
            return Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            });
        };

        audit_sink
            .emit_audit(audit_after_record(request, output_bytes))
            .await
            .map_err(|_| CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Audit,
            })
    }
}

#[async_trait]
impl CapabilityObligationHandler for BuiltinObligationHandler {
    async fn satisfy(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        // `satisfy` is the direct one-shot path for callers that need staged
        // network/secret handoff but do not need to pass prepared mounts or a
        // reservation downstream. Resource reservations are released without
        // discarding staged handoffs because successful callers still need the
        // network/secret material handed to runtime adapters. CapabilityHost
        // uses `prepare`/`complete`/`abort` directly instead. Post-dispatch
        // obligations fail closed here because this path has no dispatch result
        // to redact, limit, or audit.
        let post_dispatch = post_dispatch_obligations(request.obligations);
        if !post_dispatch.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: post_dispatch,
            });
        }
        let outcome = self
            .prepare(CapabilityObligationRequest {
                phase: request.phase,
                context: request.context,
                capability_id: request.capability_id,
                estimate: request.estimate,
                obligations: request.obligations,
            })
            .await?;
        if let Some(reservation) = &outcome.resource_reservation
            && let Err(error) = self.release_resource_reservation(reservation)
        {
            let _ = self.discard_staged_handoffs(
                &request.context.resource_scope,
                request.capability_id,
                request.obligations,
            );
            return Err(error);
        }
        Ok(())
    }

    async fn prepare(
        &self,
        request: CapabilityObligationRequest<'_>,
    ) -> Result<CapabilityObligationOutcome, CapabilityObligationError> {
        let unsupported = unsupported_obligations(request.phase, request.obligations);
        if !unsupported.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            });
        }

        let network_policy = network_policy_obligation(request.obligations)?;
        if network_policy.is_some() && self.network_policies.is_none() {
            return Err(network_obligation_failed());
        }
        let scoped_mounts = scoped_mount_obligation(request.context, request.obligations)?;
        let secret_handles = secret_injection_handles(request.obligations);
        self.preflight_secret_injection(&request, &secret_handles)
            .await?;
        self.preflight_resource_ceiling(&request)?;
        let resource_reservation = self.reserve_resource_obligation(&request)?;
        let outcome = CapabilityObligationOutcome {
            mounts: scoped_mounts,
            resource_reservation,
        };

        if let Err(error) = self
            .finish_prepare(&request, &secret_handles, network_policy)
            .await
        {
            self.abort(CapabilityObligationAbortRequest {
                phase: request.phase,
                context: request.context,
                capability_id: request.capability_id,
                estimate: request.estimate,
                obligations: request.obligations,
                outcome: &outcome,
            })
            .await?;
            return Err(error);
        }

        Ok(outcome)
    }

    async fn abort(
        &self,
        request: CapabilityObligationAbortRequest<'_>,
    ) -> Result<(), CapabilityObligationError> {
        self.discard_staged_handoffs(
            &request.context.resource_scope,
            request.capability_id,
            request.obligations,
        )?;

        if let Some(reservation) = &request.outcome.resource_reservation {
            self.release_resource_reservation(reservation)?;
        }
        Ok(())
    }

    async fn complete_dispatch(
        &self,
        request: CapabilityObligationCompletionRequest<'_>,
    ) -> Result<CapabilityDispatchResult, CapabilityObligationError> {
        let unsupported = unsupported_completion_obligations(request.phase, request.obligations);
        if !unsupported.is_empty() {
            return Err(CapabilityObligationError::Unsupported {
                obligations: unsupported,
            });
        }

        let mut dispatch = request.dispatch.clone();
        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::RedactOutput))
        {
            dispatch.output = match redact_output(dispatch.output) {
                Ok(value) => value,
                Err(error) => {
                    // Leak-detector blocked: record the boundary decision
                    // before propagating. The event is payload-free by
                    // construction — only the boundary, decision, and a
                    // stable code reach the sink. The original output never
                    // leaves the type system.
                    if let Some(sink) = &self.security_audit_sink {
                        let event = SecurityAuditEvent::new(
                            SecurityBoundary::LeakDetector,
                            SecurityDecision::Blocked,
                            LEAK_REDACT_FAILED_CODE,
                        )
                        .with_capability_id(request.capability_id.clone())
                        .with_scope(request.context.resource_scope.clone());
                        sink.record(event);
                    }
                    return Err(error);
                }
            };
            dispatch.display_preview = None;
        }

        let output_bytes = dispatch_output_bytes(&dispatch.output)?;
        for obligation in request.obligations {
            if let Obligation::EnforceResourceCeiling { ceiling } = obligation {
                validate_supported_resource_ceiling(ceiling)?;
                validate_usage_within_ceiling(&dispatch.usage, output_bytes, ceiling)?;
            }
        }
        for obligation in request.obligations {
            if let Obligation::EnforceOutputLimit { bytes } = obligation
                && output_bytes > *bytes
            {
                return Err(output_obligation_failed());
            }
        }

        self.discard_staged_handoffs(
            &request.context.resource_scope,
            request.capability_id,
            request.obligations,
        )?;

        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditAfter))
        {
            self.emit_audit_after(&request, output_bytes).await?;
        }

        Ok(dispatch)
    }
}

impl BuiltinObligationHandler {
    fn release_resource_reservation(
        &self,
        reservation: &ResourceReservation,
    ) -> Result<(), CapabilityObligationError> {
        let Some(governor) = &self.resource_governor else {
            return Err(resource_obligation_failed());
        };
        governor
            .release(reservation.id)
            .map(|_| ())
            .map_err(|_| resource_obligation_failed())
    }

    fn discard_staged_handoffs(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        obligations: &[Obligation],
    ) -> Result<(), CapabilityObligationError> {
        if obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::ApplyNetworkPolicy { .. }))
            && let Some(store) = &self.network_policies
        {
            let _ = store.take(scope, capability_id);
        }

        if let Some(store) = &self.secret_injections {
            for handle in staged_secret_injection_handles(obligations) {
                let _ = store
                    .take(scope, capability_id, &handle)
                    .map_err(|_| secret_obligation_failed())?;
            }
        }

        Ok(())
    }
}

fn post_dispatch_obligations(obligations: &[Obligation]) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| {
            matches!(
                obligation,
                Obligation::AuditAfter
                    | Obligation::RedactOutput
                    | Obligation::EnforceResourceCeiling { .. }
                    | Obligation::EnforceOutputLimit { .. }
            )
        })
        .cloned()
        .collect()
}

fn unsupported_obligations(
    phase: CapabilityObligationPhase,
    obligations: &[Obligation],
) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| !obligation_supported_before_dispatch(phase, obligation))
        .cloned()
        .collect()
}

fn obligation_supported_before_dispatch(
    phase: CapabilityObligationPhase,
    obligation: &Obligation,
) -> bool {
    match obligation {
        Obligation::AuditBefore
        | Obligation::ApplyNetworkPolicy { .. }
        | Obligation::FirstPartyCredentialStagedViaHostPort { .. }
        | Obligation::InjectCredentialAccountOnce { .. }
        | Obligation::InjectSecretOnce { .. }
        | Obligation::ReserveResources { .. }
        | Obligation::UseScopedMounts { .. } => true,
        Obligation::EnforceResourceCeiling { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
        Obligation::AuditAfter
        | Obligation::RedactOutput
        | Obligation::EnforceOutputLimit { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
    }
}

fn unsupported_completion_obligations(
    phase: CapabilityObligationPhase,
    obligations: &[Obligation],
) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| !obligation_supported_after_dispatch(phase, obligation))
        .cloned()
        .collect()
}

fn obligation_supported_after_dispatch(
    phase: CapabilityObligationPhase,
    obligation: &Obligation,
) -> bool {
    match obligation {
        Obligation::AuditBefore
        | Obligation::ApplyNetworkPolicy { .. }
        | Obligation::FirstPartyCredentialStagedViaHostPort { .. }
        | Obligation::InjectCredentialAccountOnce { .. }
        | Obligation::InjectSecretOnce { .. }
        | Obligation::ReserveResources { .. }
        | Obligation::UseScopedMounts { .. } => true,
        Obligation::EnforceResourceCeiling { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
        Obligation::AuditAfter
        | Obligation::RedactOutput
        | Obligation::EnforceOutputLimit { .. } => {
            !matches!(phase, CapabilityObligationPhase::Spawn)
        }
    }
}

fn secret_injection_handles(obligations: &[Obligation]) -> Vec<SecretHandle> {
    obligations
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::InjectSecretOnce { handle } => Some(handle.clone()),
            _ => None,
        })
        .collect()
}

struct CredentialAccountInjectionObligation<'a> {
    handle: &'a SecretHandle,
    provider: &'a RuntimeCredentialAccountProviderId,
    setup: &'a RuntimeCredentialAccountSetup,
    provider_scopes: &'a [String],
    requester_extension: &'a ExtensionId,
}

fn credential_account_injection_obligations(
    obligations: &[Obligation],
) -> Vec<CredentialAccountInjectionObligation<'_>> {
    obligations
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::InjectCredentialAccountOnce {
                handle,
                provider,
                setup,
                provider_scopes,
                requester_extension,
            } => Some(CredentialAccountInjectionObligation {
                handle,
                provider,
                setup,
                provider_scopes,
                requester_extension,
            }),
            _ => None,
        })
        .collect()
}

fn staged_secret_injection_handles(obligations: &[Obligation]) -> Vec<SecretHandle> {
    obligations
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::InjectSecretOnce { handle }
            | Obligation::InjectCredentialAccountOnce { handle, .. } => Some(handle.clone()),
            _ => None,
        })
        .collect()
}

/// Map the canonical staged-credential error to the obligation-handler error type.
///
/// Used by both [`inject_credential_accounts`] (resolver-side errors) and
/// [`stage_credential_material`] (storage-side errors) so the WASM
/// `InjectCredentialAccountOnce` path and the first-party stager path share
/// the same AuthRequired/Backend semantics.
fn credential_stage_error_to_obligation_error(
    error: CredentialStageError,
    credential_obligation: Option<&CredentialAccountInjectionObligation<'_>>,
) -> CapabilityObligationError {
    match error {
        CredentialStageError::AuthRequired => CapabilityObligationError::AuthRequired {
            credential_requirements: credential_obligation
                .map(|obligation| {
                    vec![RuntimeCredentialAuthRequirement {
                        provider: obligation.provider.clone(),
                        setup: obligation.setup.clone(),
                        requester_extension: obligation.requester_extension.clone(),
                        provider_scopes: obligation.provider_scopes.to_vec(),
                    }]
                })
                .unwrap_or_default(),
        },
        CredentialStageError::Backend => secret_obligation_failed(),
    }
}

/// Retrieve `source` from the secret store and stage the material under `target`
/// in the injection store for the given capability invocation.
///
/// Used when the secret store key (`source`) differs from the runtime injection slot
/// (`target`) — for example, when a product-auth account's backing secret is resolved
/// to a concrete handle before being injected under the WASM guest's declared slot name.
/// Lease → consume → insert the staged credential material.
///
/// Mirrors [`crate::services::ProductAuthProviderRuntimePorts::stage_secret_once`]
/// so the WASM `InjectCredentialAccountOnce` path and the first-party stager path
/// (e.g. `ProductAuthRuntimeGsuiteCredentialStager`) share identical lease/consume
/// semantics and `CredentialStageError` mapping. `SecretStoreError` variants for
/// unknown/expired/revoked/consumed material map to
/// [`CredentialStageError::AuthRequired`] via [`crate::services::stage_secret_error`];
/// other failures map to [`CredentialStageError::Backend`].
async fn stage_credential_material(
    secret_store: &dyn SecretStore,
    secret_injections: &RuntimeSecretInjectionStore,
    source_scope: &ResourceScope,
    target_scope: &ResourceScope,
    capability_id: &CapabilityId,
    source: &SecretHandle,
    target: &SecretHandle,
) -> Result<(), CredentialStageError> {
    let lease = secret_store
        .lease_once(source_scope, source)
        .await
        .map_err(|e| {
            tracing::debug!(err = %e, "stage_credential_material: lease_once failed");
            crate::services::stage_secret_error(e)
        })?;
    let secret = secret_store
        .consume(source_scope, lease.id)
        .await
        .map_err(|e| {
            tracing::debug!(err = %e, "stage_credential_material: consume failed");
            crate::services::stage_secret_error(e)
        })?;
    secret_injections
        .insert(target_scope, capability_id, target, secret)
        .map_err(|e| {
            tracing::debug!(err = %e, "stage_credential_material: insert failed");
            CredentialStageError::Backend
        })
}

fn network_policy_obligation(
    obligations: &[Obligation],
) -> Result<Option<NetworkPolicy>, CapabilityObligationError> {
    let mut policy = None;
    for obligation in obligations {
        if let Obligation::ApplyNetworkPolicy { policy: next } = obligation {
            if policy.is_some() {
                return Err(network_obligation_failed());
            }
            validate_network_policy_metadata(next)?;
            policy = Some(next.clone());
        }
    }
    Ok(policy)
}

fn scoped_mount_obligation(
    context: &ironclaw_host_api::ExecutionContext,
    obligations: &[Obligation],
) -> Result<Option<MountView>, CapabilityObligationError> {
    let mut mounts = None;
    for obligation in obligations {
        if let Obligation::UseScopedMounts { mounts: next } = obligation {
            if mounts.is_some() {
                return Err(mount_obligation_failed());
            }
            next.validate().map_err(|_| mount_obligation_failed())?;
            if !next.is_subset_of(&context.mounts) {
                return Err(mount_obligation_failed());
            }
            mounts = Some(next.clone());
        }
    }
    Ok(mounts)
}

fn resource_ceiling_obligation(
    obligations: &[Obligation],
) -> Result<Option<&ResourceCeiling>, CapabilityObligationError> {
    let mut ceiling = None;
    for obligation in obligations {
        if let Obligation::EnforceResourceCeiling { ceiling: next } = obligation {
            if ceiling.is_some() {
                return Err(resource_obligation_failed());
            }
            ceiling = Some(next);
        }
    }
    Ok(ceiling)
}

fn validate_supported_resource_ceiling(
    ceiling: &ResourceCeiling,
) -> Result<(), CapabilityObligationError> {
    if ceiling.max_wall_clock_ms.is_some() {
        return Err(resource_obligation_failed());
    }
    if let Some(sandbox) = &ceiling.sandbox {
        validate_supported_sandbox_quota(sandbox)?;
    }
    Ok(())
}

fn validate_supported_sandbox_quota(
    sandbox: &SandboxQuota,
) -> Result<(), CapabilityObligationError> {
    if sandbox.cpu_time_ms.is_some()
        || sandbox.memory_bytes.is_some()
        || sandbox.disk_bytes.is_some()
        || sandbox.network_egress_bytes.is_some()
        || sandbox.process_count.is_some()
    {
        return Err(resource_obligation_failed());
    }
    Ok(())
}

fn validate_estimate_within_ceiling(
    estimate: &ResourceEstimate,
    ceiling: &ResourceCeiling,
) -> Result<(), CapabilityObligationError> {
    check_optional_decimal_ceiling(estimate.usd, ceiling.max_usd)?;
    check_required_integer_ceiling(estimate.input_tokens, ceiling.max_input_tokens)?;
    check_required_integer_ceiling(estimate.output_tokens, ceiling.max_output_tokens)?;
    Ok(())
}

fn validate_usage_within_ceiling(
    usage: &ResourceUsage,
    output_bytes: u64,
    ceiling: &ResourceCeiling,
) -> Result<(), CapabilityObligationError> {
    check_decimal_ceiling(usage.usd, ceiling.max_usd)?;
    check_integer_ceiling(usage.input_tokens, ceiling.max_input_tokens)?;
    check_integer_ceiling(usage.output_tokens, ceiling.max_output_tokens)?;
    check_output_bytes_ceiling(output_bytes, ceiling.max_output_bytes)?;
    Ok(())
}

fn check_output_bytes_ceiling(
    actual: u64,
    ceiling: Option<u64>,
) -> Result<(), CapabilityObligationError> {
    if let Some(ceiling) = ceiling
        && actual > ceiling
    {
        return Err(output_obligation_failed());
    }
    Ok(())
}

fn check_optional_decimal_ceiling(
    actual: Option<rust_decimal::Decimal>,
    ceiling: Option<rust_decimal::Decimal>,
) -> Result<(), CapabilityObligationError> {
    let Some(ceiling) = ceiling else {
        return Ok(());
    };
    let Some(actual) = actual else {
        return Err(resource_obligation_failed());
    };
    check_decimal_ceiling(actual, Some(ceiling))
}

fn check_decimal_ceiling(
    actual: rust_decimal::Decimal,
    ceiling: Option<rust_decimal::Decimal>,
) -> Result<(), CapabilityObligationError> {
    if let Some(ceiling) = ceiling
        && actual > ceiling
    {
        return Err(resource_obligation_failed());
    }
    Ok(())
}

fn check_required_integer_ceiling(
    actual: Option<u64>,
    ceiling: Option<u64>,
) -> Result<(), CapabilityObligationError> {
    let Some(ceiling) = ceiling else {
        return Ok(());
    };
    let Some(actual) = actual else {
        return Err(resource_obligation_failed());
    };
    check_integer_ceiling(actual, Some(ceiling))
}

fn check_integer_ceiling(
    actual: u64,
    ceiling: Option<u64>,
) -> Result<(), CapabilityObligationError> {
    if let Some(ceiling) = ceiling
        && actual > ceiling
    {
        return Err(resource_obligation_failed());
    }
    Ok(())
}

fn validate_network_policy_metadata(
    policy: &NetworkPolicy,
) -> Result<(), CapabilityObligationError> {
    if policy.allowed_targets.is_empty() {
        return Err(network_obligation_failed());
    }
    Ok(())
}

fn network_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Network,
    }
}

fn secret_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Secret,
    }
}

/// Single source of truth for "is this required secret present in `scope`".
///
/// Both the credential pre-flight (ordering — `DefaultHostRuntime::
/// credential_preflight_check`) and the dispatch-time obligation backstop
/// (enforcement — [`BuiltinObligationHandler::preflight_secret_injection`])
/// consult this one rule so "what counts as a present credential" cannot drift
/// between the two call sites. Each caller decides how to treat a store `Err`
/// (the pre-flight fails open and skips; the obligation backstop fails closed).
pub(crate) async fn secret_present(
    store: &dyn SecretStore,
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<bool, SecretStoreError> {
    Ok(store.metadata(scope, handle).await?.is_some())
}

fn resource_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Resource,
    }
}

fn mount_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Mount,
    }
}

fn output_obligation_failed() -> CapabilityObligationError {
    CapabilityObligationError::Failed {
        kind: CapabilityObligationFailureKind::Output,
    }
}

fn dispatch_output_bytes(output: &serde_json::Value) -> Result<u64, CapabilityObligationError> {
    serde_json::to_vec(output)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| output_obligation_failed())
}

/// Security-audit reason code emitted when [`redact_output`] rejects output
/// because the leak detector matched. Stable grep target for SRE pattern
/// matching across durable security-audit logs.
pub const LEAK_REDACT_FAILED_CODE: &str = "leak_redact_failed";

fn redact_output(
    output: serde_json::Value,
) -> Result<serde_json::Value, CapabilityObligationError> {
    match output {
        serde_json::Value::String(value) => {
            redact_output_string(value).map(serde_json::Value::String)
        }
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(redact_output)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(entries) => {
            let mut redacted = serde_json::Map::with_capacity(entries.len());
            for (key, value) in entries {
                let key = redact_output_string(key)?;
                let value = redact_output(value)?;
                if redacted.insert(key, value).is_some() {
                    return Err(output_obligation_failed());
                }
            }
            Ok(serde_json::Value::Object(redacted))
        }
        value => Ok(value),
    }
}

fn redact_output_string(value: String) -> Result<String, CapabilityObligationError> {
    LeakDetector::new()
        .scan_and_clean(&value)
        .map_err(|_| output_obligation_failed())
}

fn audit_before_record(request: &CapabilityObligationRequest<'_>) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: request.context.correlation_id,
        stage: AuditStage::Before,
        timestamp: Utc::now(),
        tenant_id: request.context.tenant_id.clone(),
        user_id: request.context.user_id.clone(),
        agent_id: request.context.agent_id.clone(),
        project_id: request.context.project_id.clone(),
        mission_id: request.context.mission_id.clone(),
        thread_id: request.context.thread_id.clone(),
        invocation_id: request.context.invocation_id,
        process_id: request.context.process_id,
        approval_request_id: None,
        extension_id: Some(request.context.extension_id.clone()),
        action: ActionSummary {
            kind: capability_action_kind(request.phase).to_string(),
            target: Some(request.capability_id.as_str().to_string()),
            effects: capability_action_effects(request.phase),
        },
        decision: DecisionSummary {
            kind: "obligation_satisfied".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(obligation_status(request.obligations)),
            output_bytes: None,
        }),
    }
}

fn audit_after_record(
    request: &CapabilityObligationCompletionRequest<'_>,
    output_bytes: u64,
) -> AuditEnvelope {
    AuditEnvelope {
        event_id: AuditEventId::new(),
        correlation_id: request.context.correlation_id,
        stage: AuditStage::After,
        timestamp: Utc::now(),
        tenant_id: request.context.tenant_id.clone(),
        user_id: request.context.user_id.clone(),
        agent_id: request.context.agent_id.clone(),
        project_id: request.context.project_id.clone(),
        mission_id: request.context.mission_id.clone(),
        thread_id: request.context.thread_id.clone(),
        invocation_id: request.context.invocation_id,
        process_id: request.context.process_id,
        approval_request_id: None,
        extension_id: Some(request.context.extension_id.clone()),
        action: ActionSummary {
            kind: capability_action_kind(request.phase).to_string(),
            target: Some(request.capability_id.as_str().to_string()),
            effects: capability_action_effects(request.phase),
        },
        decision: DecisionSummary {
            kind: "obligation_satisfied".to_string(),
            reason: None,
            actor: None,
        },
        result: Some(ActionResultSummary {
            success: true,
            status: Some(obligation_status(request.obligations)),
            output_bytes: Some(output_bytes),
        }),
    }
}

fn capability_action_kind(phase: CapabilityObligationPhase) -> &'static str {
    match phase {
        CapabilityObligationPhase::Invoke => "capability_invoke",
        CapabilityObligationPhase::Resume => "capability_resume",
        CapabilityObligationPhase::Spawn => "capability_spawn",
    }
}

fn capability_action_effects(phase: CapabilityObligationPhase) -> Vec<EffectKind> {
    match phase {
        CapabilityObligationPhase::Invoke | CapabilityObligationPhase::Resume => {
            vec![EffectKind::DispatchCapability]
        }
        CapabilityObligationPhase::Spawn => {
            vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess]
        }
    }
}

fn obligation_status(obligations: &[Obligation]) -> String {
    obligations
        .iter()
        .filter_map(obligation_label)
        .collect::<Vec<_>>()
        .join(",")
}

fn obligation_label(obligation: &Obligation) -> Option<&'static str> {
    match obligation {
        Obligation::AuditBefore => Some("audit_before"),
        Obligation::AuditAfter => Some("audit_after"),
        Obligation::RedactOutput => Some("redact_output"),
        Obligation::ApplyNetworkPolicy { .. } => Some("apply_network_policy"),
        Obligation::InjectSecretOnce { .. } => Some("inject_secret_once"),
        Obligation::InjectCredentialAccountOnce { .. } => Some("inject_credential_account_once"),
        Obligation::FirstPartyCredentialStagedViaHostPort { .. } => {
            Some("first_party_credential_staged_via_host_port")
        }
        Obligation::EnforceOutputLimit { .. } => Some("enforce_output_limit"),
        Obligation::ReserveResources { .. } => Some("reserve_resources"),
        Obligation::UseScopedMounts { .. } => Some("use_scoped_mounts"),
        Obligation::EnforceResourceCeiling { .. } => Some("enforce_resource_ceiling"),
    }
}

/// **Finding H2 — compile-time regression guard.**
///
/// The original H2 claim was that `RuntimeSecretInjectionStore`'s
/// `HashMap<_, RuntimeSecretInjectionEntry>` would bitwise-copy plaintext out
/// of the old bucket array on rehash and free it without zeroization. On
/// closer inspection that does *not* happen, because `SecretMaterial =
/// secrecy::SecretBox<str>`: the rehash moves a `Box<str>` pointer plus the
/// `Instant`, while the actual buffer stays at its original heap address
/// until `SecretBox::drop` zeroizes it.
///
/// The protection is real but depends on the staged entry's `material` field
/// being a `ZeroizeOnDrop` carrier. If it ever swaps to a non-zeroizing type
/// (plain `String`, `Vec<u8>`, etc.), the bitwise-copy concern returns. This
/// `const _: fn(...) = ...` references the field through a
/// `ZeroizeOnDrop`-bounded helper, so the swap is rejected at compile time
/// rather than only failing a test run. The function is never called — only
/// type-checked.
const _: fn(&RuntimeSecretInjectionEntry) = |entry| {
    fn require_zeroize_on_drop<T: ?Sized + secrecy::zeroize::ZeroizeOnDrop>(_: &T) {}
    require_zeroize_on_drop(&entry.material);
};

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use ironclaw_events::InMemoryAuditSink;
    use ironclaw_host_api::{
        AgentId, CapabilityDisplayOutputPreview, CapabilitySet, CorrelationId, ExecutionContext,
        ExtensionId, InvocationId, NetworkScheme, NetworkTargetPattern, ProjectId,
        ResourceReservationId, RuntimeKind, TenantId, TrustClass, UserId,
    };
    use ironclaw_resources::{InMemoryResourceGovernor, ResourceAccount};
    use ironclaw_secrets::InMemorySecretStore;

    use super::*;

    #[tokio::test]
    async fn runtime_secret_injection_store_prunes_expired_handoffs() {
        let store = RuntimeSecretInjectionStore::with_ttl(Duration::from_millis(5));
        let scope = resource_scope_with_agent("agent-a");
        let capability_id = capability_id();
        let handle = SecretHandle::new("api_token").unwrap();

        store
            .insert(
                &scope,
                &capability_id,
                &handle,
                SecretMaterial::from("runtime-secret"),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(store.prune_expired().unwrap(), 1);
        assert!(
            store
                .take(&scope, &capability_id, &handle)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn network_obligation_policy_store_isolates_agent_scope() {
        let store = NetworkObligationPolicyStore::new();
        let (agent_a, agent_b) = same_invocation_agent_scopes();
        let capability_id = capability_id();

        store.insert(&agent_a, &capability_id, allowed_network_policy());

        assert!(store.take(&agent_b, &capability_id).is_none());
        assert!(store.take(&agent_a, &capability_id).is_some());
    }

    #[test]
    fn runtime_secret_injection_store_isolates_agent_scope() {
        let store = RuntimeSecretInjectionStore::new();
        let (agent_a, agent_b) = same_invocation_agent_scopes();
        let capability_id = capability_id();
        let handle = SecretHandle::new("api_token").unwrap();

        store
            .insert(
                &agent_a,
                &capability_id,
                &handle,
                SecretMaterial::from("runtime-secret"),
            )
            .unwrap();

        assert!(
            store
                .take(&agent_b, &capability_id, &handle)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .take(&agent_a, &capability_id, &handle)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn builtin_obligation_handler_satisfy_release_preserves_staged_handoffs() {
        let network_policies = Arc::new(NetworkObligationPolicyStore::new());
        let secret_injections = Arc::new(RuntimeSecretInjectionStore::new());
        let secret_store = Arc::new(InMemorySecretStore::new());
        let governor = Arc::new(InMemoryResourceGovernor::new());
        let services = BuiltinObligationServices::with_handoff_stores(
            Arc::new(InMemoryAuditSink::new()),
            network_policies.clone(),
            secret_store.clone(),
            secret_injections.clone(),
            governor.clone(),
        );
        let handler = services.obligation_handler();
        let context = execution_context();
        let account = ResourceAccount::tenant(context.resource_scope.tenant_id.clone());
        let capability_id = capability_id();
        let handle = SecretHandle::new("api_token").unwrap();
        let estimate = ResourceEstimate {
            concurrency_slots: Some(1),
            ..ResourceEstimate::default()
        };
        secret_store
            .put(
                context.resource_scope.clone(),
                handle.clone(),
                SecretMaterial::from("runtime-secret"),
                None,
            )
            .await
            .unwrap();
        let obligations = vec![
            Obligation::ApplyNetworkPolicy {
                policy: allowed_network_policy(),
            },
            Obligation::InjectSecretOnce {
                handle: handle.clone(),
            },
            Obligation::ReserveResources {
                reservation_id: ResourceReservationId::new(),
            },
        ];

        handler
            .satisfy(CapabilityObligationRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id: &capability_id,
                estimate: &estimate,
                obligations: &obligations,
            })
            .await
            .unwrap();

        assert_eq!(governor.reserved_for(&account).concurrency_slots, 0);
        assert!(
            network_policies
                .take(&context.resource_scope, &capability_id)
                .is_some()
        );
        assert!(
            secret_injections
                .take(&context.resource_scope, &capability_id, &handle)
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn redact_output_clears_display_preview_side_channel() {
        use ironclaw_host_api::{ReservationStatus, ResourceReceipt, ResourceUsage, RuntimeKind};

        let services = BuiltinObligationServices::with_handoff_stores(
            Arc::new(InMemoryAuditSink::new()),
            Arc::new(NetworkObligationPolicyStore::new()),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(RuntimeSecretInjectionStore::new()),
            Arc::new(InMemoryResourceGovernor::new()),
        );
        let handler = services.obligation_handler();
        let context = execution_context();
        let capability_id = capability_id();
        let estimate = ResourceEstimate::default();
        let obligations = vec![Obligation::RedactOutput];
        let dispatch = CapabilityDispatchResult {
            capability_id: capability_id.clone(),
            provider: context.extension_id.clone(),
            runtime: RuntimeKind::Wasm,
            output: serde_json::json!({"secret": "sk-secret", "safe": "ok"}),
            display_preview: Some(CapabilityDisplayOutputPreview {
                output_summary: Some("contains secret".to_string()),
                output_preview: "sk-secret".to_string(),
                output_kind: "text".to_string(),
                subtitle: None,
                truncated: false,
            }),
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: context.resource_scope.clone(),
                status: ReservationStatus::Released,
                estimate: ResourceEstimate::default(),
                actual: None,
            },
        };

        let completed = handler
            .complete_dispatch(CapabilityObligationCompletionRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id: &capability_id,
                estimate: &estimate,
                obligations: &obligations,
                dispatch: &dispatch,
            })
            .await
            .expect("redacted dispatch completes");

        assert!(completed.display_preview.is_none());
        assert_eq!(completed.output["safe"], serde_json::json!("ok"));
    }

    #[tokio::test]
    async fn leak_detector_block_records_security_audit_event_through_complete_dispatch() {
        use ironclaw_events::{
            InMemorySecurityAuditSink, SecurityAuditSink, SecurityBoundary, SecurityDecision,
        };
        use ironclaw_host_api::{ReservationStatus, ResourceReceipt, ResourceUsage, RuntimeKind};

        // Build a handler with both an audit sink (unused here — we hit the
        // redact branch, not the AuditAfter branch) and a recording
        // security-audit sink. Other backing stores are not exercised by
        // the redact-only path, but the handler requires them to be set
        // for safety; we install minimal in-memory ones.
        let security_sink: Arc<InMemorySecurityAuditSink> =
            Arc::new(InMemorySecurityAuditSink::new());
        let security_sink_dyn: Arc<dyn SecurityAuditSink> = security_sink.clone();

        let services = BuiltinObligationServices::with_handoff_stores(
            Arc::new(InMemoryAuditSink::new()),
            Arc::new(NetworkObligationPolicyStore::new()),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(RuntimeSecretInjectionStore::new()),
            Arc::new(InMemoryResourceGovernor::new()),
        );
        let handler = services
            .obligation_handler()
            .with_security_audit_sink(security_sink_dyn);

        let context = execution_context();
        let capability_id = capability_id();
        let estimate = ResourceEstimate::default();
        let obligations = vec![Obligation::RedactOutput];

        // An AWS access-key shaped string is a built-in BLOCK pattern in
        // `ironclaw_safety::LeakDetector` (`AKIA[0-9A-Z]{16}`). Per the
        // module invariant we drive the *caller* (`complete_dispatch`),
        // not the helper, and assert the recorded event:
        //   - boundary  == LeakDetector
        //   - decision  == Blocked
        //   - code      == LEAK_REDACT_FAILED_CODE
        //   - capability_id + scope are populated
        //   - no payload (the offending string never appears in the event)
        let leaky_payload =
            serde_json::Value::String("hello AKIAIOSFODNN7EXAMPLE goodbye".to_string());
        let dispatch = CapabilityDispatchResult {
            capability_id: capability_id.clone(),
            provider: context.extension_id.clone(),
            runtime: RuntimeKind::Wasm,
            output: leaky_payload,
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: context.resource_scope.clone(),
                status: ReservationStatus::Released,
                estimate: ResourceEstimate::default(),
                actual: None,
            },
        };

        let request = CapabilityObligationCompletionRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id: &capability_id,
            estimate: &estimate,
            obligations: &obligations,
            dispatch: &dispatch,
        };

        let result = handler.complete_dispatch(request).await;
        assert!(
            matches!(
                result,
                Err(CapabilityObligationError::Failed {
                    kind: CapabilityObligationFailureKind::Output
                })
            ),
            "expected output-obligation failure, got {result:?}"
        );

        let events = security_sink.snapshot();
        assert_eq!(
            events.len(),
            1,
            "exactly one boundary decision should have been recorded, got {events:?}"
        );
        let event = &events[0];
        assert_eq!(event.boundary, SecurityBoundary::LeakDetector);
        assert_eq!(event.decision, SecurityDecision::Blocked);
        assert_eq!(event.code, LEAK_REDACT_FAILED_CODE);
        assert_eq!(event.code, "leak_redact_failed"); // stability lock
        assert_eq!(event.capability_id.as_ref(), Some(&capability_id));
        assert_eq!(event.scope.as_ref(), Some(&context.resource_scope));

        // The `SecurityAuditEvent` shape has no free-form payload field.
        // That invariant is enforced at the type level by the absence of
        // a `String` member on the struct. The check below is therefore a
        // documentation-only assertion: it locks the field set at the
        // value level for future readers, but the real guard is the type
        // shape in `ironclaw_events::security_audit`.
        //
        //   pub struct SecurityAuditEvent {
        //       pub boundary: SecurityBoundary,
        //       pub decision: SecurityDecision,
        //       pub capability_id: Option<CapabilityId>,
        //       pub scope: Option<ResourceScope>,
        //       pub timestamp: SystemTime,
        //       pub code: &'static str,
        //   }
    }

    #[tokio::test]
    async fn leak_detector_block_without_security_sink_does_not_panic() {
        use ironclaw_host_api::{ReservationStatus, ResourceReceipt, ResourceUsage, RuntimeKind};

        let services = BuiltinObligationServices::with_handoff_stores(
            Arc::new(InMemoryAuditSink::new()),
            Arc::new(NetworkObligationPolicyStore::new()),
            Arc::new(InMemorySecretStore::new()),
            Arc::new(RuntimeSecretInjectionStore::new()),
            Arc::new(InMemoryResourceGovernor::new()),
        );
        // No `.with_security_audit_sink(...)` — confirms the sink is
        // optional and the original failure semantics are preserved.
        let handler = services.obligation_handler();

        let context = execution_context();
        let capability_id = capability_id();
        let estimate = ResourceEstimate::default();
        let obligations = vec![Obligation::RedactOutput];
        let dispatch = CapabilityDispatchResult {
            capability_id: capability_id.clone(),
            provider: context.extension_id.clone(),
            runtime: RuntimeKind::Wasm,
            output: serde_json::Value::String("leak AKIAIOSFODNN7EXAMPLE".to_string()),
            display_preview: None,
            usage: ResourceUsage::default(),
            receipt: ResourceReceipt {
                id: ResourceReservationId::new(),
                scope: context.resource_scope.clone(),
                status: ReservationStatus::Released,
                estimate: ResourceEstimate::default(),
                actual: None,
            },
        };

        let result = handler
            .complete_dispatch(CapabilityObligationCompletionRequest {
                phase: CapabilityObligationPhase::Invoke,
                context: &context,
                capability_id: &capability_id,
                estimate: &estimate,
                obligations: &obligations,
                dispatch: &dispatch,
            })
            .await;
        assert!(matches!(
            result,
            Err(CapabilityObligationError::Failed {
                kind: CapabilityObligationFailureKind::Output
            })
        ));
    }

    fn same_invocation_agent_scopes() -> (ResourceScope, ResourceScope) {
        let mut agent_a = resource_scope_with_agent("agent-a");
        agent_a.invocation_id = InvocationId::new();
        let mut agent_b = agent_a.clone();
        agent_b.agent_id = Some(AgentId::new("agent-b").unwrap());
        (agent_a, agent_b)
    }

    fn resource_scope_with_agent(agent_id: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: Some(AgentId::new(agent_id).unwrap()),
            project_id: Some(ProjectId::new("project1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn execution_context() -> ExecutionContext {
        let invocation_id = InvocationId::new();
        let resource_scope = ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: Some(AgentId::new("agent-a").unwrap()),
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
            grants: CapabilitySet::default(),
            mounts: MountView::default(),
            resource_scope,
        }
    }

    fn capability_id() -> CapabilityId {
        CapabilityId::new("echo.say").unwrap()
    }

    fn allowed_network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.example.test".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: Some(1024),
        }
    }
}
