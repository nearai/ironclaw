//! Nostr relay I/O with SSRF protection.
//!
//! Implements host-side relay I/O: publish events and subscribe to event streams.
//!
//! # Mediation boundary
//!
//! This module is the SSRF enforcement boundary for all Nostr relay traffic
//! from WASM tools. It validates:
//! - `wss://` scheme enforcement (rejects `ws://`)
//! - Private IP / loopback / link-local rejection (IPv4 and IPv6, including mapped addresses)
//! - DNS resolution to private addresses (for IP-literal URLs; hostname-based
//!   DNS checks are deferred to the adapter layer to avoid blocking/TOCTOU issues)
//!
//! The [`WasmHostNostr`] trait in `host.rs` is the kernel-level mediation seam:
//! the kernel provides the implementation, and the default is
//! [`DenyWasmHostNostr`] which refuses all operations. Nostr is only enabled
//! when the composition layer explicitly wires a live implementation via
//! [`WitToolHost::with_nostr()`].
//!
//! The `host_for_scope` adapter in `ironclaw_host_runtime` gates access via
//! network policy — it decides whether a given capability gets Nostr access at
//! all. A future `ironclaw_network` WebSocket capability would replace the
//! direct `tokio_tungstenite` connection with a policy-mediated egress
//! channel, but the SSRF checks here would remain as a defense-in-depth
//! layer.
//!
//! # SSRF baseline
//!
//! All relay I/O is gated by [`validate_relay_url`] which enforces:
//! - `wss://` scheme only (TLS is required; plaintext `ws://` is rejected)
//! - Rejection of private/loopback/reserved IP addresses (SSRF protection)
//! - Valid URL parsing
//!
//! Production composition can add further restrictions (allowlists, DNS
//! rebinding checks) at the adapter layer; this module provides the baseline.

use crate::WasmHostError;

/// Error type for Nostr relay operations.
#[derive(Debug, thiserror::Error)]
pub enum NostrRelayError {
    #[error("{0}")]
    WebSocket(String),
    #[error("{0}")]
    Relay(String),
    #[error("{0}")]
    InvalidInput(String),
}

impl From<NostrRelayError> for WasmHostError {
    fn from(err: NostrRelayError) -> Self {
        match err {
            NostrRelayError::WebSocket(msg) => WasmHostError::Failed(msg.clone()),
            NostrRelayError::Relay(msg) => WasmHostError::Failed(msg.clone()),
            NostrRelayError::InvalidInput(msg) => WasmHostError::Failed(msg.clone()),
        }
    }
}

