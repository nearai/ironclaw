//! GitHub capability dispatch abstraction for the issue workflow.
//!
//! [`GithubIssueWorkflowCapabilityDispatcher`] is the seam the workflow port
//! invokes GitHub capabilities through;
//! [`HostRuntimeGithubIssueWorkflowCapabilityDispatcher`] is the production
//! implementation that routes each request through the host runtime with a
//! fresh execution context and the workflow's credential account selection.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_github_issue_workflow::GithubProviderAccountRef;
use ironclaw_host_api::{
    CapabilityId, CorrelationId, ExecutionContext, InvocationId, ResourceEstimate,
    RuntimeCredentialAccountId, RuntimeCredentialAccountProviderId,
    RuntimeCredentialAccountSelection,
};
use ironclaw_host_runtime::{RuntimeCapabilityOutcome, RuntimeCapabilityRequest};
use ironclaw_trust::TrustDecision;
use serde_json::Value as JsonValue;

pub(crate) struct GithubIssueWorkflowCapabilityDispatchRequest {
    pub(crate) capability_id: String,
    pub(crate) provider_account_ref: GithubProviderAccountRef,
    pub(crate) input: JsonValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GithubIssueWorkflowCapabilityDispatchError {
    AuthRequired,
    ApprovalRequired,
    Backend { kind: String, message: String },
}

#[async_trait]
pub(crate) trait GithubIssueWorkflowCapabilityDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        request: GithubIssueWorkflowCapabilityDispatchRequest,
    ) -> Result<JsonValue, GithubIssueWorkflowCapabilityDispatchError>;
}

pub(crate) struct HostRuntimeGithubIssueWorkflowCapabilityDispatcher {
    host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    execution_context: ExecutionContext,
    trust_decision: TrustDecision,
}

impl HostRuntimeGithubIssueWorkflowCapabilityDispatcher {
    pub(crate) fn new(
        host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
        execution_context: ExecutionContext,
        trust_decision: TrustDecision,
    ) -> Self {
        Self {
            host_runtime,
            execution_context,
            trust_decision,
        }
    }

    fn fresh_execution_context(
        &self,
    ) -> Result<ExecutionContext, GithubIssueWorkflowCapabilityDispatchError> {
        let mut context = self.execution_context.clone();
        let invocation_id = InvocationId::new();
        context.invocation_id = invocation_id;
        context.correlation_id = CorrelationId::new();
        context.resource_scope.invocation_id = invocation_id;
        context.validate().map_err(|error| {
            GithubIssueWorkflowCapabilityDispatchError::Backend {
                kind: "invalid_execution_context".to_string(),
                message: error.to_string(),
            }
        })?;
        Ok(context)
    }
}

#[async_trait]
impl GithubIssueWorkflowCapabilityDispatcher
    for HostRuntimeGithubIssueWorkflowCapabilityDispatcher
{
    async fn dispatch(
        &self,
        request: GithubIssueWorkflowCapabilityDispatchRequest,
    ) -> Result<JsonValue, GithubIssueWorkflowCapabilityDispatchError> {
        let capability_id = CapabilityId::new(request.capability_id.clone()).map_err(|error| {
            GithubIssueWorkflowCapabilityDispatchError::Backend {
                kind: "invalid_capability_id".to_string(),
                message: error.to_string(),
            }
        })?;
        let provider =
            RuntimeCredentialAccountProviderId::new(request.provider_account_ref.provider.clone())
                .map_err(
                    |error| GithubIssueWorkflowCapabilityDispatchError::Backend {
                        kind: "invalid_provider_account_ref".to_string(),
                        message: error.to_string(),
                    },
                )?;
        let account_id =
            RuntimeCredentialAccountId::new(request.provider_account_ref.account_id.clone())
                .map_err(
                    |error| GithubIssueWorkflowCapabilityDispatchError::Backend {
                        kind: "invalid_provider_account_ref".to_string(),
                        message: error.to_string(),
                    },
                )?;
        let runtime_request = RuntimeCapabilityRequest::new(
            self.fresh_execution_context()?,
            capability_id.clone(),
            ResourceEstimate::default(),
            request.input,
            self.trust_decision.clone(),
        )
        .with_credential_account_selection(RuntimeCredentialAccountSelection::new(
            provider, account_id,
        ));

        match self.host_runtime.invoke_capability(runtime_request).await {
            Ok(RuntimeCapabilityOutcome::Completed(completed)) => Ok(completed.output),
            Ok(RuntimeCapabilityOutcome::AuthRequired(_)) => {
                Err(GithubIssueWorkflowCapabilityDispatchError::AuthRequired)
            }
            Ok(RuntimeCapabilityOutcome::ApprovalRequired(_)) => {
                Err(GithubIssueWorkflowCapabilityDispatchError::ApprovalRequired)
            }
            Ok(RuntimeCapabilityOutcome::Failed(failure)) => {
                Err(GithubIssueWorkflowCapabilityDispatchError::Backend {
                    kind: failure.kind.as_str().to_string(),
                    message: failure.message.unwrap_or_else(|| {
                        format!("GitHub capability {} failed", capability_id.as_str())
                    }),
                })
            }
            Ok(other) => Err(GithubIssueWorkflowCapabilityDispatchError::Backend {
                kind: other.kind().to_string(),
                message: format!(
                    "GitHub capability {} returned unsupported runtime outcome {}",
                    capability_id.as_str(),
                    other.kind()
                ),
            }),
            Err(error) => Err(GithubIssueWorkflowCapabilityDispatchError::Backend {
                kind: "host_runtime_error".to_string(),
                message: error.to_string(),
            }),
        }
    }
}
