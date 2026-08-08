//! Push subscription record grammar and validation.
//!
//! A subscription is what the browser's `PushManager.subscribe()` returns:
//! an endpoint capability URL on a push service plus the client's P-256
//! public key (`p256dh`) and 16-byte auth secret (`auth`). The endpoint URL
//! is a bearer capability — anyone holding it can attempt (unreadable)
//! deliveries — so records never render it into errors or logs; only the
//! host is ever surfaced.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use crate::error::WebPushError;

/// Longest accepted endpoint URL. Real push-service endpoints run ~150-300
/// bytes; 1 KiB leaves headroom without accepting unbounded input.
pub const MAX_ENDPOINT_BYTES: usize = 1024;

/// Browsers enrolled per user. A user realistically holds a handful of
/// browser profiles; the cap bounds fan-out and storage.
pub const MAX_SUBSCRIPTIONS_PER_USER: usize = 16;

const MAX_USER_AGENT_BYTES: usize = 256;
/// Uncompressed P-256 point: 0x04 || x || y.
const P256_UNCOMPRESSED_POINT_LEN: usize = 65;
const AUTH_SECRET_LEN: usize = 16;

/// A validated push service endpoint URL.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct PushEndpoint(String);

impl PushEndpoint {
    pub fn new(raw: impl Into<String>) -> Result<Self, WebPushError> {
        let raw = raw.into();
        Self::validate(&raw)?;
        Ok(Self(raw))
    }

    fn validate(raw: &str) -> Result<(), WebPushError> {
        if raw.len() > MAX_ENDPOINT_BYTES {
            return Err(WebPushError::InvalidSubscription {
                reason: format!("endpoint exceeds {MAX_ENDPOINT_BYTES} bytes"),
            });
        }
        let parsed = url::Url::parse(raw).map_err(|_| WebPushError::InvalidSubscription {
            reason: "endpoint is not a valid URL".to_string(),
        })?;
        if parsed.scheme() != "https" {
            return Err(WebPushError::InvalidSubscription {
                reason: "endpoint must use https".to_string(),
            });
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(WebPushError::InvalidSubscription {
                reason: "endpoint must not carry userinfo".to_string(),
            });
        }
        if parsed.fragment().is_some() {
            return Err(WebPushError::InvalidSubscription {
                reason: "endpoint must not carry a fragment".to_string(),
            });
        }
        if parsed.port().is_some() {
            return Err(WebPushError::InvalidSubscription {
                reason: "endpoint must use the default https port".to_string(),
            });
        }
        if parsed.host_str().is_none() {
            return Err(WebPushError::InvalidSubscription {
                reason: "endpoint has no host".to_string(),
            });
        }
        Ok(())
    }

    /// Enrollment-time gate: the endpoint's push-service host must be one of
    /// the deployment-declared hosts (the `[[channel.egress]]` entries of the
    /// web-push manifest, resolved at composition — one source of truth, the
    /// same list restricted egress enforces at send time). Shape validation
    /// happens at construction so stored records rehydrate without the list;
    /// this check runs only where a new enrollment is accepted.
    pub fn validate_against_push_services(
        &self,
        allowed_hosts: &[String],
    ) -> Result<(), WebPushError> {
        let host = self.host()?;
        if !allowed_hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&host))
        {
            return Err(WebPushError::UnsupportedPushService { host });
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Lowercased push service host (validated at construction).
    pub fn host(&self) -> Result<String, WebPushError> {
        let parsed = url::Url::parse(&self.0).map_err(|_| WebPushError::InvalidSubscription {
            reason: "endpoint is not a valid URL".to_string(),
        })?;
        parsed
            .host_str()
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| WebPushError::InvalidSubscription {
                reason: "endpoint has no host".to_string(),
            })
    }

    /// Lowercase-hex SHA-256 of the full endpoint URL. The endpoint itself is
    /// a bearer capability the settings surface must not echo, but the browser
    /// can compute the same digest over its local subscription endpoint and
    /// match it against the caller's enrolled set — so a shared browser profile
    /// tells "enrolled for this account" apart from "enrolled for another"
    /// without the backend ever surfacing the URL.
    pub fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(self.0.as_bytes());
        let mut hex = String::with_capacity(hash.len() * 2);
        for byte in hash {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    /// Origin-form path + query for the restricted egress request.
    pub fn path_and_query(&self) -> Result<String, WebPushError> {
        let parsed = url::Url::parse(&self.0).map_err(|_| WebPushError::InvalidSubscription {
            reason: "endpoint is not a valid URL".to_string(),
        })?;
        let mut path = parsed.path().to_string();
        if let Some(query) = parsed.query() {
            path.push('?');
            path.push_str(query);
        }
        Ok(path)
    }
}