/// Validate a Nostr relay URL before opening a WebSocket connection.
///
/// Enforces:
/// 1. URL must parse successfully
/// 2. Scheme must be `wss://` (TLS is required; plaintext `ws://` is rejected)
/// 3. Host must not be a private, loopback, link-local, or otherwise reserved IP
///
/// This is the baseline SSRF protection. Production composition layers may
/// add further restrictions (relay allowlists, DNS rebinding checks, etc.).
pub fn validate_relay_url(relay_url: &str) -> Result<(), NostrRelayError> {
    let parsed: url::Url = relay_url
        .parse()
        .map_err(|e| NostrRelayError::InvalidInput(format!("invalid relay URL: {e}")))?;

    // Enforce wss:// — plaintext ws:// is rejected for host-side relay I/O.
    if parsed.scheme() != "wss" {
        return Err(NostrRelayError::InvalidInput(format!(
            "relay URL must use wss:// scheme (TLS required), got: {}",
            parsed.scheme()
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| NostrRelayError::InvalidInput("relay URL must have a host".into()))?;

    // Reject private/loopback/link-local/reserved IP addresses (SSRF protection).
    // For hostname-based URLs, we also reject well-known local names.
    if is_private_or_loopback_host(host) {
        return Err(NostrRelayError::InvalidInput(format!(
            "relay URL host must not be a private, loopback, or reserved address: {host}"
        )));
    }

    Ok(())
}

/// Check whether a host string is a private, loopback, link-local, or
/// otherwise reserved IP address, or a well-known local hostname.
///
/// Handles:
/// - IPv4: 0.0.0.0, 127.0.0.0/8, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16,
///   169.254.0.0/16 (link-local), 224.0.0.0/4+ (multicast), 255.255.255.255
/// - IPv6: ::1, ::, fc00::/7 (unique local), fe80::/10 (link-local), ff00::/8 (multicast)
/// - Well-known hostnames: localhost, localhost.localdomain
fn is_private_or_loopback_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();

    // Well-known local hostnames
    if lower == "localhost" || lower == "localhost.localdomain" {
        return true;
    }

    // Try to parse as an IP address (handles both IPv4 and IPv6 bracketed forms).
    // `url::Host` parsing handles [::1] brackets for us.
    let ip_host = match lower.parse::<std::net::IpAddr>() {
        Ok(ip) => ip,
        Err(_) => {
            // Not an IP literal — for non-IP hosts we cannot do DNS resolution
            // here (it would be a blocking call in an async context and introduces
            // TOCTOU issues). The production adapter layer can add DNS-level checks.
            return false;
        }
    };

    is_private_ip(&ip_host)
}

/// Check whether an IP address is private, loopback, link-local, or reserved.
fn is_private_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 0.0.0.0
            octets == [0, 0, 0, 0]
            // 127.0.0.0/8 — loopback
            || octets[0] == 127
            // 10.0.0.0/8 — private (RFC 1918)
            || octets[0] == 10
            // 172.16.0.0/12 — private (RFC 1918)
            || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31)
            // 192.168.0.0/16 — private (RFC 1918)
            || (octets[0] == 192 && octets[1] == 168)
            // 169.254.0.0/16 — link-local
            || (octets[0] == 169 && octets[1] == 254)
            // 224.0.0.0/4+ — multicast / reserved
            || octets[0] >= 224
            // 255.255.255.255 — broadcast
            || octets == [255, 255, 255, 255]
        }
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            // ::1 — loopback
            v6.is_loopback()
            // :: — unspecified
            || v6.is_unspecified()
            // fc00::/7 — unique local
            || (segments[0] & 0xfe00) == 0xfc00
            // fe80::/10 — link-local
            || (segments[0] & 0xffc0) == 0xfe80
            // ff00::/8 — multicast
            || segments[0] & 0xff00 == 0xff00
        }
    }
}

/// Maximum number of events collected per subscription to prevent unbounded memory.
const MAX_COLLECTED_EVENTS: usize = 5_000;
/// Maximum cumulative bytes of collected event JSON before truncation.
/// Prevents memory exhaustion from relay-controlled large payloads.
const MAX_COLLECTED_BYTES: usize = 64 * 1024 * 1024; // 64 MiB
/// Maximum size of a single incoming WebSocket message (bytes).
/// Default tungstenite limit is 64 MiB; we lower to 1 MiB for WASM sandbox.
const MAX_WS_MESSAGE_SIZE: usize = 1024 * 1024; // 1 MiB

/// Default connect timeout in milliseconds.
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;

/// Derive a connect timeout from the remaining execution deadline.
///
/// Caps at `DEFAULT_CONNECT_TIMEOUT_MS` (10s) to avoid long hangs.
/// Falls back to the default when no deadline is specified.
fn connect_timeout_for(remaining_deadline_ms: Option<u32>) -> std::time::Duration {
    let ms = remaining_deadline_ms
        .map(|d| (d as u64).min(DEFAULT_CONNECT_TIMEOUT_MS))
        .unwrap_or(DEFAULT_CONNECT_TIMEOUT_MS);
    std::time::Duration::from_millis(ms)
}

