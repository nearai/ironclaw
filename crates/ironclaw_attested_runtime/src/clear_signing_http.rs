//! The upstream descriptor fetcher (attested-signing §D3).
//!
//! Fetches ERC-7730 descriptors from Ledger's Crypto Asset List so the SPA —
//! which has a zero-remote-origins CSP — can read them same-origin through the
//! proxy route.
//!
//! ## The host is an allowlist, not configuration
//!
//! [`ALLOWED_UPSTREAM_HOSTS`] is a compile-time constant. An operator selects
//! *which* allowed upstream to use; they cannot introduce a new one through the
//! environment. A descriptor decides what a hardware wallet displays to a human
//! about to sign, so an attacker who could point this at a host they control
//! would not need to touch the transaction at all — they would only need to
//! change what the device says it is. That is a bigger prize than most
//! injection targets, and it is why the host is not a free-text setting.
//!
//! HTTPS is required for the same reason: the descriptor is only as trustworthy
//! as the channel it arrived on.
//!
//! ## Everything failing shape is the same shape
//!
//! Network error, timeout, non-200, oversized body, unparseable JSON, and a
//! genuine 404 all return [`DescriptorLookup::NotAvailable`]. That is not
//! laziness about error reporting — it is the §D3 contract. A caller must not
//! be able to distinguish "the registry is down" from "no descriptor exists",
//! because the only safe response to both is to block signing, and any
//! distinction is an invitation to treat one of them as recoverable.
//! Operators get the real cause in the log.

use async_trait::async_trait;

use crate::clear_signing::{DescriptorKey, DescriptorLookup, DescriptorSource};

/// Upstreams an operator may select. Compile-time: see the module note.
pub const ALLOWED_UPSTREAM_HOSTS: &[&str] = &["global.api.prd.ledger.com"];

/// Ledger's production Crypto Asset List — the SDK's own default, and the
/// source the plan names.
pub const LEDGER_CAL_BASE_URL: &str = "https://global.api.prd.ledger.com/cal/v1";

/// Selects the upstream. Absent means clear signing stays off.
pub const CLEAR_SIGNING_UPSTREAM_ENV: &str = "ATTESTED_CLEAR_SIGNING_UPSTREAM";

/// A descriptor body larger than this is hostile or a misconfigured endpoint,
/// and must not be allowed to exhaust memory.
const MAX_DESCRIPTOR_BYTES: usize = 512 * 1024;

/// One upstream round trip. Bounded so a hanging registry cannot pin a request
/// handler; a timeout is simply "no descriptor", i.e. blocked.
const UPSTREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Why an upstream configuration was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UpstreamConfigError {
    /// The URL did not parse.
    #[error("clear-signing upstream is not a valid URL")]
    Malformed,

    /// Not HTTPS.
    #[error("clear-signing upstream must be https")]
    NotHttps,

    /// Host is not on the compile-time allowlist.
    #[error("clear-signing upstream host {host} is not allowlisted")]
    HostNotAllowed {
        /// The rejected host.
        host: String,
    },

    /// The HTTP client could not be built.
    #[error("could not build the clear-signing HTTP client: {reason}")]
    Client {
        /// Sanitized builder error.
        reason: String,
    },
}

/// Validate an upstream base URL against the allowlist.
///
/// Split out so the policy is testable without a network or a client.
pub fn validate_upstream(base_url: &str) -> Result<String, UpstreamConfigError> {
    let url = reqwest::Url::parse(base_url).map_err(|_| UpstreamConfigError::Malformed)?;
    if url.scheme() != "https" {
        return Err(UpstreamConfigError::NotHttps);
    }
    let host = url.host_str().ok_or(UpstreamConfigError::Malformed)?;
    // Exact match, not a suffix test: a suffix check would accept
    // `global.api.prd.ledger.com.evil.test`.
    if !ALLOWED_UPSTREAM_HOSTS.contains(&host) {
        return Err(UpstreamConfigError::HostNotAllowed {
            host: host.to_string(),
        });
    }
    Ok(url.to_string().trim_end_matches('/').to_string())
}

/// Fetches descriptors over HTTPS from an allowlisted upstream.
pub struct HttpDescriptorSource {
    client: reqwest::Client,
    base_url: String,
}

