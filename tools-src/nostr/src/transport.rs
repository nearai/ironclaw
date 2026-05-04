//! Transport layer for Nostr relay communication.
//!
//! Uses the host's `http-request` capability with method "WS" to perform
//! one-shot WebSocket roundtrips. The host interprets method "WS" + wss://
//! URLs as a WebSocket connect-send-recv-close cycle.
//!
//! For services that speak plain HTTP (e.g. nostr.band search), normal
//! HTTP methods are used.

use crate::near::agent::host;

/// Default relays to use when none specified.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
];

/// Parse a JSON array of WS frame strings returned by the host.
///
/// The host's ws_roundtrip now returns a JSON array like `["msg1","msg2"]`
/// instead of newline-separated text. Binary frames appear as base64 strings
/// but for nostr relay communication all frames are text.
fn parse_ws_response_frames(body: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(body).unwrap_or_else(|_| {
        // Fallback: treat the whole body as a single frame (backward compat)
        vec![body.to_string()]
    })
}

/// Publish a signed event to a relay via WebSocket.
///
/// Sends: ["EVENT", <event_json>]
/// Expects: ["OK", event_id, true, "..."]
pub fn publish_event(relay_url: &str, event_json: &str) -> Result<String, String> {
    let payload = format!("[\"EVENT\",{event_json}]");
    let body = payload.as_bytes().to_vec();

    host::log(
        host::LogLevel::Info,
        &format!("Publishing event to {relay_url} via WS"),
    );

    let resp = host::http_request("WS", relay_url, "{}", Some(&body), Some(5000))
        .map_err(|e| format!("WS publish to relay failed: {e}"))?;

    if resp.status != 101 {
        let body_text = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Relay returned unexpected status {}: {}",
            resp.status,
            truncate(&body_text, 300)
        ));
    }

    // Look for ["OK", event_id, true, "..."] in the response frames
    let response_text = String::from_utf8(resp.body)
        .map_err(|e| format!("Invalid UTF-8 response: {e}"))?;

    let frames = parse_ws_response_frames(&response_text);

    // Check for OK in any frame
    for frame in &frames {
        let line = frame.trim();
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(line) {
            if arr.get(0).and_then(|v| v.as_str()) == Some("OK") {
                let ok = arr.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
                let msg = arr.get(3).and_then(|v| v.as_str()).unwrap_or("");
                if ok {
                    return Ok(line.to_string());
                } else {
                    return Err(format!("Relay rejected event: {msg}"));
                }
            }
        }
    }

    // No OK received — relay may not have responded in time
    Ok(frames.join("\n"))
}

/// Query a relay via WebSocket.
///
/// Sends: ["REQ", "<sub-id>", <filter_json>]
/// Collects all ["EVENT", sub_id, event] responses until timeout.
/// Returns the raw JSON array of frame strings from the host.
pub fn query_events(relay_url: &str, filter_json: &str) -> Result<String, String> {
    let sub_id = format!("q{}", &hash_short(filter_json));
    let payload = format!("[\"REQ\",\"{sub_id}\",{filter_json}]");
    let body = payload.as_bytes().to_vec();

    host::log(
        host::LogLevel::Info,
        &format!("Querying {relay_url} via WS"),
    );

    // 8 second timeout to collect events
    let resp = host::http_request("WS", relay_url, "{}", Some(&body), Some(8000))
        .map_err(|e| format!("WS query to relay failed: {e}"))?;

    if resp.status != 101 {
        let body_text = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "Relay query returned unexpected status {}: {}",
            resp.status,
            truncate(&body_text, 300)
        ));
    }

    // Return the raw response body (JSON array of frames) for the caller to parse
    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

/// Search notes using nostr.band API (stays HTTP — it's a REST endpoint).
pub fn search_nostr_band(query: &str, limit: u32) -> Result<String, String> {
    let url = format!(
        "https://api.nostr.band/v1/search?limit={limit}&q={}",
        url_encode(query)
    );

    let headers = r#"{"Accept":"application/json"}"#.to_string();

    host::log(
        host::LogLevel::Info,
        &format!("Searching nostr.band for: {query}"),
    );

    let resp = host::http_request("GET", &url, &headers, None, None)
        .map_err(|e| format!("nostr.band search failed: {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        let body_text = String::from_utf8_lossy(&resp.body);
        return Err(format!(
            "nostr.band returned HTTP {}: {}",
            resp.status,
            truncate(&body_text, 300)
        ));
    }

    String::from_utf8(resp.body).map_err(|e| format!("Invalid UTF-8 response: {e}"))
}

/// Publish to multiple relays, return first success or all errors.
pub fn publish_to_relays(relays: &[String], event_json: &str) -> Result<String, String> {
    let mut errors = Vec::new();
    for relay in relays.iter() {
        match publish_event(relay, event_json) {
            Ok(resp) => {
                host::log(
                    host::LogLevel::Info,
                    &format!("Published to {relay}: {}", truncate(&resp, 100)),
                );
                return Ok(resp);
            }
            Err(e) => {
                host::log(
                    host::LogLevel::Warn,
                    &format!("Failed to publish to {relay}: {e}"),
                );
                errors.push(format!("{relay}: {e}"));
            }
        }
    }
    Err(format!(
        "Failed to publish to any relay:\n{}",
        errors.join("\n")
    ))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0xf) as usize]));
            }
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Short deterministic hash for subscription IDs (first 8 hex chars of a simple FNV-like hash).
fn hash_short(s: &str) -> String {
    let mut h: u64 = 0x811c9dc5;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{:08x}", h)
}
