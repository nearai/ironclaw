use ironclaw_host_api::{NetworkScheme, NetworkTarget};

use crate::error::NetworkHttpError;

pub fn scheme_label(scheme: NetworkScheme) -> &'static str {
    match scheme {
        NetworkScheme::Http => "http",
        NetworkScheme::Https => "https",
    }
}

pub fn network_target_for_url(
    raw: &str,
    request_bytes: u64,
) -> Result<NetworkTarget, NetworkHttpError> {
    let url = url::Url::parse(raw).map_err(|error| NetworkHttpError::InvalidUrl {
        reason: error.to_string(),
        request_bytes,
        response_bytes: 0,
    })?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NetworkHttpError::InvalidUrl {
            reason: "URL userinfo is not allowed".to_string(),
            request_bytes,
            response_bytes: 0,
        });
    }
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        other => {
            return Err(NetworkHttpError::InvalidUrl {
                reason: format!("unsupported URL scheme {other}"),
                request_bytes,
                response_bytes: 0,
            });
        }
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or_else(|| NetworkHttpError::InvalidUrl {
            reason: "URL host is required".to_string(),
            request_bytes,
            response_bytes: 0,
        })?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port_or_known_default(),
    })
}

pub fn default_port(scheme: NetworkScheme) -> u16 {
    match scheme {
        NetworkScheme::Http => 80,
        NetworkScheme::Https => 443,
    }
}
