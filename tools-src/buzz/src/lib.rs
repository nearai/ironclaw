//! Buzz messaging WASM tool for IronClaw.
//!
//! All Buzz-specific logic lives here. The host (ironclaw) provides only
//! generic Nostr primitives. This tool handles:
//! - Channel routing (Buzz channel UUID → `#e` tags)
//! - Thread replies via `#e` tags (NIP-01)
//! - Mentions via `#p` tags
//! - Event construction, signing via host, relay publish via host
//!
//! # Credentials
//!
//! Store your Nostr private key:
//! `ironclaw secret set buzz_private_key <nsec_or_hex>`
//!
//! The host resolves the key from the secrets store at runtime.
//! It never enters the WASM sandbox.

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

use serde::Deserialize;

const MAX_CONTENT_LENGTH: usize = 65536;

// ── Action types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action")]
enum BuzzAction {
    /// Send a message to a Buzz channel.
    #[serde(rename = "send_message")]
    SendMessage {
        /// Buzz channel UUID.
        channel_id: String,
        /// Message content.
        content: String,
        /// WebSocket relay URL (default: Buzz relay).
        relay_url: Option<String>,
        /// Event ID to reply to (creates a thread).
        reply_to_event_id: Option<String>,
        /// Nostr pubkeys to mention via `#p` tags.
        mention_pubkeys: Option<Vec<String>>,
    },
    /// Subscribe to events in a Buzz channel.
    #[serde(rename = "subscribe_channel")]
    SubscribeChannel {
        /// Buzz channel UUID.
        channel_id: String,
        /// WebSocket relay URL (default: Buzz relay).
        relay_url: Option<String>,
        /// Subscribe timeout in ms (default 5000, max 30000).
        timeout_ms: Option<u32>,
        /// Fetch events after this event ID (pagination cursor).
        since_event_id: Option<String>,
        /// Max events to return.
        limit: Option<u32>,
    },
}

// ── Event construction ─────────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct UnsignedEvent {
    kind: u64,
    content: String,
    tags: Vec<Vec<String>>,
    created_at: u64,
    pubkey: String,
}

/// Normalize a Nostr identifier (npub, nsec, or hex) to 64-char lowercase hex.
/// Accepts bech32 (npub1..., nsec1...) or raw 64-char hex strings.
fn normalize_nostr_id(id: &str) -> Result<String, String> {
    let trimmed = id.trim();

    // Try bech32 decode (npub or nsec)
    if trimmed.starts_with("nsec1") || trimmed.starts_with("npub1") {
        let (_hrp, data) =
            bech32::decode(trimmed).map_err(|e| format!("bech32 decode error: {e}"))?;
        if data.len() != 32 {
            return Err(format!("decoded to {} bytes, expected 32", data.len()));
        }
        // Convert bytes to hex — this is what NIP-01 expects for tags
        Ok(data.iter().map(|b| format!("{b:02x}")).collect())
    } else if trimmed.len() == 64 && trimmed.as_bytes().iter().all(|b| b.is_ascii_hexdigit()) {
        // Raw hex — just lowercase it
        Ok(trimmed.to_ascii_lowercase())
    } else {
        Err(format!(
            "invalid Nostr identifier: must be npub1..., nsec1..., or 64-char hex, got: {trimmed}"
        ))
    }
}

fn build_send_event(
    channel_id: &str,
    content: &str,
    reply_to_event_id: &Option<String>,
    mention_pubkeys: &[String],
    pubkey: &str,
    now: u64,
) -> Result<UnsignedEvent, String> {
    // Validate and normalize reply_to_event_id
    let reply_id_normalized = reply_to_event_id
        .as_ref()
        .map(|id| normalize_nostr_id(id))
        .transpose()?
        .map(|s| s.to_ascii_lowercase());

    // Validate and normalize mention_pubkeys
    let mut normalized_mentions = Vec::with_capacity(mention_pubkeys.len());
    for pk in mention_pubkeys {
        normalized_mentions.push(normalize_nostr_id(pk)?.to_ascii_lowercase());
    }

    let mut tags = Vec::new();

    // Buzz channel tag (root event reference)
    tags.push(vec!["e".into(), channel_id.into(), "".into(), "root".into()]);

    // Thread reply tag
    if let Some(ref reply_id) = reply_id_normalized {
        tags.push(vec![
            "e".into(),
            reply_id.clone(),
            "".into(),
            "reply".into(),
        ]);
    }

    // Mention tags
    for pk in &normalized_mentions {
        tags.push(vec!["p".into(), pk.clone()]);
    }

    Ok(UnsignedEvent {
        kind: 1, // NIP-01 text note
        content: content.to_string(),
        tags,
        created_at: now,
        pubkey: pubkey.to_string(),
    })
}

