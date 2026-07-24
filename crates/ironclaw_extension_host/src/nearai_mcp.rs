use std::net::IpAddr;

use secrecy::{ExposeSecret, SecretString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearAiMcpEndpoint {
    pub url: String,
    pub host_pattern: String,
    pub port: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct NearAiMcpBootstrapConfig {
    pub base_url: String,
    pub api_key: SecretString,
}

pub const DEFAULT_NEARAI_MCP_BASE_URL: &str = "https://cloud-api.near.ai";

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

    pub fn endpoint(&self) -> Result<NearAiMcpEndpoint, String> {
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

pub fn nearai_mcp_endpoint_from_env() -> Result<NearAiMcpEndpoint, String> {
    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    nearai_mcp_endpoint_from_base(configured_base.as_deref())
}

pub fn nearai_mcp_endpoint_from_base(
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

pub fn durable_product_auth_storage_enabled() -> bool {
    true
}
