use std::{net::IpAddr, sync::Arc};

use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountUpdateBinding,
};
use ironclaw_host_api::{ExtensionId, InvocationId, ResourceScope, UserId};
use ironclaw_product_workflow::{
    ExtensionCredentialSetupService, ExtensionCredentialSubmitRequest, LifecyclePackageKind,
    LifecyclePackageRef, LifecyclePhase, RebornServicesError, RebornServicesErrorCode,
    RebornServicesErrorKind,
};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    RebornBuildError, RebornProductAuthServices,
    extension_lifecycle::{ExtensionActivationMode, RebornLocalExtensionManagementPort},
    webui_extension_credentials::ProductAuthExtensionCredentialSetup,
};

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

impl NearAiMcpBootstrapConfig {
    pub fn new(
        base_url: impl Into<String>,
        api_key: SecretString,
    ) -> Result<Self, NearAiMcpBootstrapConfigError> {
        let base_url = base_url.into().trim().to_string();
        if base_url.is_empty() {
            return Err(NearAiMcpBootstrapConfigError::MissingBaseUrl);
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
            (None, Some(_)) => Err(NearAiMcpBootstrapConfigError::MissingBaseUrl),
            (Some(_), None) => Err(NearAiMcpBootstrapConfigError::MissingApiKey),
        }
    }

    pub(crate) fn endpoint(&self) -> Result<NearAiMcpEndpoint, String> {
        nearai_mcp_endpoint_from_base(Some(&self.base_url))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NearAiMcpBootstrapConfigError {
    MissingBaseUrl,
    MissingApiKey,
}

impl std::fmt::Display for NearAiMcpBootstrapConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingBaseUrl => {
                write!(f, "NEARAI_BASE_URL is required when NEARAI_API_KEY is set")
            }
            Self::MissingApiKey => {
                write!(f, "NEARAI_API_KEY is required when NEARAI_BASE_URL is set")
            }
        }
    }
}

impl std::error::Error for NearAiMcpBootstrapConfigError {}

pub(crate) fn nearai_mcp_endpoint_from_env() -> Result<NearAiMcpEndpoint, String> {
    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    nearai_mcp_endpoint_from_base(configured_base.as_deref())
}

pub fn nearai_mcp_bootstrap_config_from_env()
-> Result<Option<NearAiMcpBootstrapConfig>, NearAiMcpBootstrapConfigError> {
    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let api_key = std::env::var("NEARAI_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(SecretString::from);
    NearAiMcpBootstrapConfig::from_optional_parts(configured_base, api_key)
}

pub(crate) fn nearai_mcp_endpoint_from_base(
    configured_base: Option<&str>,
) -> Result<NearAiMcpEndpoint, String> {
    let base = configured_base.unwrap_or("https://private.near.ai");
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

pub(crate) async fn bootstrap_local_dev_nearai_mcp(
    config: Option<NearAiMcpBootstrapConfig>,
    product_auth: &Arc<RebornProductAuthServices>,
    extension_management: &Arc<RebornLocalExtensionManagementPort>,
    owner_user_id: &UserId,
) -> Result<(), RebornBuildError> {
    let Some(config) = config else {
        return Ok(());
    };
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
    let phase = extension_management
        .project(package_ref.clone())
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP extension projection failed: {error}"),
        })?
        .phase;
    match phase {
        LifecyclePhase::Discovered | LifecyclePhase::Installed | LifecyclePhase::Active => {}
        LifecyclePhase::Disabled => {
            tracing::debug!(
                "NEAR AI MCP credentials are present, but the extension is disabled; preserving explicit disabled state"
            );
            return Ok(());
        }
        other => {
            tracing::warn!(
                phase = ?other,
                "NEAR AI MCP credentials are present, but the extension is not in an auto-activatable phase"
            );
            return Ok(());
        }
    }

    let scope = AuthProductScope::new(
        ResourceScope::local_default(owner_user_id.clone(), InvocationId::new()).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP auto-enable scope could not be built: {error}"),
            },
        )?,
        AuthSurface::Api,
    );
    let provider =
        AuthProviderId::new("nearai").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP provider id is invalid: {error}"),
        })?;
    let existing_account = product_auth
        .credential_account_record_source()
        .accounts_for_owner(&scope)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP product-auth lookup failed: {error}"),
        })?
        .into_iter()
        .filter(|account| account.provider == provider)
        .max_by_key(|account| (account.updated_at, account.created_at, account.id))
        .map(|account| CredentialAccountUpdateBinding {
            account_id: account.id,
            ownership: account.ownership,
            owner_extension: account.owner_extension.clone(),
            granted_extensions: account.granted_extensions.clone(),
        });

    let credential_submit = ProductAuthExtensionCredentialSetup::new(Arc::clone(product_auth))
        .submit_manual_token(ExtensionCredentialSubmitRequest {
            scope,
            provider,
            label: "NEAR AI API key".to_string(),
            requester_extension: ExtensionId::new("nearai").map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("NEAR AI MCP requester extension id is invalid: {error}"),
                }
            })?,
            existing_account,
            secret: config.api_key,
        })
        .await;
    if let Err(error) = credential_submit {
        if is_nearai_mcp_disabled_or_removed(&error) {
            tracing::debug!(
                "NEAR AI MCP credentials are present, but the extension participant is disabled or removed; preserving explicit operator state"
            );
            return Ok(());
        }
        return Err(RebornBuildError::InvalidConfig {
            reason: format!("NEAR AI MCP product-auth credential submit failed: {error:?}"),
        });
    }

    if phase == LifecyclePhase::Discovered {
        extension_management
            .install(package_ref.clone())
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("NEAR AI MCP extension install failed: {error}"),
            })?;
    }
    match phase {
        LifecyclePhase::Discovered | LifecyclePhase::Installed => {
            extension_management
                .activate(package_ref, ExtensionActivationMode::Static)
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("NEAR AI MCP extension activation failed: {error}"),
                })?;
        }
        LifecyclePhase::Active => {}
        LifecyclePhase::Disabled => {}
        _ => {}
    }
    Ok(())
}

fn is_nearai_mcp_disabled_or_removed(error: &RebornServicesError) -> bool {
    error.code == RebornServicesErrorCode::Forbidden
        && error.kind == RebornServicesErrorKind::ParticipantDenied
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn bootstrap_config_from_optional_parts_rejects_partial_values() {
        assert_eq!(
            NearAiMcpBootstrapConfig::from_optional_parts(
                None::<String>,
                Some(SecretString::from("nearai-test-key"))
            )
            .expect_err("missing base url"),
            NearAiMcpBootstrapConfigError::MissingBaseUrl
        );
        assert_eq!(
            NearAiMcpBootstrapConfig::from_optional_parts(Some("https://private.near.ai"), None)
                .expect_err("missing api key"),
            NearAiMcpBootstrapConfigError::MissingApiKey
        );
    }
}
