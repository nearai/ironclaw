//! The decisions endpoint the classifier posts to, and the egress pin derived
//! from it.

use ironclaw_host_api::action::{NetworkPolicy, NetworkScheme, NetworkTargetPattern};

use crate::classifier::JevConfigError;

/// TypeSafe's decisions endpoint, the default. Any provider serving the same
/// decisions API can be configured instead.
pub const DEFAULT_JEV_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
/// The host of [`DEFAULT_JEV_ENDPOINT`].
const DEFAULT_JEV_HOST: &str = "api.typesafe.ai";
const HTTPS_PORT: u16 = 443;

/// A checked decisions endpoint: an `https` URL with a host name (not an IP
/// address), no userinfo, no query and no fragment. The egress policy the
/// classifier sends with every request allows exactly this endpoint's scheme,
/// host and port, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JevEndpoint {
    url: String,
    host: String,
    port: u16,
}

impl Default for JevEndpoint {
    /// [`DEFAULT_JEV_ENDPOINT`].
    fn default() -> Self {
        Self {
            url: DEFAULT_JEV_ENDPOINT.to_string(),
            host: DEFAULT_JEV_HOST.to_string(),
            port: HTTPS_PORT,
        }
    }
}

impl JevEndpoint {
    /// Check `raw`. The error names the rule broken, never the URL, which
    /// could carry a password in its userinfo.
    pub fn parse(raw: &str) -> Result<Self, JevConfigError> {
        let invalid = |reason: &'static str| JevConfigError::InvalidEndpoint { reason };
        let url = url::Url::parse(raw.trim()).map_err(|error| match error {
            url::ParseError::EmptyHost => invalid("it must name a host"),
            _ => invalid("it is not a valid URL"),
        })?;
        if url.scheme() != "https" {
            return Err(invalid("it must use https"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid("it must not carry userinfo"));
        }
        if url.query().is_some() {
            return Err(invalid("it must not carry a query"));
        }
        if url.fragment().is_some() {
            return Err(invalid("it must not carry a fragment"));
        }
        // An address is refused outright: the pin denies private ranges, so
        // a private or loopback address would pass startup and then fail
        // every request, and a public one is better named by its host name.
        if matches!(url.host(), Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))) {
            return Err(invalid("its host must be a host name, not an IP address"));
        }
        let host = url
            .host_str()
            .filter(|host| !host.is_empty())
            .ok_or(invalid("it must name a host"))?
            .to_ascii_lowercase();
        // The pin is an exact host: a wildcard label would widen it.
        if host.contains('*') {
            return Err(invalid("its host must not be a wildcard"));
        }
        ironclaw_network::parse_host_pattern(&host).map_err(|error| {
            // The host only (userinfo is already refused), at `debug!`.
            tracing::debug!(error = %error, "Jev endpoint host refused as an egress pin");
            invalid("its host must be a host name")
        })?;
        let port = url.port_or_known_default().unwrap_or(HTTPS_PORT);
        Ok(Self {
            url: url.to_string(),
            host,
            port,
        })
    }

    /// The URL requests are posted to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The host the egress pin allows.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The egress policy for this endpoint: HTTPS to exactly its host and
    /// port, private address ranges denied.
    pub(crate) fn policy(&self) -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: self.host.clone(),
                port: Some(self.port),
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_good_url_keeps_its_path_and_pins_its_host_and_port() {
        let default = JevEndpoint::default();
        assert_eq!(JevEndpoint::parse(DEFAULT_JEV_ENDPOINT), Ok(default));
        let endpoint =
            JevEndpoint::parse(" https://Jev.Example.test:8443/api/v1/decisions ").expect("valid");
        assert_eq!(
            endpoint.url(),
            "https://jev.example.test:8443/api/v1/decisions"
        );
        assert_eq!(endpoint.host(), "jev.example.test");
        let policy = endpoint.policy();
        assert_eq!(
            policy.allowed_targets,
            vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "jev.example.test".to_string(),
                port: Some(8443),
            }]
        );
        assert!(policy.deny_private_ip_ranges);
    }

    #[test]
    fn anything_but_a_plain_https_url_with_a_host_name_is_refused() {
        const NOT_A_NAME: &str = "its host must be a host name, not an IP address";
        for (raw, reason) in [
            ("http://jev.example.test/v1/systemone", "it must use https"),
            ("https://", "it must name a host"),
            ("not a url", "it is not a valid URL"),
            (
                "https://user:secret@jev.example.test/v1",
                "it must not carry userinfo",
            ),
            (
                "https://jev.example.test/v1?key=1",
                "it must not carry a query",
            ),
            (
                "https://jev.example.test/v1#top",
                "it must not carry a fragment",
            ),
            (
                "https://*.example.test/v1",
                "its host must not be a wildcard",
            ),
            ("https://127.0.0.1:8443/v1/systemone", NOT_A_NAME),
            ("https://8.8.8.8/v1/systemone", NOT_A_NAME),
            ("https://[::1]/v1/systemone", NOT_A_NAME),
        ] {
            let error = JevEndpoint::parse(raw).expect_err(raw);
            assert_eq!(error, JevConfigError::InvalidEndpoint { reason }, "{raw}");
            assert!(!error.to_string().contains("secret"));
        }
    }
}
