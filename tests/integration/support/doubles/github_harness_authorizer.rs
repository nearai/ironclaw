use super::super::github as github_support;
use async_trait::async_trait;
use ironclaw_authorization::TrustAwareCapabilityDispatchAuthorizer;
use ironclaw_host_api::{
    CapabilityDescriptor, Decision, ExecutionContext, ExtensionId, Obligation, Obligations,
    ResourceEstimate, SecretHandle, VendorId,
};
use ironclaw_trust::TrustDecision;

use super::super::harness::HarnessResult;

pub(crate) struct GithubHarnessAuthorizer {
    obligations: Obligations,
}

impl GithubHarnessAuthorizer {
    pub(crate) fn new() -> HarnessResult<Self> {
        Ok(Self {
            obligations: Obligations::new(vec![
                Obligation::ApplyNetworkPolicy {
                    policy: github_support::api_policy(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: SecretHandle::new("github_runtime_token")?,
                    provider: VendorId::new("github")?,
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                    provider_scopes: Vec::new(),
                    requester_extension: ExtensionId::new("github")?,
                },
            ])?,
        })
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for GithubHarnessAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }
}
