//! Obligation handling — one of the three chartered owners of the obligation
//! module (PROPOSAL §6.5.9, CHECKLIST WS3).
//!
//! This module owns the decision half: which obligations this host runtime
//! supports, what each one does before and after dispatch, and the audit,
//! redaction, resource-ceiling and mount validation that back them. It reads
//! and writes the staging stores in [`super::staged_handoffs`] but does not own
//! them, and it hands post-start cleanup to [`super::process_store`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_capabilities::{
    CapabilityObligationAbortRequest, CapabilityObligationCompletionRequest,
    CapabilityObligationError, CapabilityObligationFailureKind, CapabilityObligationHandler,
    CapabilityObligationOutcome, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_event_log::{
    AuditSink, SecurityAuditEvent, SecurityAuditSink, SecurityBoundary, SecurityDecision,
};
use ironclaw_host_api::{
    action::NetworkPolicy,
    audit::{ActionResultSummary, ActionSummary, AuditEnvelope, AuditStage, DecisionSummary},
    capability::{EffectKind, RuntimeCredentialAccountSetup},
    decision::{Obligation, RuntimeCredentialAuthRequirement},
    dispatch::{CapabilityDispatchResult, CredentialStageError},
    ids::{AuditEventId, CapabilityId, ExtensionId, SecretHandle, VendorId},
    mount::MountView,
    resource::{
        ResourceCeiling, ResourceEstimate, ResourceReservation, ResourceScope, ResourceUsage,
        SandboxQuota,
    },
};
use ironclaw_resources::ResourceGovernor;
use ironclaw_safety::LeakDetector;
use ironclaw_secrets::{SecretStoreError, SecretStorePort};

use super::staged_handoffs::{
    NetworkObligationPolicyStore, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, RuntimeSecretInjectionStore,
};

/// Built-in obligation handler for the current host-runtime slice.
#[derive(Clone, Default)]
pub struct BuiltinObligationHandler {
    audit_sink: Option<Arc<dyn AuditSink>>,
    security_audit_sink: Option<Arc<dyn SecurityAuditSink>>,
    network_policies: Option<Arc<NetworkObligationPolicyStore>>,
    secret_store: Option<Arc<dyn SecretStorePort>>,
    secret_injections: Option<Arc<RuntimeSecretInjectionStore>>,
    resource_governor: Option<Arc<dyn ResourceGovernor>>,
    credential_account_resolver: Option<Arc<dyn RuntimeCredentialAccountResolver>>,
}

struct ResolvedSecretInjection {
    handle: SecretHandle,
    source_scope: ResourceScope,
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
        T: SecretStorePort + 'static,
    {
        let store: Arc<dyn SecretStorePort> = store;
        self.secret_store = Some(store);
        self
    }

