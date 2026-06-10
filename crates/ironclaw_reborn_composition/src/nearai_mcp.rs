use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NearAiMcpEndpoint {
    pub(crate) url: String,
    pub(crate) host_pattern: String,
    pub(crate) port: Option<u16>,
}

pub(crate) fn nearai_mcp_endpoint_from_env() -> Result<NearAiMcpEndpoint, String> {
    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    nearai_mcp_endpoint_from_base(configured_base.as_deref())
}

pub(crate) fn nearai_mcp_env_credentials() -> Option<(String, secrecy::SecretString)> {
    #[cfg(test)]
    if let Some((base_url, api_key)) = nearai_mcp_env_credentials_override() {
        return Some((base_url, secrecy::SecretString::from(api_key)));
    }

    let configured_base = std::env::var("NEARAI_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let api_key = std::env::var("NEARAI_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some((configured_base, secrecy::SecretString::from(api_key)))
}

#[cfg(test)]
static NEARAI_MCP_ENV_CREDENTIALS_OVERRIDE: std::sync::Mutex<Option<(String, String)>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
pub(crate) struct NearAiMcpEnvCredentialsOverrideGuard {
    previous: Option<(String, String)>,
}

#[cfg(test)]
impl Drop for NearAiMcpEnvCredentialsOverrideGuard {
    fn drop(&mut self) {
        let mut override_slot = NEARAI_MCP_ENV_CREDENTIALS_OVERRIDE
            .lock()
            .expect("NEAR AI MCP env override lock should not be poisoned");
        *override_slot = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) fn override_nearai_mcp_env_credentials_for_test(
    base_url: impl Into<String>,
    api_key: impl Into<String>,
) -> NearAiMcpEnvCredentialsOverrideGuard {
    let mut override_slot = NEARAI_MCP_ENV_CREDENTIALS_OVERRIDE
        .lock()
        .expect("NEAR AI MCP env override lock should not be poisoned");
    let previous = override_slot.replace((base_url.into(), api_key.into()));
    NearAiMcpEnvCredentialsOverrideGuard { previous }
}

#[cfg(test)]
fn nearai_mcp_env_credentials_override() -> Option<(String, String)> {
    NEARAI_MCP_ENV_CREDENTIALS_OVERRIDE
        .lock()
        .expect("NEAR AI MCP env override lock should not be poisoned")
        .clone()
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
}
