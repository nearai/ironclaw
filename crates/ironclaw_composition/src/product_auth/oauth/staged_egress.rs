//! Obligation-staging decorator around the runtime HTTP egress for the auth
//! engine. The engine builds requests carrying the vendor network policy
//! (host allowlist + body cap); this wrapper stages that policy with the
//! capability obligation handler before the transport call and discards it
//! afterwards, so the staged-policy store never leaks one entry per exchange.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::AuthProductError;
use ironclaw_capabilities::{
    CapabilityObligationAbortRequest, CapabilityObligationHandler, CapabilityObligationOutcome,
    CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_host_api::{
    CapabilitySet, CorrelationId, ExtensionId, MountView, NetworkPolicy, Obligation,
    ResourceEstimate, ResourceScope, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind, TrustClass,
};

/// Wraps the production egress so every engine vendor call runs with its
/// request-carried network policy staged as an invoke obligation.
pub(crate) struct ObligationStagedAuthEgress {
    inner: Arc<dyn RuntimeHttpEgress>,
    obligations: Arc<dyn CapabilityObligationHandler>,
}

impl ObligationStagedAuthEgress {
    pub(crate) fn new(
        inner: Arc<dyn RuntimeHttpEgress>,
        obligations: Arc<dyn CapabilityObligationHandler>,
    ) -> Self {
        Self { inner, obligations }
    }

    async fn stage(
        &self,
        request: &RuntimeHttpEgressRequest,
    ) -> Result<(), RuntimeHttpEgressError> {
        authorize_auth_egress(
            Arc::clone(&self.obligations),
            &request.scope,
            &request.capability_id,
            &request.network_policy,
        )
        .await
        .map_err(|_| RuntimeHttpEgressError::Request {
            reason: "auth egress network policy could not be staged".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        })
    }

    async fn discard(&self, request: &RuntimeHttpEgressRequest) {
        discard_auth_egress_policy(
            Arc::clone(&self.obligations),
            &request.scope,
            &request.capability_id,
            &request.network_policy,
        )
        .await;
    }
}

impl fmt::Debug for ObligationStagedAuthEgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObligationStagedAuthEgress")
            .finish()
    }
}

#[async_trait]
impl RuntimeHttpEgress for ObligationStagedAuthEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.stage(&request).await?;
        let result = self.inner.execute(request.clone()).await;
        self.discard(&request).await;
        result
    }

    async fn execute_credential_exchange(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        self.stage(&request).await?;
        let result = self
            .inner
            .execute_credential_exchange(request.clone())
            .await;
        // Success or failure, the staged policy must not outlive the call
        // (the pipeline's own discard only covers pre-transport failures).
        self.discard(&request).await;
        result
    }
}

async fn authorize_auth_egress(
    handler: Arc<dyn CapabilityObligationHandler>,
    scope: &ResourceScope,
    capability_id: &ironclaw_host_api::CapabilityId,
    policy: &NetworkPolicy,
) -> Result<(), AuthProductError> {
    let context = auth_execution_context(scope.clone())?;
    let estimate = ResourceEstimate {
        network_egress_bytes: policy.max_egress_bytes,
        ..ResourceEstimate::default()
    };
    handler
        .satisfy(CapabilityObligationRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &estimate,
            obligations: &[Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            }],
        })
        .await
        .map_err(|_| AuthProductError::BackendUnavailable)
}

/// Best-effort: a discard failure is logged, never surfaced — the vendor-call
/// outcome must not change because cleanup hiccuped.
async fn discard_auth_egress_policy(
    handler: Arc<dyn CapabilityObligationHandler>,
    scope: &ResourceScope,
    capability_id: &ironclaw_host_api::CapabilityId,
    policy: &NetworkPolicy,
) {
    let context = match auth_execution_context(scope.clone()) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(
                target: "ironclaw::oauth",
                ?error,
                "skipped auth egress-policy discard: execution context unavailable"
            );
            return;
        }
    };
    let estimate = ResourceEstimate {
        network_egress_bytes: policy.max_egress_bytes,
        ..ResourceEstimate::default()
    };
    if let Err(error) = handler
        .abort(CapabilityObligationAbortRequest {
            phase: CapabilityObligationPhase::Invoke,
            context: &context,
            capability_id,
            estimate: &estimate,
            obligations: &[Obligation::ApplyNetworkPolicy {
                policy: policy.clone(),
            }],
            outcome: &CapabilityObligationOutcome::default(),
        })
        .await
    {
        tracing::warn!(
            obligation_error = ?error,
            "failed to discard staged auth egress policy after vendor call"
        );
    }
}

fn auth_execution_context(
    resource_scope: ResourceScope,
) -> Result<ironclaw_host_api::ExecutionContext, AuthProductError> {
    let context = ironclaw_host_api::ExecutionContext {
        run_id: None,
        invocation_id: resource_scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        // Internal auth-engine egress context: no human actor is present.
        authenticated_actor_user_id: None,
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
        origin: None,
        extension_id: ExtensionId::new("ironclaw_auth")
            .map_err(|_| AuthProductError::BackendUnavailable)?,
        runtime: RuntimeKind::System,
        trust: TrustClass::System,
        grants: CapabilitySet::default(),
        mounts: MountView::default(),
        resource_scope,
    };
    context
        .validate()
        .map_err(|_| AuthProductError::BackendUnavailable)?;
    Ok(context)
}