    pub fn with_secret_store_dyn(mut self, store: Arc<dyn SecretStorePort>) -> Self {
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
    ) -> Result<Vec<ResolvedSecretInjection>, CapabilityObligationError> {
        if handles.is_empty() {
            return Ok(Vec::new());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        if self.secret_injections.is_none() {
            return Err(secret_obligation_failed());
        }
        let mut resolved = Vec::with_capacity(handles.len());
        for handle in handles {
            // Fail closed on a store error: the dispatch-time backstop must never
            // let an uncredentialed call through on a transient failure. Preserve the
            // cause as a server-side trail (`SecretStoreError` Display carries no raw
            // secret material — handles/reasons only); the caller still receives the
            // opaque, sanitized secret-obligation failure.
            let owner = match secret_owner_scope(
                secret_store.as_ref(),
                &request.context.resource_scope,
                handle,
            )
            .await
            {
                Ok(owner) => owner,
                Err(error) => {
                    tracing::debug!(
                        secret_handle = handle.as_str(),
                        error = %error,
                        "dispatch-time secret presence probe failed; failing closed at the obligation backstop"
                    );
                    return Err(secret_obligation_failed());
                }
            };
            let Some(source_scope) = owner else {
                return Err(CapabilityObligationError::AuthRequired {
                    credential_requirements: Vec::new(),
                });
            };
            resolved.push(ResolvedSecretInjection {
                handle: handle.clone(),
                source_scope,
            });
        }
        Ok(resolved)
    }

    async fn inject_secrets(
        &self,
        request: &CapabilityObligationRequest<'_>,
        resolved: &[ResolvedSecretInjection],
    ) -> Result<(), CapabilityObligationError> {
        if resolved.is_empty() {
            return Ok(());
        }
        let Some(secret_store) = &self.secret_store else {
            return Err(secret_obligation_failed());
        };
        let Some(secret_injections) = &self.secret_injections else {
            return Err(secret_obligation_failed());
        };

        let mut material = Vec::with_capacity(resolved.len());
        for resolved in resolved {
            // Use the same source scope the presence probe accepted: the caller's
            // own secret if present, else the tenant-shared admin-managed secret
            // (#5459). The injection target below stays the caller's invocation
            // slot regardless of where the source material came from. The
            // lease/consume operations remain authoritative if the secret
            // vanishes between preflight and here.
            // Every arm below fails closed; the bound error is logged first so a
            // shared-secret lease/consume fault (e.g. an AAD/scope regression on
            // the cross-scope read) leaves a server-side trail. `SecretStoreError`
            // Display carries no raw secret material — handles/reasons only.
            let lease = secret_store
                .lease_once(&resolved.source_scope, &resolved.handle)
                .await
                .map_err(|error| {
                    tracing::debug!(
                        secret_handle = resolved.handle.as_str(),
                        error = %error,
                        "secret injection: lease failed; failing closed"
                    );
                    secret_obligation_failed()
                })?;
            let secret = secret_store
                .consume(&resolved.source_scope, lease.id)
                .await
                .map_err(|error| {
                    tracing::debug!(
                        secret_handle = resolved.handle.as_str(),
                        error = %error,
                        "secret injection: lease consume failed; failing closed"
                    );
                    secret_obligation_failed()
                })?;
            material.push((resolved.handle.clone(), secret));
        }

        for (handle, secret) in material {
            secret_injections
                .insert(
                    &request.context.resource_scope,
                    request.capability_id,
                    &handle,
                    secret,
                )
                .map_err(|error| {
                    tracing::debug!(
                        secret_handle = handle.as_str(),
                        error = %error,
                        "secret injection: injection-slot insert failed; failing closed"
                    );
                    secret_obligation_failed()
                })?;
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
        resolved_secret_injections: &[ResolvedSecretInjection],
        network_policy: Option<NetworkPolicy>,
    ) -> Result<(), CapabilityObligationError> {
        if request
            .obligations
            .iter()
            .any(|obligation| matches!(obligation, Obligation::AuditBefore))
        {
            self.emit_audit_before(request).await?;
        }

        self.inject_secrets(request, resolved_secret_injections)
            .await?;
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
            if let Err(cleanup_error) = self.discard_staged_handoffs(
                &request.context.resource_scope,
                request.capability_id,
                request.obligations,
            ) {
                tracing::debug!(error = ?cleanup_error, "best-effort discard of staged handoffs failed");
            }
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
        let resolved_secret_injections = self
            .preflight_secret_injection(&request, &secret_handles)
            .await?;
        self.preflight_resource_ceiling(&request)?;
        let resource_reservation = self.reserve_resource_obligation(&request)?;
        let outcome = CapabilityObligationOutcome {
            mounts: scoped_mounts,
            resource_reservation,
        };

        if let Err(error) = self
            .finish_prepare(&request, &resolved_secret_injections, network_policy)
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
        // Turn any base64 document payload into extracted text before redaction
        // and the output-size obligations run, so the model gets bounded text
        // (leak-scanned, size-checked) and the large base64 never survives.
        dispatch.output = crate::document_output::extract_documents_in_output(
            dispatch.capability_id.as_str(),
            dispatch.output,
        );
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
        .filter(|obligation| !obligation_supported(phase, obligation))
        .cloned()
        .collect()
}

fn unsupported_completion_obligations(
    phase: CapabilityObligationPhase,
    obligations: &[Obligation],
) -> Vec<Obligation> {
    obligations
        .iter()
        .filter(|obligation| !obligation_supported(phase, obligation))
        .cloned()
        .collect()
}

/// Whether the host can honour `obligation` at `phase`.
///
/// One predicate, deliberately. This was two — `obligation_supported_before_dispatch`
/// and `obligation_supported_after_dispatch` — with byte-identical bodies: every arm
/// and every phase condition matched. The names asserted a pre-dispatch/post-dispatch
/// distinction that the code never implemented, and the pair gates *admission* of
/// `RedactOutput`, `EnforceOutputLimit` and `EnforceResourceCeiling`. A later edit to
/// one copy would have left the other stage silently accepting an obligation it cannot
/// honour, which is a fail-open. Collapsed rather than resynchronised: if a real
/// pre/post difference is ever needed, reintroduce it as an explicit parameter here so
/// the distinction lives in one place instead of in two bodies that must be kept equal
/// by hand.
fn obligation_supported(phase: CapabilityObligationPhase, obligation: &Obligation) -> bool {
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
    provider: &'a VendorId,
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
    secret_store: &dyn SecretStorePort,
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
    context: &ironclaw_host_api::scope::ExecutionContext,
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
async fn secret_present(
    store: &dyn SecretStorePort,
    scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<bool, SecretStoreError> {
    Ok(store.metadata(scope, handle).await?.is_some())
}

/// Resolve which scope owns `handle` for this caller, honoring tenant-shared,
/// admin-managed credentials (#5459). A caller's OWN secret wins; otherwise the
/// tenant-shared admin-managed scope ([`ResourceScope::tenant_shared_managed_scope`]),
/// so one admin-set key satisfies every user of the tenant. `Ok(None)` means the
/// secret is absent in both scopes.
///
/// Single source of truth for BOTH "is this required secret present" (callers map
/// to `.is_some()`) and "where does the lease read from" — the pre-flight ordering
/// probe (`credential_preflight_check`) and the dispatch-time backstop
/// (`preflight_secret_injection`) consult this rule, then the injection lease
/// (`inject_secrets`) consumes the resolved source scope. Each caller decides how
/// to treat a store `Err` (the pre-flight fails open and skips; the obligation
/// backstop and lease fail closed).
pub(crate) async fn secret_owner_scope(
    store: &dyn SecretStorePort,
    caller_scope: &ResourceScope,
    handle: &SecretHandle,
) -> Result<Option<ResourceScope>, SecretStoreError> {
    if secret_present(store, caller_scope, handle).await? {
        return Ok(Some(caller_scope.clone()));
    }
    let shared = caller_scope.tenant_shared_managed_scope();
    if secret_present(store, &shared, handle).await? {
        return Ok(Some(shared));
    }
    Ok(None)
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
