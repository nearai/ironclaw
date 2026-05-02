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
use ironclaw_events::AuditSink;
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AuditEnvelope, AuditEventId, AuditStage,
    CapabilityDispatchResult, CapabilityId, DecisionSummary, EffectKind, MountView, NetworkPolicy,
    Obligation, ProcessId, ResourceCeiling, ResourceEstimate, ResourceReservation, ResourceScope,
    ResourceUsage, SandboxQuota, SecretHandle,
};
use ironclaw_processes::{ProcessError, ProcessRecord, ProcessStart, ProcessStore};
use ironclaw_resources::{ResourceError, ResourceGovernor};
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::{SecretMaterial, SecretStore};

/// Default maximum lifetime for one-shot runtime secret material staged in memory.
pub const DEFAULT_RUNTIME_SECRET_INJECTION_TTL: Duration = Duration::from_secs(300);

/// One-shot runtime secret material staged after `InjectSecretOnce` lease consumption.
///
/// The store is keyed by scoped invocation, capability, and handle. Runtime adapters
/// must use `take(...)` so staged material is removed before it can be reused.
/// Entries also expire after a short TTL so abandoned handoffs from setup
/// failures, cancellation, or adapter bugs cannot remain usable indefinitely.
#[derive(Clone)]
pub struct RuntimeSecretInjectionStore {
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            state: Arc::new(RuntimeSecretInjectionState {
                secrets: Mutex::new(HashMap::new()),
                ttl,
            }),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.state.ttl
    }

    pub fn insert(
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

    pub fn take(
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

    pub fn prune_expired(&self) -> Result<usize, RuntimeSecretInjectionStoreError> {
        let mut secrets = self.lock()?;
        Ok(prune_expired_entries(&mut secrets, Instant::now()))
    }

    /// Discard all staged secrets for a scoped capability before process ownership exists.
    ///
    /// Background process lifecycle cleanup is guarded by a single-active-handoff
    /// invariant for the scoped capability; this method remains the abort/inline cleanup seam.
    pub fn discard_for_capability(
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
pub enum RuntimeSecretInjectionStoreError {
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
/// capability id, and are consumed by runtime adapters immediately before the
/// actual runtime dispatch.
#[derive(Debug, Clone, Default)]
pub struct NetworkObligationPolicyStore {
    policies: Arc<Mutex<HashMap<NetworkPolicyKey, NetworkPolicy>>>,
}

impl NetworkObligationPolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
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

    pub fn take(
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
    pub fn discard_for_capability(&self, scope: &ResourceScope, capability_id: &CapabilityId) {
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

    pub fn with_handoff_stores(
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
        }
    }

    pub fn audit_sink(&self) -> Arc<dyn AuditSink> {
        self.audit_sink.clone()
    }

    pub fn network_policy_store(&self) -> Arc<NetworkObligationPolicyStore> {
        self.network_policies.clone()
    }

    pub fn secret_store(&self) -> Arc<dyn SecretStore> {
        self.secret_store.clone()
    }

    pub fn secret_injection_store(&self) -> Arc<RuntimeSecretInjectionStore> {
        self.secret_injections.clone()
    }

    pub fn resource_governor(&self) -> Arc<dyn ResourceGovernor> {
        self.resource_governor.clone()
    }

    pub fn obligation_handler(&self) -> BuiltinObligationHandler {
        BuiltinObligationHandler::new()
            .with_audit_sink_dyn(self.audit_sink.clone())
            .with_network_policy_store(self.network_policies.clone())
            .with_secret_store_dyn(self.secret_store.clone())
            .with_secret_injection_store(self.secret_injections.clone())
            .with_resource_governor_dyn(self.resource_governor.clone())
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
            .finish()
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
    active_process_handoffs: Mutex<HashMap<ProcessObligationHandoffKey, ProcessId>>,
    cleaned_process_handoffs: Mutex<HashSet<ProcessObligationProcessKey>>,
}

impl ProcessObligationLifecycleStore {
    pub fn new<S>(
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

    pub fn from_dyn(
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
            active_process_handoffs: Mutex::new(HashMap::new()),
            cleaned_process_handoffs: Mutex::new(HashSet::new()),
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
            Ok(record) => Ok(record),
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
        self.cleanup_terminal(&record, false)?;
        Ok(record)
    }

    async fn kill(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> Result<ProcessRecord, ProcessError> {
        let record = self.inner.kill(scope, process_id).await?;
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
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
    secret_store: Option<Arc<dyn SecretStore>>,
    secret_injections: Option<Arc<RuntimeSecretInjectionStore>>,
    resource_governor: Option<Arc<dyn ResourceGovernor>>,
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

    pub fn with_network_policy_store(mut self, store: Arc<NetworkObligationPolicyStore>) -> Self {
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

    pub fn with_secret_injection_store(mut self, store: Arc<RuntimeSecretInjectionStore>) -> Self {
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
            let exists = secret_store
                .metadata(&request.context.resource_scope, handle)
                .await
                .map_err(|_| secret_obligation_failed())?
                .is_some();
            if !exists {
                return Err(secret_obligation_failed());
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
        let secret_handles = secret_injection_obligations(request.obligations);
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
            dispatch.output = redact_output(dispatch.output)?;
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
            for handle in secret_injection_obligations(obligations) {
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

fn secret_injection_obligations(obligations: &[Obligation]) -> Vec<SecretHandle> {
    obligations
        .iter()
        .filter_map(|obligation| match obligation {
            Obligation::InjectSecretOnce { handle } => Some(handle.clone()),
            _ => None,
        })
        .collect()
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
        Obligation::EnforceOutputLimit { .. } => Some("enforce_output_limit"),
        Obligation::ReserveResources { .. } => Some("reserve_resources"),
        Obligation::UseScopedMounts { .. } => Some("use_scoped_mounts"),
        Obligation::EnforceResourceCeiling { .. } => Some("enforce_resource_ceiling"),
    }
}
