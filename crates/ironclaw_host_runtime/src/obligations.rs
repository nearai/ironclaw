use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
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
    Obligation, ResourceReservation, ResourceScope, SecretHandle,
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::{SecretMaterial, SecretStore};

/// One-shot runtime secret material staged after `InjectSecretOnce` lease consumption.
///
/// The store is keyed by scoped invocation, capability, and handle. Runtime adapters
/// must use `take(...)` so staged material is removed before it can be reused.
#[derive(Clone, Default)]
pub struct RuntimeSecretInjectionStore {
    secrets: Arc<Mutex<HashMap<RuntimeSecretInjectionKey, SecretMaterial>>>,
}

impl RuntimeSecretInjectionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
        material: SecretMaterial,
    ) -> Result<(), RuntimeSecretInjectionStoreError> {
        self.lock()?.insert(
            RuntimeSecretInjectionKey::new(scope, capability_id, handle),
            material,
        );
        Ok(())
    }

    pub fn take(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMaterial>, RuntimeSecretInjectionStoreError> {
        Ok(self.lock()?.remove(&RuntimeSecretInjectionKey::new(
            scope,
            capability_id,
            handle,
        )))
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, HashMap<RuntimeSecretInjectionKey, SecretMaterial>>,
        RuntimeSecretInjectionStoreError,
    > {
        self.secrets
            .lock()
            .map_err(|_| RuntimeSecretInjectionStoreError::Unavailable)
    }
}

impl fmt::Debug for RuntimeSecretInjectionStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeSecretInjectionStore")
            .field("secrets", &"[REDACTED]")
            .finish()
    }
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
        Obligation::EnforceResourceCeiling { .. } => false,
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
        Obligation::EnforceResourceCeiling { .. } => false,
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
