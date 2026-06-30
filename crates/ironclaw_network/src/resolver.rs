use std::net::{IpAddr, ToSocketAddrs};

use ironclaw_host_api::{NetworkPolicy, NetworkTarget};

use crate::{error::NetworkHttpError, policy::is_private_or_loopback_ip, url_target::default_port};

pub trait NetworkResolver: Send + Sync {
    fn resolve_ips(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, NetworkHttpError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemNetworkResolver;

impl NetworkResolver for SystemNetworkResolver {
    fn resolve_ips(&self, host: &str, port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        (host, port)
            .to_socket_addrs()
            .map_err(|error| NetworkHttpError::Dns {
                reason: error.to_string(),
                request_bytes: 0,
                response_bytes: 0,
            })
            .map(|addrs| addrs.map(|addr| addr.ip()).collect())
    }
}

pub(crate) fn resolve_public_ips<R>(
    target: &NetworkTarget,
    policy: &NetworkPolicy,
    resolver: &R,
    request_bytes: u64,
) -> Result<Vec<IpAddr>, NetworkHttpError>
where
    R: NetworkResolver,
{
    let resolved_ips = if let Ok(ip) = target.host.parse::<IpAddr>() {
        vec![ip]
    } else {
        let port = target.port.unwrap_or_else(|| default_port(target.scheme));
        resolver
            .resolve_ips(&target.host, port)
            .map_err(|error| NetworkHttpError::Dns {
                reason: error.to_string(),
                request_bytes,
                response_bytes: error.response_bytes(),
            })?
    };
    if resolved_ips.is_empty() {
        return Err(NetworkHttpError::Dns {
            reason: "network target did not resolve to any IP addresses".to_string(),
            request_bytes,
            response_bytes: 0,
        });
    }
    if policy.deny_private_ip_ranges && resolved_ips.iter().copied().any(is_private_or_loopback_ip)
    {
        return Err(NetworkHttpError::PolicyDenied {
            reason: "network target resolves to a private or host-local IP".to_string(),
            request_bytes,
            response_bytes: 0,
        });
    }
    Ok(resolved_ips)
}