impl HttpDescriptorSource {
    /// Build against an explicit base URL, validated against the allowlist.
    pub fn new(base_url: &str) -> Result<Self, UpstreamConfigError> {
        let base_url = validate_upstream(base_url)?;
        let client = reqwest::Client::builder()
            .timeout(UPSTREAM_TIMEOUT)
            .build()
            .map_err(|error| UpstreamConfigError::Client {
                reason: error.to_string(),
            })?;
        Ok(Self { client, base_url })
    }

    /// Build from the environment, or `None` when unset.
    ///
    /// `None` means clear signing stays off and every ceremony blocks — the
    /// correct unconfigured state. A *malformed or disallowed* value is an
    /// error rather than a silent fallback to off, so an operator who meant to
    /// enable this learns they failed instead of quietly shipping a build where
    /// no one can sign.
    pub fn from_env() -> Result<Option<Self>, UpstreamConfigError> {
        Self::from_env_value(std::env::var(CLEAR_SIGNING_UPSTREAM_ENV).ok().as_deref())
    }

    /// The decision [`Self::from_env`] makes, as a pure function of the value.
    ///
    /// Separated so the policy is testable without mutating process
    /// environment — which edition 2024 makes `unsafe`, and which this crate
    /// forbids outright.
    pub fn from_env_value(value: Option<&str>) -> Result<Option<Self>, UpstreamConfigError> {
        match value {
            None => Ok(None),
            Some(value) if value.trim().is_empty() => Ok(None),
            Some(value) => Self::new(value.trim()).map(Some),
        }
    }

    /// The request URL for a key.
    ///
    /// CAL addresses descriptors by chain and contract; the selector narrows
    /// which call within the contract is described.
    fn descriptor_url(&self, key: &DescriptorKey) -> String {
        format!(
            "{}/dapps?chain_id={}&contract={}&selector={}&output=descriptor",
            self.base_url,
            urlencode(&key.chain_id),
            urlencode(&key.contract),
            urlencode(&key.selector),
        )
    }
}

/// Percent-encode a query component. Hand-rolled rather than adding a
/// dependency: the inputs are already normalized hex and CAIP-2 ids, so the
/// unreserved set plus the few punctuation characters those use is sufficient.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[async_trait]
impl DescriptorSource for HttpDescriptorSource {
    async fn lookup(&self, key: &DescriptorKey) -> DescriptorLookup {
        let url = self.descriptor_url(key);

        let response = match self.client.get(&url).send().await {
            Ok(response) => response,
            Err(error) => {
                tracing::debug!(%error, "clear-signing upstream request failed");
                return DescriptorLookup::NotAvailable;
            }
        };

        if !response.status().is_success() {
            tracing::debug!(status = %response.status(), "clear-signing upstream returned non-success");
            return DescriptorLookup::NotAvailable;
        }

        // Bound the body before parsing: an unbounded descriptor is a memory
        // exhaustion vector on a path any authenticated user can trigger.
        let body = match response.bytes().await {
            Ok(body) if body.len() <= MAX_DESCRIPTOR_BYTES => body,
            Ok(body) => {
                tracing::debug!(
                    len = body.len(),
                    "clear-signing descriptor exceeded the size cap"
                );
                return DescriptorLookup::NotAvailable;
            }
            Err(error) => {
                tracing::debug!(%error, "clear-signing upstream body read failed");
                return DescriptorLookup::NotAvailable;
            }
        };

        match serde_json::from_slice::<serde_json::Value>(&body) {
            // An empty or null document describes nothing; treat it as absent
            // rather than handing the device a descriptor with no fields.
            Ok(serde_json::Value::Null) => DescriptorLookup::NotAvailable,
            Ok(descriptor) if descriptor_is_empty(&descriptor) => DescriptorLookup::NotAvailable,
            Ok(descriptor) => DescriptorLookup::Available { descriptor },
            Err(error) => {
                tracing::debug!(%error, "clear-signing descriptor did not parse");
                DescriptorLookup::NotAvailable
            }
        }
    }
}

