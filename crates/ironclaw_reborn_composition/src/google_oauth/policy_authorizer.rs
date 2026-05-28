use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::AuthProductError;
use ironclaw_capabilities::{
    CapabilityObligationHandler, CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_host_api::{
    CapabilityId, CapabilitySet, CorrelationId, ExtensionId, MountView, NetworkPolicy,
    NetworkScheme, NetworkTargetPattern, Obligation, ResourceEstimate, ResourceScope, RuntimeKind,
    TrustClass,
};

/// Boundary for staging/authorizing the Google token-exchange network policy.
#[async_trait]
pub(super) trait GoogleProviderEgressPolicyAuthorizer: Send + Sync {
    async fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), AuthProductError>;
}

pub(super) struct ObligationGoogleEgressPolicyAuthorizer {
    pub(super) handler: Arc<dyn CapabilityObligationHandler>,
}

#[async_trait]
impl GoogleProviderEgressPolicyAuthorizer for ObligationGoogleEgressPolicyAuthorizer {
    async fn authorize_google_token_exchange(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
        policy: &NetworkPolicy,
    ) -> Result<(), AuthProductError> {
        let context = google_oauth_execution_context(scope.clone())?;
        let estimate = ResourceEstimate {
            network_egress_bytes: policy.max_egress_bytes,
            ..ResourceEstimate::default()
        };
        self.handler
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
}

fn google_oauth_execution_context(
    resource_scope: ResourceScope,
) -> Result<ironclaw_host_api::ExecutionContext, AuthProductError> {
    let context = ironclaw_host_api::ExecutionContext {
        invocation_id: resource_scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: resource_scope.tenant_id.clone(),
        user_id: resource_scope.user_id.clone(),
        agent_id: resource_scope.agent_id.clone(),
        project_id: resource_scope.project_id.clone(),
        mission_id: resource_scope.mission_id.clone(),
        thread_id: resource_scope.thread_id.clone(),
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

pub(super) fn google_token_network_policy(response_body_limit: u64) -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "oauth2.googleapis.com".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(response_body_limit),
    }
}
