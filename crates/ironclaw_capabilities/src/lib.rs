//! Capability invocation host contracts for IronClaw Reborn.
//!
//! `ironclaw_capabilities` is the caller-facing capability invocation service.
//! It coordinates authorization and runtime dispatch without making callers
//! understand grant evaluation and without making the dispatcher own auth.

use ironclaw_authorization::CapabilityDispatchAuthorizer;
use ironclaw_dispatcher::{
    CapabilityDispatchRequest, CapabilityDispatchResult, DispatchError, RuntimeDispatcher,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CapabilityId, Decision, DenyReason, ExecutionContext, ResourceEstimate};
use ironclaw_resources::ResourceGovernor;
use serde_json::Value;
use thiserror::Error;

/// Caller-facing capability invocation request.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityInvocationRequest {
    pub context: ExecutionContext,
    pub capability_id: CapabilityId,
    pub estimate: ResourceEstimate,
    pub input: Value,
}

/// Caller-facing capability invocation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityInvocationResult {
    pub dispatch: CapabilityDispatchResult,
}

/// Capability invocation failures before or during dispatch.
#[derive(Debug, Error)]
pub enum CapabilityInvocationError {
    #[error("unknown capability {capability}")]
    UnknownCapability { capability: CapabilityId },
    #[error("capability {capability} invocation denied: {reason:?}")]
    AuthorizationDenied {
        capability: CapabilityId,
        reason: DenyReason,
    },
    #[error("capability {capability} invocation requires approval")]
    AuthorizationRequiresApproval { capability: CapabilityId },
    #[error("dispatch failed: {0}")]
    Dispatch(Box<DispatchError>),
}

impl From<DispatchError> for CapabilityInvocationError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(Box::new(error))
    }
}

/// Host-facing capability invocation service.
pub struct CapabilityHost<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    registry: &'a ExtensionRegistry,
    dispatcher: &'a RuntimeDispatcher<'a, F, G>,
    authorizer: &'a dyn CapabilityDispatchAuthorizer,
}

impl<'a, F, G> CapabilityHost<'a, F, G>
where
    F: RootFilesystem,
    G: ResourceGovernor,
{
    pub fn new(
        registry: &'a ExtensionRegistry,
        dispatcher: &'a RuntimeDispatcher<'a, F, G>,
        authorizer: &'a dyn CapabilityDispatchAuthorizer,
    ) -> Self {
        Self {
            registry,
            dispatcher,
            authorizer,
        }
    }

    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let descriptor = self
            .registry
            .get_capability(&request.capability_id)
            .ok_or_else(|| CapabilityInvocationError::UnknownCapability {
                capability: request.capability_id.clone(),
            })?;

        match self
            .authorizer
            .authorize_dispatch(&request.context, descriptor, &request.estimate)
        {
            Decision::Allow { .. } => {}
            Decision::Deny { reason } => {
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { .. } => {
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let dispatch = self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: request.context.resource_scope,
                estimate: request.estimate,
                input: request.input,
            })
            .await?;

        Ok(CapabilityInvocationResult { dispatch })
    }
}
