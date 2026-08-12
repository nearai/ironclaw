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
use crate::error::WebAppError;
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
const MAX_TAG_CHARS: usize = 64;
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
pub struct WebAppNotificationPayload {
    pub title: String,
    pub body: String,
    /// App-relative deep link the notification opens (e.g. `/automations`).
    pub url: String,
    /// Coalescing tag: notifications with the same tag replace each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl WebAppNotificationPayload {
    /// Build a payload with every field forced into the notification grammar:
    /// title/body sanitized and char-capped, `url` coerced to an app-relative
    /// path (the service worker opens it), `tag` sanitized, and finally the
    /// body trimmed by **serialized-JSON bytes** so a title made of multi-byte
    /// characters can never blow the single-record push budget. Truncating by
    /// character count alone let 1,500 four-byte emoji pass the char cap yet
    /// serialize past 6 KiB, failing every send.
    pub fn new(
        title: impl Into<String>,
        body: impl Into<String>,
        url: impl Into<String>,
        tag: Option<String>,
    ) -> Self {
        let mut payload = Self {
            title: truncate_chars(&sanitize_text(title.into()), MAX_TITLE_CHARS),
            body: truncate_chars(&sanitize_text(body.into()), MAX_BODY_CHARS),
            url: app_relative_url(url.into()),
            tag: tag.map(sanitize_tag),
        };
        payload.fit_body_to_byte_budget();
        payload
    }

    /// Trim `body` (char-boundary safe) until the serialized payload fits
    /// `MAX_PAYLOAD_JSON_BYTES`. Bounded: each pass drops at least one
    /// character, and only the body shrinks (title/url/tag are already capped).
    fn fit_body_to_byte_budget(&mut self) {
        while self.serialized_len() > MAX_PAYLOAD_JSON_BYTES {
            let char_count = self.body.chars().count();
            if char_count == 0 {
                break;
            }
            // Drop a proportional chunk to converge quickly on large payloads,
            // always at least one character, then re-mark the truncation.
            let overshoot = self.serialized_len() - MAX_PAYLOAD_JSON_BYTES;
            let drop = (overshoot / 2).max(1).min(char_count);
            let kept = char_count.saturating_sub(drop).saturating_sub(1);
            let mut trimmed: String = self.body.chars().take(kept).collect();
            if kept < char_count {
                trimmed.push(ELLIPSIS);
            }
            self.body = trimmed;
        }
    }

    fn serialized_len(&self) -> usize {
        serde_json::to_vec(self)
            .map(|bytes| bytes.len())
            .unwrap_or(usize::MAX)
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, WebAppError> {
        let bytes = serde_json::to_vec(self)
            .map_err(|error| WebAppError::crypto(format!("payload serialization: {error}")))?;
        if bytes.len() > MAX_PAYLOAD_JSON_BYTES {
            return Err(WebAppError::PayloadTooLarge {
                bytes: bytes.len(),
                limit: MAX_PAYLOAD_JSON_BYTES,
            });
        }
        Ok(bytes)
    }
}

/// Coerce a caller-supplied deep link into the app-relative form the service
/// worker's same-origin guard expects: a single leading `/`, no scheme, no
/// protocol-relative `//`, no control bytes. Anything else falls back to `/`
/// (the app root) rather than shipping a link the SW would reject anyway.
fn app_relative_url(raw: String) -> String {
    let is_app_relative = raw.starts_with('/')
        && !raw.starts_with("//")
        && !raw.contains("://")
        && !raw.chars().any(char::is_control);
    if is_app_relative {
        raw
    } else {
        "/".to_string()
    }
}

fn sanitize_tag(raw: String) -> String {
    truncate_chars(&sanitize_text(raw), MAX_TAG_CHARS)
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
pub struct WebAppRequestPlan {
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
    payload: &WebAppNotificationPayload,
    ttl_seconds: u32,
    urgency: PushUrgency,
) -> Result<WebAppRequestPlan, WebAppError> {
    let plaintext = payload.to_json_bytes()?;
    let ua_public = subscription.keys.p256dh_bytes()?;
    let auth = subscription.keys.auth_bytes()?;
    let body = crypto::encrypt_payload(&ua_public, &auth, &plaintext)?;
    Ok(WebAppRequestPlan {
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
        let payload = WebAppNotificationPayload::new(
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
    fn multibyte_body_is_trimmed_to_the_serialized_byte_budget() {
        // 1,500 four-byte emoji clear the character cap but serialize to ~6 KB
        // of escaped JSON; char-only truncation would let this blow the push
        // budget and fail every send. The byte-aware fit must bring it under.
        let payload = WebAppNotificationPayload::new(
            "IronClaw",
            "🍉".repeat(MAX_BODY_CHARS),
            "/automations",
            None,
        );
        let serialized = payload
            .to_json_bytes()
            .expect("multibyte payload fits the budget");
        assert!(
            serialized.len() <= MAX_PAYLOAD_JSON_BYTES,
            "serialized {} bytes exceeds the {MAX_PAYLOAD_JSON_BYTES}-byte budget",
            serialized.len()
        );
    }

    #[test]
    fn non_app_relative_urls_collapse_to_root() {
        for url in [
            "https://evil.example.com/x",
            "//evil.example.com",
            "javascript:alert(1)",
            "/ok\nnewline",
        ] {
            let payload = WebAppNotificationPayload::new("t", "b", url, None);
            assert_eq!(payload.url, "/", "{url:?} must collapse to root");
        }
        let ok = WebAppNotificationPayload::new("t", "b", "/automations?x=1", None);
        assert_eq!(ok.url, "/automations?x=1");
    }

    #[test]
    fn plan_carries_protocol_headers_and_origin_form_path() {
        let subscription = sample_subscription();
        let payload =
            WebAppNotificationPayload::new("IronClaw", "Automation finished", "/automations", None);
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
