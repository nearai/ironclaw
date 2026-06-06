use ironclaw_host_api::{NetworkScheme, NetworkTarget};
use std::borrow::Cow;
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

pub fn is_rfc3986_unreserved_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

pub fn percent_decode_url_component_lossy(input: &str) -> Cow<'_, str> {
    if !input.as_bytes().contains(&b'%') {
        Cow::Borrowed(input)
    } else {
        percent_encoding::percent_decode_str(input).decode_utf8_lossy()
    }
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

#[cfg(test)]
mod tests {
    use super::{is_rfc3986_unreserved_segment, percent_decode_url_component_lossy};
    use std::borrow::Cow;

    #[test]
    fn rfc3986_unreserved_segment_accepts_unreserved_path_parts() {
        for segment in ["abc", "ABC", "abc-._~", "a1_b2.c3~d4"] {
            assert!(is_rfc3986_unreserved_segment(segment), "{segment}");
        }
    }

    #[test]
    fn rfc3986_unreserved_segment_rejects_empty_dot_segments_and_reserved_chars() {
        for segment in ["", ".", "..", "/", "a/b", "?", "a?b", "#", "a#b", "%2f"] {
            assert!(!is_rfc3986_unreserved_segment(segment), "{segment}");
        }
    }

    #[test]
    fn percent_decode_url_component_lossy_borrows_unencoded_component() {
        assert!(matches!(
            percent_decode_url_component_lossy("plain/path"),
            Cow::Borrowed("plain/path")
        ));
    }

    #[test]
    fn percent_decode_url_component_lossy_decodes_encoded_component() {
        assert_eq!(
            percent_decode_url_component_lossy("token%20value").as_ref(),
            "token value"
        );
    }
}