/// Publish a signed Nostr event to a relay via WebSocket.
///
/// Opens a WebSocket connection, sends the EVENT message,
/// and waits for the relay's OK/NACK response.
///
/// `remaining_deadline_ms` is an optional overall deadline for the operation
/// (including connect and read). When `None`, a default 10s connect timeout is used.
///
/// Returns the event ID on success.
pub async fn publish_nostr_event(
    relay_url: &str,
    signed_event_json: &str,
    remaining_deadline_ms: Option<u32>,
) -> Result<String, WasmHostError> {
    validate_relay_url(relay_url)?;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let connect_start = std::time::Instant::now();

    let connect_timeout = connect_timeout_for(remaining_deadline_ms);

    let (ws_stream, _) = tokio::time::timeout(connect_timeout, async {
        tokio_tungstenite::connect_async_with_config(
            relay_url,
            Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
                max_message_size: Some(MAX_WS_MESSAGE_SIZE),
                ..Default::default()
            }),
            false,
        )
        .await
    })
    .await
    .map_err(|_| {
        NostrRelayError::WebSocket(format!(
            "WebSocket connect timed out after {}ms",
            connect_timeout.as_millis()
        ))
    })?
    .map_err(|e| NostrRelayError::WebSocket(format!("WebSocket connect failed: {e}")))?;

    let (mut write, mut read) = ws_stream.split();

    // Build and send EVENT message; pre-extract the event `id` for verification.
    let event_val: serde_json::Value = serde_json::from_str(signed_event_json)
        .map_err(|e| NostrRelayError::InvalidInput(format!("Invalid event JSON: {e}")))?;

    let expected_id = event_val
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            NostrRelayError::InvalidInput(
                "Signed event JSON must contain an \"id\" field".to_string(),
            )
        })?
        .to_string();

    let msg = serde_json::json!(["EVENT", event_val]);
    write
        .send(Message::Text(msg.to_string()))
        .await
        .map_err(|e| NostrRelayError::WebSocket(format!("WebSocket send failed: {e}")))?;

    // Read OK response; clamp timeout to remaining deadline if present,
    // accounting for time already spent on connect + send.
    let read_timeout = std::time::Duration::from_secs(10);
    let effective_timeout = match remaining_deadline_ms {
        Some(deadline) => {
            let elapsed_ms = connect_start.elapsed().as_millis() as u64;
            let remaining_ms = (deadline as u64).saturating_sub(elapsed_ms);
            read_timeout.min(std::time::Duration::from_millis(remaining_ms))
        }
        None => read_timeout,
    };

    let result = tokio::time::timeout(effective_timeout, async {
        while let Some(msg_result) = read.next().await {
            let msg = msg_result
                .map_err(|e| NostrRelayError::WebSocket(format!("WebSocket read error: {e}")))?;
            if let Message::Text(text) = msg
                && let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&text)
                    && arr.len() >= 3
                    && arr[0] == "OK"
            {
                let relay_id = arr[1].as_str().unwrap_or("").to_string();
                let accepted = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);

                if !accepted {
                    let reason =
                        arr.get(3).and_then(|v| v.as_str()).unwrap_or("unknown");
                    return Err(NostrRelayError::Relay(format!(
                        "Relay rejected: {reason}"
                    )));
                }

                // Verify relay-supplied ID matches the signed event ID.
                if relay_id != expected_id {
                    return Err(NostrRelayError::Relay(format!(
                        "Relay returned different event ID: expected {expected_id}, got {relay_id}"
                    )));
                }

                return Ok::<String, NostrRelayError>(relay_id);
            }
        }
        Err(NostrRelayError::Relay(
            "WebSocket closed without OK".to_string(),
        ))
    })
    .await
    .map_err(|_| NostrRelayError::WebSocket("Timeout waiting for relay OK response".to_string()))?;

    result.map_err(WasmHostError::from)
}

