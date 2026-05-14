use std::sync::Arc;

use ironclaw_extensions::{HostApiId, ManifestSectionPath};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, NetworkMethod, ResourceScope, RuntimeCredentialTarget, SecretHandle,
};
use ironclaw_secrets::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialAccountStore,
    CredentialBrokerError,
};
use ironclaw_wasm::WasmStagedRuntimeCredential;
use thiserror::Error;

/// Projected credential declaration owned by a host API manifest contract.
///
/// `ironclaw_extensions` validates only the generic manifest envelope. Domain
/// host API contracts project their typed credential sections into this cold
/// read model before runtime planning reaches host-runtime composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostApiCredentialRequirement {
    pub extension_id: ExtensionId,
    pub host_api_id: HostApiId,
    pub section_path: ManifestSectionPath,
    pub capability_id: CapabilityId,
    pub account_id: CredentialAccountId,
    pub handle: SecretHandle,
    pub target: RuntimeCredentialTarget,
    pub required: bool,
    pub exact_url: Option<String>,
}

impl HostApiCredentialRequirement {
    fn matches_request(&self, request: &CredentialAccountResolverRequest) -> bool {
        self.extension_id == request.extension_id
            && self.host_api_id == request.host_api_id
            && self.section_path == request.section_path
            && self.capability_id == request.capability_id
            && self
                .exact_url
                .as_ref()
                .is_none_or(|exact_url| exact_url == &request.url)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountResolverRequest {
    pub scope: ResourceScope,
    pub extension_id: ExtensionId,
    pub host_api_id: HostApiId,
    pub section_path: ManifestSectionPath,
    pub capability_id: CapabilityId,
    pub method: NetworkMethod,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountResolution {
    /// Secret handles that must be satisfied through `InjectSecretOnce` before
    /// the returned WASM staged credential rules can be used.
    pub required_secret_handles: Vec<SecretHandle>,
    /// Request-scoped WASM credential rules. Each rule is exact-URL scoped to
    /// the resolver request so resolved credentials cannot silently bleed into
    /// another host-mediated HTTP request in the same invocation.
    pub wasm_credentials: Vec<WasmStagedRuntimeCredential>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CredentialAccountResolverError {
    #[error("credential account resolution failed: {0}")]
    Broker(#[from] CredentialBrokerError),
    #[error("credential account {account_id} does not contain required handle {handle}")]
    MissingSecretHandle {
        account_id: CredentialAccountId,
        handle: SecretHandle,
    },
}

impl CredentialAccountResolverError {
    pub fn stable_reason(&self) -> &'static str {
        match self {
            Self::Broker(error) => error.stable_reason(),
            Self::MissingSecretHandle { .. } => "CredentialPolicyMismatch",
        }
    }
}

/// Resolves host API credential requirements into request-scoped WASM rules.
///
/// The resolver does not read raw secret material and does not grant authority
/// by itself. It proves that a projected host API credential requirement matches
/// an active scoped credential account and destination policy, then returns the
/// secret handles and exact-URL staged credential rules that later obligation
/// handling/runtime composition can consume.
pub struct CredentialAccountResolver<S>
where
    S: CredentialAccountStore + ?Sized,
{
    account_store: Arc<S>,
    requirements: Vec<HostApiCredentialRequirement>,
}

impl<S> CredentialAccountResolver<S>
where
    S: CredentialAccountStore + ?Sized,
{
    pub fn new(
        account_store: Arc<S>,
        requirements: impl IntoIterator<Item = HostApiCredentialRequirement>,
    ) -> Self {
        Self {
            account_store,
            requirements: requirements.into_iter().collect(),
        }
    }

    pub fn requirements(&self) -> &[HostApiCredentialRequirement] {
        &self.requirements
    }

    pub async fn resolve_for_wasm(
        &self,
        request: &CredentialAccountResolverRequest,
    ) -> Result<CredentialAccountResolution, CredentialAccountResolverError> {
        let mut required_secret_handles = Vec::new();
        let mut wasm_credentials = Vec::new();

        for requirement in self
            .requirements
            .iter()
            .filter(|requirement| requirement.matches_request(request))
        {
            let Some(account) = self
                .account_store
                .get_account(&request.scope, &requirement.account_id)
                .await?
            else {
                if requirement.required {
                    return Err(CredentialBrokerError::MissingCredential {
                        account_id: requirement.account_id.clone(),
                    }
                    .into());
                }
                continue;
            };

            if !account_matches_request(&account, request) {
                return Err(CredentialBrokerError::CredentialExtensionMismatch {
                    account_id: requirement.account_id.clone(),
                }
                .into());
            }
            match account.status {
                CredentialAccountStatus::Active => {}
                CredentialAccountStatus::Expired => {
                    return Err(CredentialBrokerError::CredentialExpired {
                        account_id: requirement.account_id.clone(),
                    }
                    .into());
                }
                CredentialAccountStatus::Revoked => {
                    return Err(CredentialBrokerError::CredentialRevoked {
                        account_id: requirement.account_id.clone(),
                    }
                    .into());
                }
            }
            if !account
                .allowed_targets
                .iter()
                .any(|target| target.matches(&request.method, &request.url))
            {
                return Err(CredentialBrokerError::CredentialPolicyMismatch {
                    account_id: requirement.account_id.clone(),
                }
                .into());
            }
            if !account
                .secret_handles
                .iter()
                .any(|handle| handle == &requirement.handle)
            {
                return Err(CredentialAccountResolverError::MissingSecretHandle {
                    account_id: requirement.account_id.clone(),
                    handle: requirement.handle.clone(),
                });
            }

            push_unique_handle(&mut required_secret_handles, requirement.handle.clone());
            wasm_credentials.push(WasmStagedRuntimeCredential::for_exact_url(
                requirement.handle.clone(),
                requirement.target.clone(),
                requirement.required,
                request.url.clone(),
            ));
        }

        Ok(CredentialAccountResolution {
            required_secret_handles,
            wasm_credentials,
        })
    }
}

fn account_matches_request(
    account: &CredentialAccount,
    request: &CredentialAccountResolverRequest,
) -> bool {
    account.scope.tenant_id == request.scope.tenant_id
        && account.scope.user_id == request.scope.user_id
        && account.scope.agent_id == request.scope.agent_id
        && account.scope.project_id == request.scope.project_id
        && account.provider_or_extension_id == request.extension_id
}

fn push_unique_handle(handles: &mut Vec<SecretHandle>, handle: SecretHandle) {
    if !handles.iter().any(|existing| existing == &handle) {
        handles.push(handle);
    }
}
