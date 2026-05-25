use ironclaw_host_api::{NetworkScheme, NetworkTarget};
use thiserror::Error;

use crate::error::NetworkHttpError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NetworkTargetUrlError {
    #[error("invalid URL: {0}")]
    Parse(String),
    #[error("URL userinfo is not allowed")]
    UserinfoDenied,
    #[error("unsupported URL scheme {0}")]
    UnsupportedScheme(String),
    #[error("URL host is required")]
    MissingHost,
}

pub fn network_target_for_url(raw: &str) -> Result<NetworkTarget, NetworkTargetUrlError> {
    network_target_for_url_inner(raw)
}

pub(crate) fn network_target_for_http_url(
    raw: &str,
    request_bytes: u64,
) -> Result<NetworkTarget, NetworkHttpError> {
    network_target_for_url_inner(raw).map_err(|error| NetworkHttpError::InvalidUrl {
        reason: error.to_string(),
        request_bytes,
        response_bytes: 0,
    })
}

fn network_target_for_url_inner(raw: &str) -> Result<NetworkTarget, NetworkTargetUrlError> {
    let url =
        url::Url::parse(raw).map_err(|error| NetworkTargetUrlError::Parse(error.to_string()))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(NetworkTargetUrlError::UserinfoDenied);
    }
    let scheme = match url.scheme() {
        "http" => NetworkScheme::Http,
        "https" => NetworkScheme::Https,
        other => return Err(NetworkTargetUrlError::UnsupportedScheme(other.to_string())),
    };
    let host = url
        .host_str()
        .filter(|host| !host.trim().is_empty())
        .ok_or(NetworkTargetUrlError::MissingHost)?
        .to_ascii_lowercase();
    Ok(NetworkTarget {
        scheme,
        host,
        port: url.port_or_known_default(),
    })
}

pub(crate) fn default_port(scheme: NetworkScheme) -> u16 {
    match scheme {
        NetworkScheme::Http => 80,
        NetworkScheme::Https => 443,
    }
}