fn build_subscribe_filter(
    channel_id: &str,
    since_event_id: Option<&str>,
    limit: Option<u32>,
) -> Result<serde_json::Value, String> {
    if since_event_id.is_some() {
        return Err("since_event_id is not supported. Use since_timestamp (Unix epoch seconds) instead.".into());
    }

    let mut filter = serde_json::Map::new();

    // Filter by channel: NIP-01 `#e` tag
    filter.insert("#e".into(), serde_json::json!([channel_id]));

    // Kind filter: only text notes
    filter.insert("kinds".into(), serde_json::json!([1]));

    // Limit
    if let Some(n) = limit {
        filter.insert("limit".into(), serde_json::json!(n));
    }

    // Host expects filter_json as Vec<serde_json::Value> (JSON array of filters)
    Ok(serde_json::json!([filter]))
}

// ── Validation ─────────────────────────────────────────────────────────

fn validate_content(content: &str) -> Result<(), String> {
    if content.is_empty() {
        return Err("content must not be empty".into());
    }
    if content.len() > MAX_CONTENT_LENGTH {
        return Err(format!(
            "content too long: {} chars (max {})",
            content.len(),
            MAX_CONTENT_LENGTH
        ));
    }
    Ok(())
}

fn validate_channel_id(channel_id: &str) -> Result<(), String> {
    if channel_id.is_empty() {
        return Err("channel_id must not be empty".into());
    }
    // Basic UUID format check
    let parts: Vec<&str> = channel_id.split('-').collect();
    if parts.len() != 5 {
        return Err(format!(
            "channel_id should be a UUID, got: {channel_id}"
        ));
    }
    Ok(())
}

fn validate_relay_url(url: &str) -> Result<(), String> {
    if !url.starts_with("wss://") && !url.starts_with("ws://") {
        return Err(format!(
            "relay_url must start with wss:// or ws://, got: {url}"
        ));
    }
    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────

fn handle_send_message(
    channel_id: &str,
    content: &str,
    relay_url: &Option<String>,
    reply_to_event_id: &Option<String>,
    mention_pubkeys: &Option<Vec<String>>,
) -> Result<String, String> {
    validate_content(content)?;
    validate_channel_id(channel_id)?;

    let mentions = mention_pubkeys.as_deref().unwrap_or(&[]);

    let relay = match relay_url {
        Some(ref url) => url.as_str(),
        None => "wss://nearbuilders.communities.buzz.xyz",
    };
    validate_relay_url(relay)?;

    let now = near::agent::host::now_millis() / 1000;

    // We need the pubkey to build the event. Probe by signing a minimal event.
    let probe = serde_json::json!({
        "kind": 1,
        "content": "",
        "tags": [],
        "created_at": now,
        "pubkey": ""
    });
    let signed_probe = near::agent::host::nostr_sign_event(
        &serde_json::to_string(&probe).map_err(|e| format!("{e}"))?,
    )
    .map_err(|e| format!("Failed to probe pubkey: {e}"))?;

    let probe_event: serde_json::Value =
        serde_json::from_str(&signed_probe)
            .map_err(|e| format!("Failed to parse probe response: {e}"))?;
    let pubkey = probe_event["pubkey"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if pubkey.is_empty() {
        return Err("Host returned empty pubkey".into());
    }

    // Build the real event
    let unsigned = build_send_event(
        channel_id,
        content,
        reply_to_event_id,
        mentions,
        &pubkey,
        now,
    )?;
    let unsigned_json =
        serde_json::to_string(&unsigned).map_err(|e| format!("Failed to serialize event: {e}"))?;

    // Sign via host
    let signed_json = near::agent::host::nostr_sign_event(&unsigned_json)
        .map_err(|e| format!("Failed to sign event: {e}"))?;

    // Publish to relay
    let event_id = near::agent::host::nostr_publish_event(relay, &signed_json)
        .map_err(|e| format!("Failed to publish: {e}"))?;

    let output = serde_json::json!({
        "event_id": event_id,
        "channel_id": channel_id,
        "status": "ok"
    });
    Ok(serde_json::to_string(&output).map_err(|e| format!("{e}"))?)
}

fn handle_subscribe(
    channel_id: &str,
    relay_url: &Option<String>,
    timeout_ms: Option<u32>,
    since_event_id: Option<&str>,
    limit: Option<u32>,
) -> Result<String, String> {
    validate_channel_id(channel_id)?;

    let relay = match relay_url {
        Some(ref url) => url.as_str(),
        None => "wss://nearbuilders.communities.buzz.xyz",
    };
    validate_relay_url(relay)?;

    let timeout = timeout_ms.unwrap_or(5000).min(30000);

    let filter = build_subscribe_filter(channel_id, since_event_id, limit)?;
    let filter_json =
        serde_json::to_string(&filter).map_err(|e| format!("Failed to serialize filter: {e}"))?;

    let events_json = near::agent::host::nostr_subscribe_events(relay, &filter_json, timeout)
        .map_err(|e| format!("Failed to subscribe: {e}"))?;

    let events: serde_json::Value = serde_json::from_str::<serde_json::Value>(&events_json)
        .map_err(|e| format!("Failed to parse relay response: {e}"))?;

    let output = serde_json::json!({
        "channel_id": channel_id,
        "events": events,
        "status": "ok"
    });
    Ok(serde_json::to_string(&output).map_err(|e| format!("{e}"))?)
}

// ── JSON Schema ───────────────────────────────────────────────────────

const SCHEMA: &str = r#"{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "buzz-tool",
  "description": "Buzz messaging tool — send messages and subscribe to channels via Nostr relay.",
  "type": "object",
  "required": ["action"],
  "properties": {
    "action": {
      "type": "string",
      "enum": ["send_message", "subscribe_channel"],
      "description": "The action to perform."
    }
  },
  "allOf": [
    {
      "if": { "properties": { "action": { "const": "send_message" } } },
      "then": {
        "required": ["channel_id", "content"],
        "properties": {
          "channel_id": {
            "type": "string",
            "description": "Buzz channel UUID."
          },
          "content": {
            "type": "string",
            "description": "Message content for send_message."
          },
          "relay_url": {
            "type": "string",
            "description": "WebSocket relay URL (default: wss://nearbuilders.communities.buzz.xyz)."
          },
          "reply_to_event_id": {
            "type": "string",
            "description": "Event ID to reply to (creates a thread)."
          },
          "mention_pubkeys": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Nostr pubkeys to mention via #p tags."
          }
        }
      }
    },
    {
      "if": { "properties": { "action": { "const": "subscribe_channel" } } },
      "then": {
        "required": ["channel_id"],
        "properties": {
          "channel_id": {
            "type": "string",
            "description": "Buzz channel UUID."
          },
          "relay_url": {
            "type": "string",
            "description": "WebSocket relay URL (default: wss://nearbuilders.communities.buzz.xyz)."
          },
          "timeout_ms": {
            "type": "integer",
            "description": "Subscribe timeout in ms (default 5000, max 30000)."
          },
          "since_event_id": {
            "type": "string",
            "description": "Fetch events after this event ID."
          },
          "limit": {
            "type": "integer",
            "description": "Max events to return from subscribe."
          }
        }
      }
    }
  ]
}"#;

