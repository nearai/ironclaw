//! Notification payload schema and push request planning.
//!
//! The payload JSON is the contract between the server and the web app's
//! service worker (`frontend/public/sw.js` parses exactly these fields).
//! The request plan is pure data — host, origin-form path, headers, and the
//! encrypted body. Transport and the `Authorization: vapid` header stay
//! host-side (the adapter tags the egress request with the VAPID credential
//! handle; the host computes and injects the header).

use serde::{Deserialize, Serialize};

use crate::crypto;
use crate::error::WebPushError;
use crate::subscription::PushSubscriptionRecord;

/// Default TTL for notification pushes: one day. Long enough to survive an
/// offline laptop lid-close, short enough not to replay stale automation
/// noise days later.
pub const DEFAULT_TTL_SECONDS: u32 = 86_400;

/// Soft cap for the serialized payload JSON, leaving margin under the
/// single-record plaintext budget.
pub const MAX_PAYLOAD_JSON_BYTES: usize = 3_800;

const MAX_TITLE_CHARS: usize = 120;
const MAX_BODY_CHARS: usize = 1_500;
const ELLIPSIS: char = '…';

/// RFC 8030 §5.3 urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushUrgency {
    VeryLow,
    Low,
    Normal,
    High,
}

impl PushUrgency {
    pub fn header_value(self) -> &'static str {
        match self {
            Self::VeryLow => "very-low",
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }
}

/// What the service worker receives after the browser decrypts the push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPushNotificationPayload {
    pub title: String,
    pub body: String,
    /// App-relative deep link the notification opens (e.g. `/automations`).
    pub url: String,
    /// Coalescing tag: notifications with the same tag replace each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl WebPushNotificationPayload {
    /// Build a payload, truncating title/body to their display budgets.
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        url: impl Into<String>,
        tag: Option<String>,
    ) -> Self {
        Self {
            title: truncate_chars(&sanitize_text(title.into()), MAX_TITLE_CHARS),
            body: truncate_chars(&sanitize_text(body.into()), MAX_BODY_CHARS),
            url: url.into(),
            tag,
        }
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, WebPushError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| WebPushError::crypto(format!("payload serialization: {error}")))?;
        if bytes.len() > MAX_PAYLOAD_JSON_BYTES {
            return Err(WebPushError::PayloadTooLarge {
                bytes: bytes.len(),
                limit: MAX_PAYLOAD_JSON_BYTES,
            });
        }
        Ok(bytes)
    }
}

fn sanitize_text(raw: String) -> String {
    raw.chars()
        .map(|character| {
            if character == '\n' || character == '\t' {
                character
            } else if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn truncate_chars(raw: &str, max_chars: usize) -> String {
    if raw.chars().count() <= max_chars {
        return raw.to_string();
    }
    let mut truncated: String = raw.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push(ELLIPSIS);
    truncated
}

/// A fully planned push request, minus the host-injected VAPID header.
#[derive(Debug, Clone)]
pub struct WebPushRequestPlan {
    /// Push service host (already allowlist-validated at enrollment).
    pub host: String,
    /// Origin-form path + query of the subscription endpoint.
    pub path_and_query: String,
    /// Protocol headers: TTL, Urgency, Content-Encoding, Content-Type.
    pub headers: Vec<(&'static str, String)>,
    /// The complete `aes128gcm` encrypted body.
    pub body: Vec<u8>,
}

/// Encrypt `payload` for `subscription` and plan the POST.
pub fn build_push_request(
    subscription: &PushSubscriptionRecord,
    payload: &WebPushNotificationPayload,
    ttl_seconds: u32,
    urgency: PushUrgency,
) -> Result<WebPushRequestPlan, WebPushError> {
    let plaintext = payload.to_json_bytes()?;
    let ua_public = subscription.keys.p256dh_bytes()?;
    let auth = subscription.keys.auth_bytes()?;
    let body = crypto::encrypt_payload(&ua_public, &auth, &plaintext)?;
    Ok(WebPushRequestPlan {
        host: subscription.endpoint.host()?,
        path_and_query: subscription.endpoint.path_and_query()?,
        headers: vec![
            ("ttl", ttl_seconds.to_string()),
            ("urgency", urgency.header_value().to_string()),
            ("content-encoding", "aes128gcm".to_string()),
            ("content-type", "application/octet-stream".to_string()),
        ],
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::{PushEndpoint, PushSubscriptionKeys};
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn sample_subscription() -> PushSubscriptionRecord {
        // A syntactically valid (but random) recipient: any 65-byte point
        // with 0x04 prefix passes shape checks; encryption then requires it
        // to be on-curve, so use a real generated key.
        let private = aws_lc_rs::agreement::PrivateKey::generate(&aws_lc_rs::agreement::ECDH_P256)
            .expect("generate");
        let public = private.compute_public_key().expect("public");
        PushSubscriptionRecord::new(
            PushEndpoint::new("https://push.beta.example/wpush/v2/token?x=1").expect("endpoint"),
            PushSubscriptionKeys::new(
                URL_SAFE_NO_PAD.encode(public.as_ref()),
                URL_SAFE_NO_PAD.encode([7u8; 16]),
            )
            .expect("keys"),
            Some("TestBrowser/1.0".to_string()),
            "2026-08-08T00:00:00Z",
        )
    }

    #[test]
    fn payload_truncates_and_strips_controls() {
        let payload = WebPushNotificationPayload::new(
            "t\u{0007}itle".to_string(),
            "b".repeat(5_000),
            "/automations",
            None,
        );
        assert_eq!(payload.title, "t itle");
        assert!(payload.body.chars().count() <= MAX_BODY_CHARS);
        assert!(payload.body.ends_with(ELLIPSIS));
    }

    #[test]
    fn plan_carries_protocol_headers_and_origin_form_path() {
        let subscription = sample_subscription();
        let payload = WebPushNotificationPayload::new(
            "IronClaw",
            "Automation finished",
            "/automations",
            None,
        );
        let plan = build_push_request(
            &subscription,
            &payload,
            DEFAULT_TTL_SECONDS,
            PushUrgency::Normal,
        )
        .expect("plan builds");
        assert_eq!(plan.host, "push.beta.example");
        assert_eq!(plan.path_and_query, "/wpush/v2/token?x=1");
        let header = |name: &str| {
            plan.headers
                .iter()
                .find(|(header, _)| *header == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(header("ttl").as_deref(), Some("86400"));
        assert_eq!(header("urgency").as_deref(), Some("normal"));
        assert_eq!(header("content-encoding").as_deref(), Some("aes128gcm"));
        assert!(plan.body.len() > 86, "encrypted body present");
    }
}