impl TryFrom<String> for PushEndpoint {
    type Error = WebPushError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Client keys from `PushSubscription.getKey()` — base64url (unpadded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPushSubscriptionKeys")]
pub struct PushSubscriptionKeys {
    /// Uncompressed P-256 public key (65 bytes decoded), base64url.
    pub p256dh: String,
    /// 16-byte auth secret, base64url.
    pub auth: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UncheckedPushSubscriptionKeys {
    p256dh: String,
    auth: String,
}

impl PushSubscriptionKeys {
    pub fn new(p256dh: impl Into<String>, auth: impl Into<String>) -> Result<Self, WebPushError> {
        let p256dh = p256dh.into();
        let auth = auth.into();
        let p256dh_bytes = decode_b64url_field("p256dh", &p256dh)?;
        if p256dh_bytes.len() != P256_UNCOMPRESSED_POINT_LEN || p256dh_bytes[0] != 0x04 {
            return Err(WebPushError::InvalidSubscription {
                reason: "p256dh must decode to a 65-byte uncompressed P-256 point".to_string(),
            });
        }
        let auth_bytes = decode_b64url_field("auth", &auth)?;
        if auth_bytes.len() != AUTH_SECRET_LEN {
            return Err(WebPushError::InvalidSubscription {
                reason: "auth must decode to 16 bytes".to_string(),
            });
        }
        Ok(Self { p256dh, auth })
    }

    pub fn p256dh_bytes(&self) -> Result<Vec<u8>, WebPushError> {
        decode_b64url_field("p256dh", &self.p256dh)
    }

    pub fn auth_bytes(&self) -> Result<Vec<u8>, WebPushError> {
        decode_b64url_field("auth", &self.auth)
    }
}

impl TryFrom<UncheckedPushSubscriptionKeys> for PushSubscriptionKeys {
    type Error = WebPushError;

    fn try_from(value: UncheckedPushSubscriptionKeys) -> Result<Self, Self::Error> {
        Self::new(value.p256dh, value.auth)
    }
}

fn decode_b64url_field(field: &str, value: &str) -> Result<Vec<u8>, WebPushError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|_| WebPushError::InvalidSubscription {
            reason: format!("{field} is not valid base64url"),
        })
}

/// One enrolled browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushSubscriptionRecord {
    /// Stable id for the enrollment (uuid v4 string).
    pub subscription_id: String,
    pub endpoint: PushEndpoint,
    pub keys: PushSubscriptionKeys,
    /// Bounded, control-stripped browser description for the settings UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// RFC 3339 UTC instant the browser enrolled.
    pub created_at: String,
}

impl PushSubscriptionRecord {
    pub fn new(
        endpoint: PushEndpoint,
        keys: PushSubscriptionKeys,
        user_agent: Option<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            subscription_id: uuid::Uuid::new_v4().to_string(),
            endpoint,
            keys,
            user_agent: user_agent.map(sanitize_user_agent),
            created_at: created_at.into(),
        }
    }
}