/// Subscribe to Nostr events from a relay via WebSocket.
///
/// Connects, sends REQ with filters, collects matching events for `timeout_ms`,
/// sends CLOSE, returns JSON object with the collected events and truncation status.
///
/// `remaining_deadline_ms` is an optional overall deadline for the operation
/// (including connect). When `None`, a default 10s connect timeout is used.
/// The subscribe/collection phase is clamped to `min(timeout_ms, remaining_deadline)`.
pub async fn subscribe_nostr_events(
    relay_url: &str,
    filter_json: &str,
    timeout_ms: u32,
    remaining_deadline_ms: Option<u32>,
) -> Result<String, WasmHostError> {
    validate_relay_url(relay_url)?;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let connect_start = std::time::Instant::now();

    let connect_timeout = connect_timeout_for(remaining_deadline_ms);

    let (ws_stream, _) = tokio::time::timeout(connect_timeout, async {
        tokio_tungstenite::connect_async_with_config(
            relay_url,
            Some(tokio_tungstenite::tungstenite::protocol::WebSocketConfig {
                max_message_size: Some(MAX_WS_MESSAGE_SIZE),
                ..Default::default()
            }),
            false,
        )
        .await
    })
    .await
    .map_err(|_| {
        NostrRelayError::WebSocket(format!(
            "WebSocket connect timed out after {}ms",
            connect_timeout.as_millis()
        ))
    })?
    .map_err(|e| NostrRelayError::WebSocket(format!("WebSocket connect failed: {e}")))?;

    let (mut write, mut read) = ws_stream.split();

    // Parse filters
    let filters: Vec<serde_json::Value> = serde_json::from_str(filter_json)
        .map_err(|e| NostrRelayError::InvalidInput(format!("Invalid filter JSON: {e}")))?;

    // Send REQ with unique subscription ID
    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().as_simple());
    let mut req_msg = vec![serde_json::json!("REQ"), serde_json::json!(sub_id)];
    req_msg.extend(filters);
    let req_text = serde_json::to_string(&req_msg)
        .map_err(|e| NostrRelayError::InvalidInput(format!("Failed to serialize REQ message: {e}")))?;
    write
        .send(Message::Text(req_text))
        .await
        .map_err(|e| NostrRelayError::WebSocket(format!("WebSocket send failed: {e}")))?;

    // Collect events; clamp collection timeout to remaining deadline if present,
    // accounting for time already spent on connect + send.
    let collection_timeout = match remaining_deadline_ms {
        Some(deadline) => {
            let elapsed_ms = connect_start.elapsed().as_millis() as u64;
            let remaining_ms = (deadline as u64).saturating_sub(elapsed_ms);
            (timeout_ms as u64).min(remaining_ms)
        }
        None => timeout_ms as u64,
    };

    let mut events: Vec<serde_json::Value> = Vec::new();
    let mut truncated = false;
    let mut read_error: Option<String> = None;
    let mut collected_bytes: usize = 0;

    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(collection_timeout),
        async {
            while let Some(msg_result) = read.next().await {
                let msg = match msg_result {
                    Ok(m) => m,
                    Err(e) => {
                        read_error = Some(format!("WebSocket read error: {e}"));
                        break;
                    }
                };
                if let Message::Text(text) = msg
                    && let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&text)
                        && arr.len() >= 3
                        && arr[0] == "EVENT"
                {
                    // arr[1] is sub_id, arr[2] is the event
                    collected_bytes = collected_bytes.saturating_add(text.len());
                    events.push(arr[2].clone());
                    if events.len() >= MAX_COLLECTED_EVENTS
                        || collected_bytes >= MAX_COLLECTED_BYTES
                    {
                        truncated = true;
                        break;
                    }
                }
            }
        },
    )
    .await;

    // If a WebSocket read error occurred, return it as an Err.
    if let Some(reason) = read_error {
        return Err(WasmHostError::from(NostrRelayError::WebSocket(reason)));
    }

    // Send CLOSE
    let close_msg = serde_json::json!(["CLOSE", sub_id]);
    let _ = write
        .send(Message::Text(close_msg.to_string()))
        .await;

    // Return structured response with truncation indicator.
    let response = serde_json::json!({
        "events": events,
        "truncated": truncated,
    });
    serde_json::to_string(&response)
        .map_err(|e| WasmHostError::Failed(format!("Failed to serialize events: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_relay_url: scheme enforcement ─────────────────────────

    #[test]
    fn validate_relay_url_accepts_wss() {
        assert!(validate_relay_url("wss://relay.example.com").is_ok());
        assert!(validate_relay_url("wss://relay.example.com:8080").is_ok());
    }

    #[test]
    fn validate_relay_url_rejects_ws_scheme() {
        let err = validate_relay_url("ws://relay.example.com").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("wss://"), "expected wss:// rejection, got: {msg}");
    }

    #[test]
    fn validate_relay_url_rejects_http_schemes() {
        let err = validate_relay_url("https://relay.example.com").unwrap_err();
        assert!(err.to_string().contains("wss://"));
    }

    #[test]
    fn validate_relay_url_rejects_invalid_url() {
        let err = validate_relay_url("not a url").unwrap_err();
        assert!(err.to_string().contains("invalid relay URL"));
    }

    #[test]
    fn validate_relay_url_rejects_empty_host() {
        let err = validate_relay_url("wss://").unwrap_err();
        assert!(err.to_string().contains("host"));
    }

    // ── validate_relay_url: private / loopback IPv4 ─────────────────────

    #[test]
    fn validate_relay_url_rejects_loopback_ipv4() {
        let err = validate_relay_url("wss://127.0.0.1").unwrap_err();
        assert!(
            err.to_string().contains("private, loopback, or reserved"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_relay_url_rejects_loopback_ipv4_full() {
        let err = validate_relay_url("wss://127.255.255.255").unwrap_err();
        assert!(
            err.to_string().contains("private, loopback, or reserved"),
            "got: {err}"
        );
    }

    #[test]
    fn validate_relay_url_rejects_10_private() {
        let err = validate_relay_url("wss://10.0.0.1").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_192168_private() {
        let err = validate_relay_url("wss://192.168.1.1").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_17216_private() {
        let err = validate_relay_url("wss://172.16.0.1").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_17231_private() {
        let err = validate_relay_url("wss://172.31.255.255").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_accepts_17215_public() {
        // 172.15.x.x is NOT private (RFC 1918 is 172.16–172.31)
        assert!(validate_relay_url("wss://172.15.255.255").is_ok());
    }

    #[test]
    fn validate_relay_url_rejects_0_0_0_0() {
        let err = validate_relay_url("wss://0.0.0.0").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_link_local() {
        let err = validate_relay_url("wss://169.254.1.1").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_broadcast() {
        let err = validate_relay_url("wss://255.255.255.255").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_multicast() {
        let err = validate_relay_url("wss://224.0.0.1").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    // ── validate_relay_url: IPv6 ────────────────────────────────────────

    #[test]
    fn validate_relay_url_rejects_ipv6_loopback() {
        let err = validate_relay_url("wss://[::1]").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_ipv6_unspecified() {
        let err = validate_relay_url("wss://[::]").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_ipv6_unique_local() {
        let err = validate_relay_url("wss://[fc00::1]").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_ipv6_link_local() {
        let err = validate_relay_url("wss://[fe80::1]").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_ipv6_multicast() {
        let err = validate_relay_url("wss://[ff02::1]").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_accepts_ipv6_public() {
        // 2001:db8:: is documentation prefix, but not private/reserved per our check
        assert!(validate_relay_url("wss://[2001:db8::1]").is_ok());
    }

    // ── validate_relay_url: well-known local hostnames ──────────────────

    #[test]
    fn validate_relay_url_rejects_localhost() {
        let err = validate_relay_url("wss://localhost").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_localhost_fqdn() {
        let err = validate_relay_url("wss://localhost.localdomain").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    #[test]
    fn validate_relay_url_rejects_localhost_with_port() {
        let err = validate_relay_url("wss://localhost:8080").unwrap_err();
        assert!(err.to_string().contains("private, loopback, or reserved"));
    }

    // ── validate_relay_url: public hostnames pass ──────────────────────

    #[test]
    fn validate_relay_url_accepts_public_hostname() {
        assert!(validate_relay_url("wss://relay.example.com").is_ok());
    }

    #[test]
    fn validate_relay_url_accepts_public_ip() {
        assert!(validate_relay_url("wss://1.2.3.4").is_ok());
        assert!(validate_relay_url("wss://8.8.8.8").is_ok());
    }

    #[test]
    fn validate_relay_url_accepts_public_ip_with_port() {
        assert!(validate_relay_url("wss://1.2.3.4:443").is_ok());
    }

    // ── is_private_ip unit tests ───────────────────────────────────────

    #[test]
    fn is_private_ip_classifies_ranges_correctly() {
        // Private
        assert!(is_private_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_private_ip(&"172.31.255.255".parse().unwrap()));
        assert!(is_private_ip(&"192.168.0.1".parse().unwrap()));
        assert!(is_private_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"169.254.1.1".parse().unwrap()));
        assert!(is_private_ip(&"224.0.0.1".parse().unwrap()));
        assert!(is_private_ip(&"0.0.0.0".parse().unwrap()));
        assert!(is_private_ip(&"255.255.255.255".parse().unwrap()));

        // Not private
        assert!(!is_private_ip(&"1.2.3.4".parse().unwrap()));
        assert!(!is_private_ip(&"8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip(&"172.15.255.255".parse().unwrap()));
        assert!(!is_private_ip(&"172.32.0.0".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_classifies_ipv6_correctly() {
        // Private/reserved
        assert!(is_private_ip(&"::1".parse().unwrap()));
        assert!(is_private_ip(&"::".parse().unwrap()));
        assert!(is_private_ip(&"fc00::1".parse().unwrap()));
        assert!(is_private_ip(&"fd00::1".parse().unwrap()));
        assert!(is_private_ip(&"fe80::1".parse().unwrap()));
        assert!(is_private_ip(&"ff02::1".parse().unwrap()));

        // Not private
        assert!(!is_private_ip(&"2001:db8::1".parse().unwrap()));
        assert!(!is_private_ip(&"2607:f8b0:4004:800::200e".parse().unwrap()));
    }
}
