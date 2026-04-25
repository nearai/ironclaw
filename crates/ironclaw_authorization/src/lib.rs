//! Capability authorization contracts for IronClaw Reborn.
//!
//! `ironclaw_authorization` evaluates authority-bearing host API contracts. It
//! does not execute capabilities, reserve resources, prompt users, or reach into
//! runtime internals. The first slice implements a grant-backed gate for
//! capability dispatch.

use ironclaw_host_api::{
    CapabilityDescriptor, Decision, DenyReason, EffectKind, ExecutionContext, Principal,
    ResourceEstimate,
};

/// Authorizes a capability dispatch request against an execution context.
pub trait CapabilityDispatchAuthorizer: Send + Sync {
    fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
    ) -> Decision;
}

/// Grant-backed capability dispatch authorizer.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrantAuthorizer;

impl GrantAuthorizer {
    pub fn new() -> Self {
        Self
    }
}

impl CapabilityDispatchAuthorizer for GrantAuthorizer {
    fn authorize_dispatch(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
    ) -> Decision {
        if context.validate().is_err() {
            return Decision::Deny {
                reason: DenyReason::InternalInvariantViolation,
            };
        }

        let Some(grant) = context.grants.grants.iter().find(|grant| {
            grant.capability == descriptor.id && principal_matches_context(&grant.grantee, context)
        }) else {
            return Decision::Deny {
                reason: DenyReason::MissingGrant,
            };
        };

        if !effects_are_covered(&descriptor.effects, &grant.constraints.allowed_effects) {
            return Decision::Deny {
                reason: DenyReason::PolicyDenied,
            };
        }

        Decision::Allow {
            obligations: Vec::new(),
        }
    }
}

fn principal_matches_context(principal: &Principal, context: &ExecutionContext) -> bool {
    match principal {
        Principal::Tenant(id) => id == &context.tenant_id,
        Principal::User(id) => id == &context.user_id,
        Principal::Project(id) => context.project_id.as_ref() == Some(id),
        Principal::Mission(id) => context.mission_id.as_ref() == Some(id),
        Principal::Thread(id) => context.thread_id.as_ref() == Some(id),
        Principal::Extension(id) => id == &context.extension_id,
        Principal::System => false,
    }
}

fn effects_are_covered(required: &[EffectKind], allowed: &[EffectKind]) -> bool {
    required.iter().all(|effect| allowed.contains(effect))
}
