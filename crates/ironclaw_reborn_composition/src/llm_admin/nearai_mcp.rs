use std::{net::IpAddr, sync::Arc};

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccount, CredentialAccountStatus,
    CredentialAccountUpdateBinding,
};
use ironclaw_host_api::{
    ExtensionId, InvocationId, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceErrorKind, ResourceScope,
};
use ironclaw_product::{
    ExtensionCredentialSetupService, ExtensionCredentialSubmitRequest, LifecyclePackageKind,
    LifecyclePackageRef, LifecycleProductPayload, LifecyclePublicState,
};
use secrecy::{ExposeSecret, SecretString};

use crate::extension_host::extension_activation_credentials::RuntimeExtensionActivationCredentialGate;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, ExtensionManagementPort,
};
use crate::extension_host::webui_extension_credentials::ProductAuthExtensionCredentialSetup;
use crate::{RebornBuildError, RebornProductAuthServices};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearAiMcpEndpoint {
    pub(crate) url: String,
    pub(crate) host_pattern: String,
    pub(crate) port: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct NearAiMcpBootstrapConfig {
    base_url: String,
    api_key: SecretString,
}

const DEFAULT_NEARAI_MCP_BASE_URL: &str = "https://cloud-api.near.ai";

impl NearAiMcpBootstrapConfig {
    pub fn new(
        base_url: impl Into<String>,
        api_key: SecretString,
    ) -> Result<Self, NearAiMcpBootstrapConfigError> {
        let mut base_url = base_url.into().trim().to_string();
        if base_url.is_empty() {
            base_url = DEFAULT_NEARAI_MCP_BASE_URL.to_string();
        }
        if api_key.expose_secret().trim().is_empty() {
            return Err(NearAiMcpBootstrapConfigError::MissingApiKey);
        }
        Ok(Self { base_url, api_key })
    }

    pub fn from_optional_parts(
        base_url: Option<impl Into<String>>,
        api_key: Option<SecretString>,
    ) -> Result<Option<Self>, NearAiMcpBootstrapConfigError> {
        let base_url = base_url
            .map(Into::into)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let api_key = api_key.filter(|value| !value.expose_secret().trim().is_empty());
        match (base_url, api_key) {
            (Some(base_url), Some(api_key)) => Self::new(base_url, api_key).map(Some),
            (None, None) => Ok(None),
            (None, Some(api_key)) => Ok(Some(Self {
                base_url: DEFAULT_NEARAI_MCP_BASE_URL.to_string(),
                api_key,
            })),
            (Some(_), None) => Err(NearAiMcpBootstrapConfigError::MissingApiKey),
        }
    }

    pub(crate) fn endpoint(&self) -> Result<NearAiMcpEndpoint, String> {
        nearai_mcp_endpoint_from_base(Some(&self.base_url))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NearAiMcpBootstrapConfigError {
    #[error("NEARAI_API_KEY is required when NEARAI_BASE_URL is set")]
    MissingApiKey,
    #[error("NEAR AI session token could not be read: {reason}")]
    SessionTokenRead { reason: String },
}

pub(crate) fn nearai_mcp_endpoint_from_env() -> Result<NearAiMcpEndpoint, String> {
    let configured_base = ironclaw_common::env_helpers::env_or_override("NEARAI_BASE_URL")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    nearai_mcp_endpoint_from_base(configured_base.as_deref())
}

pub fn nearai_mcp_bootstrap_config_from_env()
-> Result<Option<NearAiMcpBootstrapConfig>, NearAiMcpBootstrapConfigError> {
    nearai_mcp_bootstrap_config_from_lookup(ironclaw_common::env_helpers::env_or_override)
}

/// Env-shape parsing behind [`nearai_mcp_bootstrap_config_from_env`], with the
/// variable lookup injected so tests stay hermetic without mutating process
/// state.
fn nearai_mcp_bootstrap_config_from_lookup(
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<NearAiMcpBootstrapConfig>, NearAiMcpBootstrapConfigError> {
    let configured_base = lookup("NEARAI_BASE_URL")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let api_key = lookup("NEARAI_API_KEY")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(SecretString::from);
    NearAiMcpBootstrapConfig::from_optional_parts(configured_base, api_key)
}

pub(crate) async fn nearai_mcp_bootstrap_config_from_llm_config(
    config: &ironclaw_llm::LlmConfig,
) -> Result<Option<NearAiMcpBootstrapConfig>, NearAiMcpBootstrapConfigError> {
    if config.active_provider_id() != "nearai" {
        return Ok(None);
    }

    if let Some(api_key) = &config.nearai.api_key {
        return NearAiMcpBootstrapConfig::from_optional_parts(
            Some(config.nearai.base_url.clone()),
            Some(api_key.clone()),
        );
    }

    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    if !session.has_token().await {
        return Ok(None);
    }
    let token = session.get_token().await.map_err(|error| {
        NearAiMcpBootstrapConfigError::SessionTokenRead {
            reason: error.to_string(),
        }
    })?;
    NearAiMcpBootstrapConfig::from_optional_parts(Some(config.nearai.base_url.clone()), Some(token))
}

pub(crate) fn nearai_mcp_endpoint_from_base(
    configured_base: Option<&str>,
) -> Result<NearAiMcpEndpoint, String> {
    let base = configured_base.unwrap_or(DEFAULT_NEARAI_MCP_BASE_URL);
    let mut url = url::Url::parse(base)
        .map_err(|error| format!("NEARAI_BASE_URL must be an absolute URL: {error}"))?;
    if url.scheme() != "https" {
        return Err("NEARAI_BASE_URL must use https".to_string());
    }
    if url.username() != "" || url.password().is_some() {
        return Err("NEARAI_BASE_URL must not include userinfo".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("NEARAI_BASE_URL must not include query or fragment components".to_string());
    }

    let host = url
        .host_str()
        .ok_or_else(|| "NEARAI_BASE_URL must include a host".to_string())?
        .to_ascii_lowercase();
    let ip = host.parse::<IpAddr>().ok();
    if ip.is_some_and(is_forbidden_endpoint_ip) {
        return Err("NEARAI_BASE_URL host is not allowed".to_string());
    }
    if matches!(ip, Some(IpAddr::V6(_))) {
        return Err("NEARAI_BASE_URL IPv6 hosts are not supported yet".to_string());
    }

    let mut path = url.path().trim_end_matches('/').to_string();
    if path.eq_ignore_ascii_case("/v1") {
        path = String::new();
    }
    if path.is_empty() {
        url.set_path("/mcp");
    } else if !path.eq_ignore_ascii_case("/mcp") {
        url.set_path(&format!("{path}/mcp"));
    } else {
        url.set_path("/mcp");
    }

    Ok(NearAiMcpEndpoint {
        url: url.to_string(),
        host_pattern: host,
        port: url.port(),
    })
}

fn is_forbidden_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_multicast()
                || ip.octets()[0] == 0
        }
        IpAddr::V6(ip) => {
            ip.is_unspecified()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || is_documentation_v6(ip)
        }
    }
}

fn is_documentation_v6(ip: std::net::Ipv6Addr) -> bool {
    let segments = ip.segments();
    segments[0] == 0x2001 && segments[1] == 0x0db8
}

pub(crate) async fn bootstrap_nearai_mcp(
    config: Option<NearAiMcpBootstrapConfig>,
    product_auth: &Arc<RebornProductAuthServices>,
    extension_management: &Arc<ExtensionManagementPort>,
    owner_scope: ResourceScope,
) -> Result<NearAiMcpBootstrapOutcome, RebornBuildError> {
    let Some(config) = config else {
        return Ok(NearAiMcpBootstrapOutcome::NotConfigured);
    };
    if !durable_product_auth_storage_enabled() {
        tracing::debug!(
            "NEAR AI MCP credentials are present, but durable product-auth secret storage is not enabled; skipping auto-activation"
        );
        return Ok(NearAiMcpBootstrapOutcome::SkippedUnsupportedStorage);
    }
    config
        .endpoint()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP auto-enable skipped: invalid NEARAI_BASE_URL: {error}"),
        })?;

    let package_ref =
        LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP package ref is invalid: {error}"),
            }
        })?;
    let resource_scope = ResourceScope {
        invocation_id: InvocationId::new(),
        ..owner_scope.without_thread_and_mission()
    };
    let credential_gate = RuntimeExtensionActivationCredentialGate::new(
        resource_scope.clone(),
        product_auth.runtime_credential_account_selection_service(),
    );
    let projection = extension_management
        .project(
            package_ref.clone(),
            &owner_scope.user_id,
            Some(&credential_gate),
        )
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP extension projection failed: {error}"),
        })?;
    let phase = projection.phase;
    // `install_scope` is present exactly when the caller has a visible
    // installation; the projected `phase` is a resting state only for an
    // installed package (a not-installed projection carries a neutral phase).
    let installed = matches!(
        projection.payload.as_ref(),
        Some(LifecycleProductPayload::ExtensionList { extensions, .. })
            if extensions.first().and_then(|extension| extension.install_scope).is_some()
    );
    if installed {
        match phase {
            LifecyclePublicState::Active | LifecyclePublicState::SetupNeeded => {}
            other => {
                tracing::debug!(
                    phase = ?other,
                    "NEAR AI MCP credentials are present, but the extension is not in an auto-activatable state"
                );
                return Ok(NearAiMcpBootstrapOutcome::SkippedNonActivatable);
            }
        }
    }

    let scope = AuthProductScope::new(resource_scope.clone(), AuthSurface::Api);
    let provider =
        AuthProviderId::new("nearai").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP provider id is invalid: {error}"),
        })?;
    let requester_extension =
        ExtensionId::new("nearai").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP requester extension id is invalid: {error}"),
        })?;
    let existing_accounts = product_auth
        .credential_account_record_source()
        .accounts_for_owner(&scope)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP product-auth lookup failed: {error}"),
        })?
        .into_iter()
        .filter(|account| account.provider == provider)
        .collect::<Vec<_>>();

    let credential_decision =
        nearai_mcp_bootstrap_existing_credential_decision(&existing_accounts, &requester_extension);
    let mut submitted_credential = false;
    match credential_decision {
        NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable => {
            tracing::debug!(
                "NEAR AI MCP credential already exists; skipping boot-time token update"
            );
        }
        NearAiMcpBootstrapExistingCredentialDecision::Submit { existing_account } => {
            let credential_submit =
                ProductAuthExtensionCredentialSetup::new(Arc::clone(product_auth))
                    .submit_manual_token(ExtensionCredentialSubmitRequest {
                        scope,
                        provider,
                        label: "NEAR AI API key".to_string(),
                        requester_extension,
                        existing_account,
                        secret: config.api_key,
                    })
                    .await;
            if let Err(error) = credential_submit {
                if is_nearai_mcp_disabled_or_removed(&error) {
                    tracing::debug!(
                        "NEAR AI MCP credentials are present, but the extension participant is disabled or removed; preserving explicit operator state"
                    );
                    return Ok(NearAiMcpBootstrapOutcome::SkippedDisabled);
                }
                if is_nearai_mcp_product_auth_temporarily_unavailable(&error) {
                    tracing::debug!(
                        error = ?error,
                        "NEAR AI MCP credential bootstrap is temporarily unavailable; continuing without auto-activating the extension"
                    );
                    return Ok(NearAiMcpBootstrapOutcome::SkippedUnavailable);
                }
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("NEAR AI MCP product-auth credential submit failed: {error:?}"),
                });
            }
            submitted_credential = true;
        }
    }

    // Bootstrap installs for the owner named by the runtime scope. Extension
    // membership remains per user even when that owner is also the operator.
    let bootstrap_caller = resource_scope.user_id.clone();
    if !installed {
        extension_management
            .install(package_ref.clone(), &bootstrap_caller)
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP extension install failed: {error}"),
            })?;
    }
    // A not-installed (just installed above) or an installed-but-inactive
    // extension activates; an already-active one only reports its credential
    // outcome.
    if !installed || phase == LifecyclePublicState::SetupNeeded {
        extension_management
            .activate_with_credential_gate(
                package_ref,
                ExtensionActivationMode::Static,
                RuntimeExtensionActivationCredentialGate::new(
                    resource_scope,
                    product_auth.runtime_credential_account_selection_service(),
                ),
                &bootstrap_caller,
            )
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP extension activation failed: {error}"),
            })?;
        return Ok(NearAiMcpBootstrapOutcome::Activated);
    }
    // Installed and already active.
    if submitted_credential {
        Ok(NearAiMcpBootstrapOutcome::SubmittedCredential)
    } else {
        Ok(NearAiMcpBootstrapOutcome::ReusedCredential)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NearAiMcpBootstrapOutcome {
    NotConfigured,
    SkippedDisabled,
    SkippedUnavailable,
    SkippedUnsupportedStorage,
    SkippedNonActivatable,
    ReusedCredential,
    SubmittedCredential,
    Activated,
}

impl NearAiMcpBootstrapOutcome {
    pub(crate) fn log_completion(self) {
        match self {
            Self::NotConfigured => tracing::debug!("NEAR AI MCP bootstrap is not configured"),
            Self::SkippedDisabled
            | Self::SkippedUnavailable
            | Self::SkippedUnsupportedStorage
            | Self::SkippedNonActivatable => tracing::debug!(
                outcome = ?self,
                "NEAR AI MCP bootstrap skipped; extension will not be auto-activated"
            ),
            Self::ReusedCredential | Self::SubmittedCredential | Self::Activated => {
                tracing::debug!(outcome = ?self, "NEAR AI MCP bootstrap completed")
            }
        }
    }
}

pub(crate) fn durable_product_auth_storage_enabled() -> bool {
    true
}

fn nearai_mcp_bootstrap_account_is_usable(
    account: &CredentialAccount,
    requester_extension: &ExtensionId,
) -> bool {
    account.status == CredentialAccountStatus::Configured
        && account.access_secret.is_some()
        && account.is_authorized_for_requester(Some(requester_extension))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NearAiMcpBootstrapExistingCredentialDecision {
    ReuseUsable,
    Submit {
        existing_account: Option<CredentialAccountUpdateBinding>,
    },
}

fn nearai_mcp_bootstrap_existing_credential_decision(
    existing_accounts: &[CredentialAccount],
    requester_extension: &ExtensionId,
) -> NearAiMcpBootstrapExistingCredentialDecision {
    if existing_accounts
        .iter()
        .any(|account| nearai_mcp_bootstrap_account_is_usable(account, requester_extension))
    {
        return NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable;
    }

    let existing_account = existing_accounts
        .iter()
        .max_by_key(|account| (account.updated_at, account.created_at, account.id))
        .map(|account| CredentialAccountUpdateBinding {
            account_id: account.id,
            ownership: account.ownership,
            owner_extension: account.owner_extension.clone(),
            granted_extensions: account.granted_extensions.clone(),
        });
    NearAiMcpBootstrapExistingCredentialDecision::Submit { existing_account }
}

fn is_nearai_mcp_disabled_or_removed(error: &ProductSurfaceError) -> bool {
    error.code == ProductSurfaceErrorCode::Forbidden
        && error.kind == ProductSurfaceErrorKind::ParticipantDenied
}

fn is_nearai_mcp_product_auth_temporarily_unavailable(error: &ProductSurfaceError) -> bool {
    error.code == ProductSurfaceErrorCode::Unavailable
        && error.kind == ProductSurfaceErrorKind::ServiceUnavailable
        && error.retryable
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{InvocationId, UserId};

    /// Hermetic stand-in for the env lookup injected into
    /// [`nearai_mcp_bootstrap_config_from_lookup`]: tests must not depend on
    /// or mutate the process environment.
    fn lookup_from<'a>(entries: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_string())
        }
    }

    fn account_for_bootstrap_decision(
        requester_extension: &ExtensionId,
        status: CredentialAccountStatus,
        access_secret: Option<&str>,
        updated_at_secs: i64,
    ) -> CredentialAccount {
        let user_id = UserId::new("nearai-account-user").expect("user");
        CredentialAccount {
            id: ironclaw_auth::CredentialAccountId::new(),
            scope: AuthProductScope::new(
                ResourceScope::local_default(user_id, InvocationId::new()).expect("scope"),
                AuthSurface::Api,
            ),
            provider: AuthProviderId::new("nearai").expect("provider"),
            label: ironclaw_auth::CredentialAccountLabel::new("NEAR AI").expect("label"),
            status,
            ownership: ironclaw_auth::CredentialOwnership::ExtensionOwned,
            owner_extension: Some(requester_extension.clone()),
            granted_extensions: Vec::new(),
            access_secret: access_secret
                .map(|handle| ironclaw_host_api::SecretHandle::new(handle).expect("secret handle")),
            refresh_secret: None,
            scopes: Vec::new(),
            provider_identity: None,
            created_at: chrono::DateTime::from_timestamp(updated_at_secs, 0).expect("timestamp"),
            updated_at: chrono::DateTime::from_timestamp(updated_at_secs, 0).expect("timestamp"),
        }
    }

    #[test]
    fn bootstrap_existing_credential_decision_reuses_any_usable_account() {
        let requester_extension = ExtensionId::new("nearai").expect("extension");
        let older_usable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::Configured,
            Some("nearai-access-secret"),
            10,
        );
        let newer_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            20,
        );

        let decision = nearai_mcp_bootstrap_existing_credential_decision(
            &[newer_unusable, older_usable],
            &requester_extension,
        );

        assert_eq!(
            decision,
            NearAiMcpBootstrapExistingCredentialDecision::ReuseUsable
        );
    }

    #[test]
    fn bootstrap_existing_credential_decision_updates_newest_when_none_are_usable() {
        let requester_extension = ExtensionId::new("nearai").expect("extension");
        let older_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            10,
        );
        let newer_unusable = account_for_bootstrap_decision(
            &requester_extension,
            CredentialAccountStatus::PendingSetup,
            None,
            20,
        );
        let expected_account_id = newer_unusable.id;

        let decision = nearai_mcp_bootstrap_existing_credential_decision(
            &[newer_unusable, older_unusable],
            &requester_extension,
        );

        let NearAiMcpBootstrapExistingCredentialDecision::Submit {
            existing_account: Some(existing_account),
        } = decision
        else {
            panic!("expected newest account update binding, got {decision:?}");
        };
        assert_eq!(existing_account.account_id, expected_account_id);
    }

    #[test]
    fn endpoint_validation_normalizes_custom_https_base() {
        let endpoint = nearai_mcp_endpoint_from_base(Some("https://search.example.test/v1/"))
            .expect("custom endpoint");

        assert_eq!(endpoint.url, "https://search.example.test/mcp");
        assert_eq!(endpoint.host_pattern, "search.example.test");
        assert_eq!(endpoint.port, None);
    }

    #[test]
    fn endpoint_validation_rejects_http_and_forbidden_ips() {
        assert!(nearai_mcp_endpoint_from_base(Some("http://search.example.test")).is_err());
        assert!(nearai_mcp_endpoint_from_base(Some("https://169.254.169.254")).is_err());
        assert!(nearai_mcp_endpoint_from_base(Some("https://224.0.0.1")).is_err());
    }

    #[test]
    fn endpoint_validation_allows_private_loopback_https_targets() {
        let private =
            nearai_mcp_endpoint_from_base(Some("https://10.0.0.12:8443")).expect("private IP");
        let loopback =
            nearai_mcp_endpoint_from_base(Some("https://127.0.0.1")).expect("loopback IP");

        assert_eq!(private.host_pattern, "10.0.0.12");
        assert_eq!(private.port, Some(8443));
        assert_eq!(private.url, "https://10.0.0.12:8443/mcp");
        assert_eq!(loopback.url, "https://127.0.0.1/mcp");
    }

    #[test]
    fn bootstrap_config_from_optional_parts_trims_values() {
        let config = NearAiMcpBootstrapConfig::from_optional_parts(
            Some(" https://private.near.ai/v1/ "),
            Some(SecretString::from(" nearai-test-key ")),
        )
        .expect("valid config")
        .expect("present config");

        assert_eq!(config.base_url, "https://private.near.ai/v1/");
        assert_eq!(
            config.endpoint().expect("endpoint").url,
            "https://private.near.ai/mcp"
        );
    }

    #[test]
    fn bootstrap_config_from_optional_parts_ignores_empty_pair() {
        assert!(
            NearAiMcpBootstrapConfig::from_optional_parts(
                Some("   "),
                Some(SecretString::from(" \t "))
            )
            .expect("empty pair")
            .is_none()
        );
    }

    #[test]
    fn bootstrap_config_from_optional_parts_defaults_base_url_when_api_key_is_present() {
        let config = NearAiMcpBootstrapConfig::from_optional_parts(
            None::<String>,
            Some(SecretString::from("nearai-test-key")),
        )
        .expect("default base url")
        .expect("present config");

        assert_eq!(config.base_url, DEFAULT_NEARAI_MCP_BASE_URL);
        assert_eq!(
            config.endpoint().expect("endpoint").url,
            "https://cloud-api.near.ai/mcp"
        );
    }

    #[test]
    fn bootstrap_config_from_optional_parts_defaults_whitespace_base_url_when_api_key_is_present() {
        let config = NearAiMcpBootstrapConfig::from_optional_parts(
            Some("   "),
            Some(SecretString::from("nearai-test-key")),
        )
        .expect("default base url")
        .expect("present config");

        assert_eq!(config.base_url, DEFAULT_NEARAI_MCP_BASE_URL);
        assert_eq!(
            config.endpoint().expect("endpoint").url,
            "https://cloud-api.near.ai/mcp"
        );
    }

    #[test]
    fn bootstrap_config_from_optional_parts_rejects_base_url_without_api_key() {
        assert_eq!(
            NearAiMcpBootstrapConfig::from_optional_parts(Some("https://private.near.ai"), None)
                .expect_err("missing api key"),
            NearAiMcpBootstrapConfigError::MissingApiKey
        );
    }

    #[test]
    fn bootstrap_config_from_env_defaults_base_url_when_only_api_key_set() {
        let config = nearai_mcp_bootstrap_config_from_lookup(lookup_from(&[(
            "NEARAI_API_KEY",
            "nearai-test-key",
        )]))
        .expect("env config")
        .expect("present config");

        assert_eq!(config.base_url, DEFAULT_NEARAI_MCP_BASE_URL);
        assert_eq!(config.api_key.expose_secret(), "nearai-test-key");
    }

    #[test]
    fn bootstrap_config_from_env_uses_both_env_vars_when_set() {
        let config = nearai_mcp_bootstrap_config_from_lookup(lookup_from(&[
            ("NEARAI_BASE_URL", " https://search.example.test/v1/ "),
            ("NEARAI_API_KEY", " nearai-test-key "),
        ]))
        .expect("env config")
        .expect("present config");

        assert_eq!(config.base_url, "https://search.example.test/v1/");
        assert_eq!(config.api_key.expose_secret(), "nearai-test-key");
        assert_eq!(
            config.endpoint().expect("endpoint").url,
            "https://search.example.test/mcp"
        );
    }

    #[test]
    fn bootstrap_config_from_env_returns_none_when_no_env_vars() {
        assert!(
            nearai_mcp_bootstrap_config_from_lookup(lookup_from(&[]))
                .expect("env config")
                .is_none()
        );
    }
}