struct BuzzTool;

impl exports::near::agent::tool::Guest for BuzzTool {
    fn execute(req: exports::near::agent::tool::Request) -> exports::near::agent::tool::Response {
        match execute_inner(&req.params) {
            Ok(result) => exports::near::agent::tool::Response {
                output: Some(result),
                error: None,
            },
            Err(e) => exports::near::agent::tool::Response {
                output: None,
                error: Some(e),
            },
        }
    }

    fn schema() -> String {
        SCHEMA.to_string()
    }

    fn description() -> String {
        "Buzz messaging tool for IronClaw. Send messages to Buzz channels \
         and subscribe to events. Uses Nostr host primitives for signing \
         and relay publish — zero Buzz-specific code in ironclaw core."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: BuzzAction =
        serde_json::from_str(params).map_err(|e| format!("Failed to parse action JSON: {e}"))?;

    // Pre-flight: verify the nostr secret exists
    let _ = near::agent::host::secret_exists("buzz_private_key");

    match action {
        BuzzAction::SendMessage {
            channel_id,
            content,
            relay_url,
            reply_to_event_id,
            mention_pubkeys,
        } => handle_send_message(
            &channel_id,
            &content,
            &relay_url,
            &reply_to_event_id,
            &mention_pubkeys,
        ),
        BuzzAction::SubscribeChannel {
            channel_id,
            relay_url,
            timeout_ms,
            since_event_id,
            limit,
        } => handle_subscribe(
            &channel_id,
            &relay_url,
            timeout_ms,
            since_event_id.as_deref(),
            limit,
        ),
    }
}

export!(BuzzTool);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_valid_json() {
        let parsed: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(parsed["$schema"], "http://json-schema.org/draft-07/schema#");
    }

    #[test]
    fn test_send_message_action_parse() {
        let json = r#"{"action":"send_message","channel_id":"8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f","content":"hello"}"#;
        let action: BuzzAction = serde_json::from_str(json).unwrap();
        match action {
            BuzzAction::SendMessage { channel_id, content, .. } => {
                assert_eq!(channel_id, "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f");
                assert_eq!(content, "hello");
            }
            _ => panic!("Expected SendMessage"),
        }
    }

