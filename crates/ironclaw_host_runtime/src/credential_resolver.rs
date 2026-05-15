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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// Secret handles that should be staged through `InjectSecretOnce` before
    /// the returned WASM staged credential rules can be used.
    pub secret_requirements: Vec<CredentialAccountSecretRequirement>,
    /// Request-scoped WASM credential rules. Each rule is exact-method-and-URL
    /// scoped to the resolver request so resolved credentials cannot silently
    /// bleed into another host-mediated HTTP request in the same invocation.
    pub wasm_credentials: Vec<WasmStagedRuntimeCredential>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccountSecretRequirement {
    pub handle: SecretHandle,
    pub required: bool,
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
/// secret handles and exact-method-and-URL staged credential rules that later
/// obligation handling/runtime composition can consume.
pub struct CredentialAccountResolver<S>
where
    S: CredentialAccountStore + ?Sized,
{
    account_store: Arc<S>,
    requirements: Vec<HostApiCredentialRequirement>,
}

pub type DynCredentialAccountResolver = CredentialAccountResolver<dyn CredentialAccountStore>;

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
        let mut secret_requirements = Vec::new();
        let mut wasm_credentials = Vec::new();
        let mut account_cache = Vec::<(CredentialAccountId, Option<CredentialAccount>)>::new();

        for requirement in self
            .requirements
            .iter()
            .filter(|requirement| requirement.matches_request(request))
        {
            let account = if let Some((_, account)) = account_cache
                .iter()
                .find(|(account_id, _)| account_id == &requirement.account_id)
            {
                account.clone()
            } else {
                let account = self
                    .account_store
                    .get_account(&request.scope, &requirement.account_id)
                    .await?;
                account_cache.push((requirement.account_id.clone(), account.clone()));
                account
            };
            let Some(account) = account else {
                if requirement.required {
                    return Err(CredentialBrokerError::MissingCredential {
                        account_id: requirement.account_id.clone(),
                    }
                    .into());
                }
                continue;
            };

            if account.id != requirement.account_id {
                return Err(CredentialBrokerError::StoreIdentityViolation {
                    requested_account_id: requirement.account_id.clone(),
                    returned_account_id: account.id,
                }
                .into());
            }
            if !account.scope.is_account_visible_to(&request.scope) {
                return Err(CredentialBrokerError::CredentialScopeMismatch {
                    account_id: requirement.account_id.clone(),
                }
                .into());
            }
            if account.provider_or_extension_id != request.extension_id {
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

            push_secret_requirement(
                &mut secret_requirements,
                requirement.handle.clone(),
                requirement.required,
            );
            push_wasm_credential(
                &mut wasm_credentials,
                WasmStagedRuntimeCredential::for_exact_request(
                    requirement.handle.clone(),
                    requirement.target.clone(),
                    requirement.required,
                    request.method,
                    request.url.clone(),
                ),
            );
        }

        debug_assert!(
            wasm_credentials
                .iter()
                .all(|credential| credential.exact_method().is_some()
                    && credential.exact_url().is_some()),
            "credential account resolver must only emit exact-method-and-URL WASM credentials"
        );

        Ok(CredentialAccountResolution {
            secret_requirements,
            wasm_credentials,
        })
    }
}

fn push_secret_requirement(
    requirements: &mut Vec<CredentialAccountSecretRequirement>,
    handle: SecretHandle,
    required: bool,
) {
    if let Some(existing) = requirements
        .iter_mut()
        .find(|existing| existing.handle == handle)
    {
        existing.required |= required;
    } else {
        requirements.push(CredentialAccountSecretRequirement { handle, required });
    }
}

fn push_wasm_credential(
    credentials: &mut Vec<WasmStagedRuntimeCredential>,
    credential: WasmStagedRuntimeCredential,
) {
    if let Some(existing) = credentials
        .iter_mut()
        .find(|existing| same_wasm_credential_scope(existing, &credential))
    {
        existing.required |= credential.required;
    } else {
        credentials.push(credential);
    }
}

fn same_wasm_credential_scope(
    left: &WasmStagedRuntimeCredential,
    right: &WasmStagedRuntimeCredential,
) -> bool {
    left.handle == right.handle
        && left.target == right.target
        && left.exact_method() == right.exact_method()
        && left.exact_url() == right.exact_url()
}