/// Whether a parsed document carries nothing usable.
fn descriptor_is_empty(descriptor: &serde_json::Value) -> bool {
    match descriptor {
        serde_json::Value::Array(items) => items.is_empty(),
        serde_json::Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The allowlist is the control that matters: a descriptor decides what the
    /// device shows a human, so a hostile upstream would not need to touch the
    /// transaction at all.
    #[test]
    fn only_allowlisted_https_hosts_are_accepted() {
        assert!(validate_upstream(LEDGER_CAL_BASE_URL).is_ok());

        assert_eq!(
            validate_upstream("https://evil.test/cal/v1"),
            Err(UpstreamConfigError::HostNotAllowed {
                host: "evil.test".to_string()
            })
        );
        assert_eq!(
            validate_upstream("http://global.api.prd.ledger.com/cal/v1"),
            Err(UpstreamConfigError::NotHttps),
            "plaintext must be refused: the descriptor is only as good as its channel"
        );
        assert_eq!(
            validate_upstream("not a url"),
            Err(UpstreamConfigError::Malformed)
        );
    }

    /// A suffix check would accept this. The comparison is exact.
    #[test]
    fn a_lookalike_host_that_merely_ends_with_the_allowed_one_is_refused() {
        for host in [
            "global.api.prd.ledger.com.evil.test",
            "evil-global.api.prd.ledger.com.attacker.test",
            "notglobal.api.prd.ledger.com",
        ] {
            assert!(
                matches!(
                    validate_upstream(&format!("https://{host}/cal/v1")),
                    Err(UpstreamConfigError::HostNotAllowed { .. })
                ),
                "{host} must not be accepted"
            );
        }
    }

    /// Unset means off — clear signing must be opted into, never defaulted on.
    /// But a value that is set and WRONG is an error, so an operator who meant
    /// to enable it is told they failed rather than silently shipping a build
    /// where nobody can sign.
    #[test]
    fn an_unset_upstream_is_off_but_a_bad_one_is_an_error() {
        assert!(matches!(
            HttpDescriptorSource::from_env_value(None),
            Ok(None)
        ));
        assert!(
            matches!(HttpDescriptorSource::from_env_value(Some("   ")), Ok(None)),
            "a blank value is the same as unset"
        );
        assert!(
            matches!(
                HttpDescriptorSource::from_env_value(Some("https://evil.test/cal/v1")),
                Err(UpstreamConfigError::HostNotAllowed { .. })
            ),
            "a disallowed host must fail startup, not silently disable the feature"
        );
        assert!(matches!(
            HttpDescriptorSource::from_env_value(Some(LEDGER_CAL_BASE_URL)),
            Ok(Some(_))
        ));
        // Surrounding whitespace from a config file is tolerated.
        assert!(matches!(
            HttpDescriptorSource::from_env_value(Some(&format!("  {LEDGER_CAL_BASE_URL}  "))),
            Ok(Some(_))
        ));
    }

    #[test]
    fn the_request_url_carries_the_whole_key_and_encodes_it() {
        let source = HttpDescriptorSource::new(LEDGER_CAL_BASE_URL).expect("source");
        let url = source.descriptor_url(&DescriptorKey {
            chain_id: "eip155:1".to_string(),
            contract: "0xa0b8".to_string(),
            selector: "0xa9059cbb".to_string(),
        });

        // `:` must be encoded rather than passed raw into the query.
        assert!(url.contains("chain_id=eip155%3A1"), "got {url}");
        assert!(url.contains("contract=0xa0b8"));
        assert!(url.contains("selector=0xa9059cbb"));
        assert!(url.starts_with(LEDGER_CAL_BASE_URL));
    }

    /// A trailing slash on the configured base must not produce a `//` path.
    #[test]
    fn a_trailing_slash_in_configuration_is_normalized() {
        let source =
            HttpDescriptorSource::new("https://global.api.prd.ledger.com/cal/v1/").expect("source");
        assert!(!source.descriptor_url(&key()).contains("v1//"));
    }

    fn key() -> DescriptorKey {
        DescriptorKey {
            chain_id: "eip155:1".to_string(),
            contract: "0xa0b8".to_string(),
            selector: "0xa9059cbb".to_string(),
        }
    }

    /// An empty document describes nothing; handing it to the device would put
    /// a fieldless "clear sign" screen in front of the human.
    #[test]
    fn an_empty_document_counts_as_no_descriptor() {
        assert!(descriptor_is_empty(&serde_json::json!({})));
        assert!(descriptor_is_empty(&serde_json::json!([])));
        assert!(!descriptor_is_empty(&serde_json::json!({"display": {}})));
        assert!(!descriptor_is_empty(&serde_json::json!([{"a": 1}])));
    }
}