    #[test]
    fn test_subscribe_action_parse() {
        let json = r#"{"action":"subscribe_channel","channel_id":"8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f"}"#;
        let action: BuzzAction = serde_json::from_str(json).unwrap();
        match action {
            BuzzAction::SubscribeChannel { channel_id, .. } => {
                assert_eq!(channel_id, "8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f");
            }
            _ => panic!("Expected SubscribeChannel"),
        }
    }

    #[test]
    fn test_validate_channel_id() {
        assert!(validate_channel_id("8b8e2988-c5d9-4ee1-adf7-5b4d37cccc9f").is_ok());
        assert!(validate_channel_id("").is_err());
        assert!(validate_channel_id("not-a-uuid").is_err());
    }

    #[test]
    fn test_validate_content() {
        assert!(validate_content("hello").is_ok());
        assert!(validate_content("").is_err());
    }

    #[test]
    fn test_build_send_event() {
        let event = build_send_event("test-uuid", "hello", &None, &[], "pk", 1000).unwrap();
        assert_eq!(event.kind, 1);
        assert_eq!(event.content, "hello");
        assert_eq!(event.tags.len(), 1); // root channel tag
        assert_eq!(event.tags[0][0], "e");
    }

    #[test]
    fn test_build_send_event_with_reply() {
        let event = build_send_event(
            "test-uuid",
            "reply",
            &Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            &[],
            "pk",
            1000,
        )
        .unwrap();
        assert_eq!(event.tags.len(), 2);
        assert_eq!(event.tags[1][3], "reply");
    }

    #[test]
    fn test_build_send_event_with_mentions() {
        let event = build_send_event(
            "test-uuid",
            "hi",
            &None,
            &["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()],
            "pk",
            1000,
        )
        .unwrap();
        assert_eq!(event.tags.len(), 2);
        assert_eq!(event.tags[1][0], "p");
    }

    #[test]
    fn test_build_send_event_rejects_short_reply_id() {
        let result = build_send_event("test-uuid", "hi", &Some("short".into()), &[], "pk", 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid Nostr identifier"));
    }

    #[test]
    fn test_build_send_event_rejects_short_mention_pubkey() {
        let result = build_send_event("test-uuid", "hi", &None, &["not64chars".into()], "pk", 1000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid Nostr identifier"));
    }

    #[test]
    fn test_build_send_event_lowercases_hex_ids() {
        let event = build_send_event(
            "test-uuid",
            "hi",
            &Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into()),
            &["BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".into()],
            "pk",
            1000,
        )
        .unwrap();
        assert_eq!(event.tags[1][1], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(event.tags[2][1], "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    }

    #[test]
    fn test_build_subscribe_filter() {
        let filter = build_subscribe_filter("ch-id", None, Some(10)).unwrap();
        // Filter is now wrapped in an array for host deserialization as Vec<Value>
        assert!(filter.is_array());
        let inner = &filter[0];
        assert_eq!(inner["#e"][0], "ch-id");
        assert_eq!(inner["limit"], 10);
    }

    #[test]
    fn test_build_subscribe_filter_rejects_since_event_id() {
        let result = build_subscribe_filter("ch-id", Some("abc"), Some(10));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("since_event_id is not supported"));
    }

    #[test]
    fn test_validate_relay_url() {
        assert!(validate_relay_url("wss://relay.example.com").is_ok());
        assert!(validate_relay_url("ws://localhost:3000").is_ok());
        assert!(validate_relay_url("https://example.com").is_err());
        assert!(validate_relay_url("ftp://relay.example.com").is_err());
        assert!(validate_relay_url("relay.example.com").is_err());
        assert!(validate_relay_url("").is_err());
    }

    #[test]
    fn test_normalize_nostr_id_hex() {
        assert_eq!(
            normalize_nostr_id("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Ok("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
        );
    }

    #[test]
    fn test_normalize_nostr_id_uppercase_hex() {
        assert_eq!(
            normalize_nostr_id("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            Ok("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string())
        );
    }

    #[test]
    fn test_normalize_nostr_id_npub() {
        // npub for pubkey 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
        let npub = "npub10xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqpkge6d";
        let result = normalize_nostr_id(npub).unwrap();
        assert_eq!(
            result,
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        );
    }

    #[test]
    fn test_normalize_nostr_id_invalid() {
        assert!(normalize_nostr_id("not-a-key").is_err());
        assert!(normalize_nostr_id("npub1invalid").is_err());
    }
}
