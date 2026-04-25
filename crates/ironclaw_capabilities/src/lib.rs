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
use ironclaw_run_state::{RunStart, RunStateError, RunStateStore};
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
    #[error("run-state update failed: {0}")]
    RunState(Box<RunStateError>),
    #[error("dispatch failed: {0}")]
    Dispatch(Box<DispatchError>),
}

impl From<RunStateError> for CapabilityInvocationError {
    fn from(error: RunStateError) -> Self {
        Self::RunState(Box::new(error))
    }
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
    run_state: Option<&'a dyn RunStateStore>,
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
            run_state: None,
        }
    }

    pub fn with_run_state(mut self, run_state: &'a dyn RunStateStore) -> Self {
        self.run_state = Some(run_state);
        self
    }

    pub async fn invoke_json(
        &self,
        request: CapabilityInvocationRequest,
    ) -> Result<CapabilityInvocationResult, CapabilityInvocationError> {
        let invocation_id = request.context.invocation_id;
        let capability_id = request.capability_id.clone();
        if let Some(run_state) = self.run_state {
            run_state.start(RunStart {
                invocation_id,
                capability_id,
                scope: request.context.resource_scope.clone(),
            });
        }

        let descriptor = self
            .registry
            .get_capability(&request.capability_id)
            .ok_or_else(|| {
                if let Some(run_state) = self.run_state {
                    let _ = run_state.fail(invocation_id, "UnknownCapability".to_string());
                }
                CapabilityInvocationError::UnknownCapability {
                    capability: request.capability_id.clone(),
                }
            })?;

        match self
            .authorizer
            .authorize_dispatch(&request.context, descriptor, &request.estimate)
        {
            Decision::Allow { .. } => {}
            Decision::Deny { reason } => {
                if let Some(run_state) = self.run_state {
                    run_state.fail(invocation_id, "AuthorizationDenied".to_string())?;
                }
                return Err(CapabilityInvocationError::AuthorizationDenied {
                    capability: request.capability_id,
                    reason,
                });
            }
            Decision::RequireApproval { request: approval } => {
                if let Some(run_state) = self.run_state {
                    run_state.block_approval(invocation_id, approval)?;
                }
                return Err(CapabilityInvocationError::AuthorizationRequiresApproval {
                    capability: request.capability_id,
                });
            }
        }

        let dispatch = match self
            .dispatcher
            .dispatch_json(CapabilityDispatchRequest {
                capability_id: request.capability_id,
                scope: request.context.resource_scope,
                estimate: request.estimate,
                input: request.input,
            })
            .await
        {
            Ok(dispatch) => dispatch,
            Err(error) => {
                if let Some(run_state) = self.run_state {
                    run_state.fail(invocation_id, "Dispatch".to_string())?;
                }
                return Err(CapabilityInvocationError::from(error));
            }
        };

        if let Some(run_state) = self.run_state {
            run_state.complete(invocation_id)?;
        }

        Ok(CapabilityInvocationResult { dispatch })
    }
}
