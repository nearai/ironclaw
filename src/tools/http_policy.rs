use std::net::{IpAddr, Ipv4Addr};

use crate::config::env_or_override;
use crate::config::http::{
    EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV, EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV,
    EFFECTIVE_HTTP_SECURITY_MODE_ENV, HttpSecurityMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolHttpRuntimePolicy {
    pub security_mode: HttpSecurityMode,
    pub allow_private_http: bool,
    pub allow_private_ip_literals: bool,
}

impl Default for ToolHttpRuntimePolicy {
    fn default() -> Self {
        Self {
            security_mode: HttpSecurityMode::Strict,
            allow_private_http: false,
            allow_private_ip_literals: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IpDisposition {
    Public,
    PrivateNetwork,
    AlwaysBlocked,
}

pub(crate) fn current_runtime_policy() -> ToolHttpRuntimePolicy {
    let security_mode = match env_or_override(EFFECTIVE_HTTP_SECURITY_MODE_ENV)
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("infra_trusted") => HttpSecurityMode::InfraTrusted,
        _ => HttpSecurityMode::Strict,
    };

    let allow_private_http = matches!(
        env_or_override(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV).as_deref(),
        Some("1" | "true" | "TRUE")
    );
    let allow_private_ip_literals = matches!(
        env_or_override(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV).as_deref(),
        Some("1" | "true" | "TRUE")
    );

    ToolHttpRuntimePolicy {
        security_mode,
        allow_private_http,
        allow_private_ip_literals,
    }
}

pub(crate) fn is_localhost_host(host: &str) -> bool {
    let host_lower = host.to_ascii_lowercase();
    host_lower == "localhost" || host_lower.ends_with(".localhost")
}

pub(crate) fn classify_ip(ip: &IpAddr) -> IpDisposition {
    match ip {
        IpAddr::V4(v4) => classify_ipv4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return classify_ipv4(&v4);
            }

            if v6.is_loopback()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
            {
                IpDisposition::AlwaysBlocked
            } else if v6.is_unique_local() {
                IpDisposition::PrivateNetwork
            } else {
                IpDisposition::Public
            }
        }
    }
}

fn classify_ipv4(v4: &Ipv4Addr) -> IpDisposition {
    if v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_unspecified()
        || *v4 == Ipv4Addr::new(169, 254, 169, 254)
    {
        IpDisposition::AlwaysBlocked
    } else if v4.is_private() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64) {
        IpDisposition::PrivateNetwork
    } else {
        IpDisposition::Public
    }
}

pub(crate) fn validate_ip_literal(
    ip: &IpAddr,
    scheme: &str,
    policy: ToolHttpRuntimePolicy,
    allow_localhost_escape_hatch: bool,
) -> Result<(), String> {
    if allow_localhost_escape_hatch {
        return Ok(());
    }

    match classify_ip(ip) {
        IpDisposition::AlwaysBlocked => Err(format!(
            "IP literal {} is denied by local safety policy",
            ip
        )),
        IpDisposition::PrivateNetwork => match policy.security_mode {
            HttpSecurityMode::Strict => Err(format!(
                "private IP literal {} is not allowed in strict mode",
                ip
            )),
            HttpSecurityMode::InfraTrusted => {
                if !policy.allow_private_ip_literals {
                    return Err(format!(
                        "private IP literals are disabled in infra_trusted mode: {}",
                        ip
                    ));
                }
                if scheme == "http" && !policy.allow_private_http {
                    return Err(format!(
                        "plaintext http is disabled for private IP literal {}",
                        ip
                    ));
                }
                Ok(())
            }
        },
        IpDisposition::Public => {
            if scheme != "https" {
                Err("only https URLs are allowed for public IP literals".to_string())
            } else {
                Ok(())
            }
        }
    }
}

pub(crate) fn validate_hostname_resolution(
    host: &str,
    scheme: &str,
    resolved_ips: &[IpAddr],
    policy: ToolHttpRuntimePolicy,
    allow_localhost_escape_hatch: bool,
) -> Result<(), String> {
    if allow_localhost_escape_hatch {
        return Ok(());
    }

    let mut saw_public = false;
    let mut saw_private = false;

    for ip in resolved_ips {
        match classify_ip(ip) {
            IpDisposition::AlwaysBlocked => {
                return Err(format!(
                    "hostname '{}' resolves to address {} denied by local safety policy",
                    host, ip
                ));
            }
            IpDisposition::PrivateNetwork => {
                saw_private = true;
                if policy.security_mode == HttpSecurityMode::Strict {
                    return Err(format!(
                        "hostname '{}' resolves to private IP {} in strict mode",
                        host, ip
                    ));
                }
            }
            IpDisposition::Public => saw_public = true,
        }
    }

    if scheme == "http" {
        if saw_public {
            return Err(format!(
                "plaintext http is only allowed for private-network targets, got hostname '{}'",
                host
            ));
        }
        if saw_private && !policy.allow_private_http {
            return Err(format!(
                "plaintext http is disabled for private target '{}'",
                host
            ));
        }
    } else if scheme != "https" {
        return Err(format!("unsupported URL scheme: {}", scheme));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{remove_runtime_env, set_runtime_env};

    #[test]
    fn classify_ip_distinguishes_private_from_always_blocked() {
        assert_eq!(
            classify_ip(&"10.0.0.1".parse::<IpAddr>().expect("ip")),
            IpDisposition::PrivateNetwork
        );
        assert_eq!(
            classify_ip(&"127.0.0.1".parse::<IpAddr>().expect("ip")),
            IpDisposition::AlwaysBlocked
        );
        assert_eq!(
            classify_ip(&"169.254.169.254".parse::<IpAddr>().expect("ip")),
            IpDisposition::AlwaysBlocked
        );
        assert_eq!(
            classify_ip(&"8.8.8.8".parse::<IpAddr>().expect("ip")),
            IpDisposition::Public
        );
    }

    #[test]
    fn runtime_policy_reads_effective_overrides() {
        set_runtime_env(EFFECTIVE_HTTP_SECURITY_MODE_ENV, "infra_trusted");
        set_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV, "true");
        set_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV, "true");

        let policy = current_runtime_policy();
        assert_eq!(policy.security_mode, HttpSecurityMode::InfraTrusted);
        assert!(policy.allow_private_http);
        assert!(policy.allow_private_ip_literals);

        remove_runtime_env(EFFECTIVE_HTTP_SECURITY_MODE_ENV);
        remove_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_HTTP_ENV);
        remove_runtime_env(EFFECTIVE_HTTP_ALLOW_PRIVATE_IP_LITERALS_ENV);
    }
}