fn sanitize_user_agent(raw: String) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if cleaned.len() <= MAX_USER_AGENT_BYTES {
        return cleaned;
    }
    let mut cut = MAX_USER_AGENT_BYTES;
    while cut > 0 && !cleaned.is_char_boundary(cut) {
        cut -= 1;
    }
    cleaned[..cut].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample_keys() -> PushSubscriptionKeys {
        // 65-byte uncompressed point (0x04 then 64 bytes) and 16-byte auth.
        let mut point = vec![0x04u8];
        point.extend_from_slice(&[0x11u8; 64]);
        PushSubscriptionKeys::new(
            URL_SAFE_NO_PAD.encode(point),
            URL_SAFE_NO_PAD.encode([0x22u8; 16]),
        )
        .expect("sample keys are valid")
    }

    #[test]
    fn endpoint_allowlist_gate_admits_declared_hosts_only() {
        let allowed = vec![
            "push.alpha.example".to_string(),
            "push.beta.example".to_string(),
        ];
        let enrolled = PushEndpoint::new("https://push.alpha.example/send/abc123")
            .expect("well-formed endpoint validates by shape");
        enrolled
            .validate_against_push_services(&allowed)
            .expect("declared host is admitted");
        let foreign = PushEndpoint::new("https://evil.example.com/send/abc")
            .expect("shape validation alone admits any https host");
        assert!(matches!(
            foreign.validate_against_push_services(&allowed),
            Err(WebPushError::UnsupportedPushService { host }) if host == "evil.example.com"
        ));
        assert!(
            matches!(
                enrolled.validate_against_push_services(&[]),
                Err(WebPushError::UnsupportedPushService { .. })
            ),
            "an empty declared set fails closed"
        );
    }

    #[test]
    fn endpoint_rejects_unsafe_shapes() {
        for raw in [
            "http://push.alpha.example/send/abc",
            "https://user:pw@push.alpha.example/send/abc",
            "https://push.alpha.example/send/abc#frag",
            "https://push.alpha.example:8443/send/abc",
            "not a url",
        ] {
            assert!(PushEndpoint::new(raw).is_err(), "{raw:?} must be rejected");
        }
        let long = format!(
            "https://push.alpha.example/send/{}",
            "a".repeat(MAX_ENDPOINT_BYTES)
        );
        assert!(PushEndpoint::new(long).is_err());
    }

    #[test]
    fn endpoint_host_comparison_is_case_insensitive() {
        let endpoint = PushEndpoint::new("https://PUSH.Alpha.EXAMPLE/send/abc")
            .expect("host case folds at the boundary");
        assert_eq!(endpoint.host().expect("host"), "push.alpha.example");
        endpoint
            .validate_against_push_services(&["push.ALPHA.example".to_string()])
            .expect("allowlist comparison is case-insensitive");
    }

    #[test]
    fn keys_validate_decoded_shapes() {
        assert!(PushSubscriptionKeys::new("!!!", "AAAAAAAAAAAAAAAAAAAAAA").is_err());
        // Wrong point length.
        assert!(
            PushSubscriptionKeys::new(URL_SAFE_NO_PAD.encode([0x04; 10]), "AAAAAAAAAAAAAAAAAAAAAA")
                .is_err()
        );
        // Wrong auth length.
        let mut point = vec![0x04u8];
        point.extend_from_slice(&[0u8; 64]);
        assert!(
            PushSubscriptionKeys::new(
                URL_SAFE_NO_PAD.encode(&point),
                URL_SAFE_NO_PAD.encode([0u8; 5])
            )
            .is_err()
        );
        sample_keys();
    }

    #[test]
    fn path_and_query_is_origin_form() {
        let endpoint =
            PushEndpoint::new("https://push.alpha.example/fcm/send/abc?x=1").expect("valid");
        assert_eq!(
            endpoint.path_and_query().expect("path"),
            "/fcm/send/abc?x=1"
        );
    }

    #[test]
    fn user_agent_is_bounded_and_control_stripped() {
        let record = PushSubscriptionRecord::new(
            PushEndpoint::new("https://push.alpha.example/send/a").expect("valid"),
            sample_keys(),
            Some(format!("bad\r\nagent {}", "x".repeat(400))),
            "2026-08-08T00:00:00Z",
        );
        let agent = record.user_agent.expect("agent kept");
        assert!(!agent.contains('\n'));
        assert!(agent.len() <= MAX_USER_AGENT_BYTES);
    }
}
