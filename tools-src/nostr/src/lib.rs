//! Nostr WASM Tool for IronClaw.
//!
//! Publish notes, search, send DMs, react, repost, and manage profiles on
//! the Nostr network. Event signing uses Schnorr/secp256k1 (BIP-340) and
//! runs entirely inside the WASM sandbox. Events are published to relays
//! via one-shot WebSocket roundtrips (method "WS" through the host's
//! http-request capability).
//!
//! # Architecture
//!
//! ```text
//! WASM Tool ──Schnorr signed event──► WS roundtrip ──► Nostr Relays
//! ```
//!
//! # Private Key Storage
//!
//! The private key is stored in the workspace at `nostr/private_key` (hex or
//! nsec1... format). The WASM sandbox reads it via `workspace_read`. This
//! follows the same pattern as the Telegram tool's session storage.
//!
//! # Relay Communication
//!
//! Relays are contacted via WebSocket (wss://). The tool sends ["EVENT",...]
//! or ["REQ",...] and collects responses until timeout. The host normalizes
//! wss:// → https:// for allowlist checks. Nostr.band search stays HTTP.

pub mod event;
pub mod nip04;
pub mod nip19;
pub mod transport;
pub mod types;

use types::*;

wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../wit/tool.wit",
});

struct NostrTool;

impl exports::near::agent::tool::Guest for NostrTool {
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
        let schema = schemars::schema_for!(NostrAction);
        serde_json::to_string(&schema).expect("schema serialization is infallible")
    }

    fn description() -> String {
        "Nostr integration for publishing notes, searching, sending DMs, reacting, \
         reposting, and managing profiles. Event signing via Schnorr/secp256k1 runs \
         entirely in the sandbox. Events are published to Nostr relays over WebSocket \
         (wss://). Store your private key in the workspace at nostr/private_key (hex \
         or nsec1...). Use 'get_pubkey' to verify your setup, 'publish_note' to post, \
         'search_notes' to find content, and 'send_dm' for encrypted direct messages."
            .to_string()
    }
}

fn execute_inner(params: &str) -> Result<String, String> {
    let action: NostrAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {e}"))?;

    near::agent::host::log(
        near::agent::host::LogLevel::Info,
        &format!("Executing Nostr action: {action:?}"),
    );

    match action {
        NostrAction::PublishNote { content, relays } => {
            execute_publish_note(&content, &relays)
        }
        NostrAction::GetProfile { pubkey } => execute_get_profile(&pubkey),
        NostrAction::SetProfile {
            name,
            about,
            picture,
            nip05,
            website,
            relays,
        } => execute_set_profile(name, about, picture, nip05, website, &relays),
        NostrAction::SearchNotes { query, limit } => execute_search_notes(&query, limit),
        NostrAction::GetNotes { authors, limit } => execute_get_notes(&authors, limit),
        NostrAction::SendDm {
            recipient,
            content,
            relays,
        } => execute_send_dm(&recipient, &content, &relays),
        NostrAction::React {
            event_id,
            event_pubkey,
            reaction,
            relays,
        } => execute_react(&event_id, &event_pubkey, &reaction, &relays),
        NostrAction::Repost {
            event_id,
            event_pubkey,
            relays,
        } => execute_repost(&event_id, &event_pubkey, &relays),
        NostrAction::GetPubkey => execute_get_pubkey(),
        NostrAction::SetRelays { relays } => execute_set_relays(&relays),
        NostrAction::GetRelays => execute_get_relays(),
    }
}

// ---------------------------------------------------------------------------
// Key and relay helpers
// ---------------------------------------------------------------------------

/// Load the private key from workspace.
fn load_private_key() -> Result<[u8; 32], String> {
    let stored = near::agent::host::workspace_read("nostr/private_key")
        .ok_or("No private key found. Store your key (hex or nsec1...) at nostr/private_key using memory_write.")?;

    let trimmed = stored.trim();

    if trimmed.starts_with("nsec1") {
        nip19::decode_key(trimmed)
    } else {
        let bytes =
            hex::decode(trimmed).map_err(|e| format!("Invalid hex private key: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!(
                "Private key must be 32 bytes, got {}",
                bytes.len()
            ));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(key)
    }
}

/// Get the current timestamp in seconds.
fn now_secs() -> u64 {
    near::agent::host::now_millis() / 1000
}

/// Resolve relay list: use provided or fall back to stored or defaults.
fn resolve_relays(provided: &[String]) -> Vec<String> {
    if !provided.is_empty() {
        return provided.to_vec();
    }
    // Try loading from workspace
    if let Some(stored) = near::agent::host::workspace_read("nostr/relays.json") {
        if let Ok(relays) = serde_json::from_str::<Vec<String>>(&stored) {
            if !relays.is_empty() {
                return relays;
            }
        }
    }
    transport::DEFAULT_RELAYS.iter().map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Action implementations
// ---------------------------------------------------------------------------

fn execute_publish_note(content: &str, relays: &[String]) -> Result<String, String> {
    let sk = load_private_key()?;
    let created_at = now_secs();
    let relays = resolve_relays(relays);

    let ev = event::build_signed_event(&sk, 1, vec![], content.to_string(), created_at)?;
    let event_id = ev.id.clone();
    let event_json = serde_json::to_string(&ev)
        .map_err(|e| format!("Event serialization: {e}"))?;

    let relay_resp = transport::publish_to_relays(&relays, &event_json)?;

    let result = PublishResult {
        event_id,
        kind: 1,
        relay_response: relay_resp,
    };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_get_profile(pubkey: &str) -> Result<String, String> {
    let pk_bytes = nip19::parse_pubkey(pubkey)?;
    let pk_hex = hex::encode(pk_bytes);

    let relays = resolve_relays(&[]);
    if relays.is_empty() {
        return Err("No relays configured".to_string());
    }

    let filter = serde_json::json!({
        "kinds": [0],
        "authors": [pk_hex],
        "limit": 1
    })
    .to_string();

    // Query multiple relays, return first valid profile found
    let mut last_error: Option<String> = None;
    for relay in &relays {
        match transport::query_events(relay, &filter) {
            Ok(response) => {
                if let Some(profile) = parse_profile_from_relay_response(&response, &pk_hex) {
                    return serde_json::to_string(&profile)
                        .map_err(|e| format!("Serialization: {e}"));
                }
            }
            Err(e) => {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!("Failed to query relay {relay} for profile: {e}"),
                );
                last_error = Some(format!("{relay}: {e}"));
            }
        }
    }

    // No profile found on any relay
    match last_error {
        Some(err) if relays.len() == 1 => {
            // Only one relay was queried and it failed — propagate the error
            Err(format!("Relay query failed: {err}"))
        }
        _ => Ok(serde_json::json!({
            "pubkey": pk_hex,
            "npub": nip19::encode_npub(&pk_bytes).unwrap_or_default(),
            "profile": null,
            "message": "No profile metadata found on any relay"
        })
        .to_string()),
    }
}

fn execute_set_profile(
    name: Option<String>,
    about: Option<String>,
    picture: Option<String>,
    nip05: Option<String>,
    website: Option<String>,
    relays: &[String],
) -> Result<String, String> {
    let sk = load_private_key()?;
    let relays = resolve_relays(relays);

    let metadata = ProfileMetadata {
        name,
        about,
        picture,
        nip05,
        website,
    };

    let content = serde_json::to_string(&metadata)
        .map_err(|e| format!("Profile serialization: {e}"))?;

    let ev = event::build_signed_event(&sk, 0, vec![], content, now_secs())?;
    let event_id = ev.id.clone();
    let event_json = serde_json::to_string(&ev)
        .map_err(|e| format!("Event serialization: {e}"))?;

    let relay_resp = transport::publish_to_relays(&relays, &event_json)?;

    let result = PublishResult {
        event_id,
        kind: 0,
        relay_response: relay_resp,
    };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_search_notes(query: &str, limit: u32) -> Result<String, String> {
    let response = transport::search_nostr_band(query, limit)?;

    // nostr.band returns { notes: [...] } -- parse out the events
    let parsed: serde_json::Value =
        serde_json::from_str(&response).map_err(|e| format!("Parse nostr.band response: {e}"))?;

    let mut notes = Vec::new();
    if let Some(notes_arr) = parsed.get("notes").and_then(|n| n.as_array()) {
        for note_val in notes_arr.iter() {
            if let Some(note_obj) = note_val.get("event").or(Some(note_val)) {
                notes.push(NoteInfo {
                    event_id: note_obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    pubkey: note_obj
                        .get("pubkey")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    created_at: note_obj
                        .get("created_at")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    content: note_obj
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tags: vec![],
                });
            }
        }
    }

    let count = notes.len();
    let result = SearchResult { notes, count };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_get_notes(authors: &[String], limit: u32) -> Result<String, String> {
    let author_hexs: Vec<String> = authors
        .iter()
        .map(|a| nip19::parse_pubkey(a).map(|pk| hex::encode(pk)))
        .collect::<Result<Vec<_>, _>>()?;

    let relays = resolve_relays(&[]);
    if relays.is_empty() {
        return Err("No relays configured".to_string());
    }

    let filter = serde_json::json!({
        "kinds": [1],
        "authors": author_hexs,
        "limit": limit
    })
    .to_string();

    // Query all relays, aggregate and deduplicate by event_id
    let mut seen_ids = std::collections::HashSet::new();
    let mut all_notes: Vec<NoteInfo> = Vec::new();

    for relay in &relays {
        match transport::query_events(relay, &filter) {
            Ok(response) => {
                let notes = parse_events_from_relay_response(&response);
                for note in notes {
                    if seen_ids.insert(note.event_id.clone()) {
                        all_notes.push(note);
                    }
                }
            }
            Err(e) => {
                near::agent::host::log(
                    near::agent::host::LogLevel::Warn,
                    &format!("Failed to query relay {relay} for notes: {e}"),
                );
            }
        }
    }

    let count = all_notes.len();
    let result = SearchResult { notes: all_notes, count };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_send_dm(
    recipient: &str,
    content: &str,
    relays: &[String],
) -> Result<String, String> {
    let sk = load_private_key()?;
    let recipient_pk = nip19::parse_pubkey(recipient)?;
    let relays = resolve_relays(relays);

    // Generate random IV for AES-256-CBC
    let mut iv = [0u8; 16];
    getrandom::getrandom(&mut iv).map_err(|e| format!("Random IV generation failed: {e}"))?;

    // NIP-04: ECDH shared secret → SHA-256 → AES-256-CBC encrypt
    let encrypted_content = nip04::encrypt(content, &sk, &recipient_pk, &iv)?;

    // p-tag the recipient
    let tags = vec![vec!["p".to_string(), hex::encode(recipient_pk)]];

    let ev = event::build_signed_event(&sk, 4, tags, encrypted_content, now_secs())?;
    let event_id = ev.id.clone();
    let event_json =
        serde_json::to_string(&ev).map_err(|e| format!("Event serialization: {e}"))?;

    let relay_resp = transport::publish_to_relays(&relays, &event_json)?;

    let result = PublishResult {
        event_id,
        kind: 4,
        relay_response: relay_resp,
    };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_react(
    event_id: &str,
    event_pubkey: &str,
    reaction: &str,
    relays: &[String],
) -> Result<String, String> {
    let sk = load_private_key()?;
    let relays = resolve_relays(relays);

    // kind 7 reaction: tag the event and author
    let tags = vec![
        vec!["e".to_string(), event_id.to_string()],
        vec!["p".to_string(), event_pubkey.to_string()],
    ];

    let ev = event::build_signed_event(&sk, 7, tags, reaction.to_string(), now_secs())?;
    let result_id = ev.id.clone();
    let event_json = serde_json::to_string(&ev)
        .map_err(|e| format!("Event serialization: {e}"))?;

    let relay_resp = transport::publish_to_relays(&relays, &event_json)?;

    let result = PublishResult {
        event_id: result_id,
        kind: 7,
        relay_response: relay_resp,
    };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_repost(
    event_id: &str,
    event_pubkey: &str,
    relays: &[String],
) -> Result<String, String> {
    let sk = load_private_key()?;
    let relays = resolve_relays(relays);

    // kind 6 repost: tag the event and author, content is empty or the reposted event JSON
    let tags = vec![
        vec!["e".to_string(), event_id.to_string()],
        vec!["p".to_string(), event_pubkey.to_string()],
    ];

    let ev = event::build_signed_event(&sk, 6, tags, String::new(), now_secs())?;
    let result_id = ev.id.clone();
    let event_json = serde_json::to_string(&ev)
        .map_err(|e| format!("Event serialization: {e}"))?;

    let relay_resp = transport::publish_to_relays(&relays, &event_json)?;

    let result = PublishResult {
        event_id: result_id,
        kind: 6,
        relay_response: relay_resp,
    };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_get_pubkey() -> Result<String, String> {
    let sk = load_private_key()?;
    let hex = event::derive_pubkey(&sk)?;
    let bytes: [u8; 32] = hex::decode(&hex)
        .map_err(|e| format!("hex decode: {e}"))?
        .try_into()
        .map_err(|v: Vec<u8>| format!("expected 32 bytes, got {}", v.len()))?;
    let npub = nip19::encode_npub(&bytes)?;

    let result = PubkeyInfo { hex, npub };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

fn execute_set_relays(relays: &[String]) -> Result<String, String> {
    let json = serde_json::to_string(relays)
        .map_err(|e| format!("Serialization: {e}"))?;

    // Store in workspace for persistence (agent should save via memory_write)
    Ok(serde_json::json!({
        "relays": relays,
        "instructions": format!("Save this relay list to nostr/relays.json using memory_write: {json}")
    })
    .to_string())
}

fn execute_get_relays() -> Result<String, String> {
    let relays = resolve_relays(&[]);
    let result = RelayConfig { relays };
    serde_json::to_string(&result).map_err(|e| format!("Serialization: {e}"))
}

// ---------------------------------------------------------------------------
// Relay response parsing helpers
// ---------------------------------------------------------------------------

/// Parse the JSON array of WS frame strings from the host.
///
/// The host ws_roundtrip returns a JSON array like `["msg1","msg2"]`.
/// Falls back to treating the body as a single string for backward compat.
fn parse_frames_from_response(response: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(response).unwrap_or_else(|_| {
        // Fallback: split on newlines for backward compat
        response.lines().map(|l| l.to_string()).collect()
    })
}

/// Parse a profile (kind 0) from relay response.
///
/// The response is a JSON array of frame strings from the host.
fn parse_profile_from_relay_response(response: &str, _expected_pubkey: &str) -> Option<serde_json::Value> {
    let frames = parse_frames_from_response(response);
    for line in frames.iter() {
        let line = line.trim();
        if !line.starts_with("['EVENT") && !line.starts_with("[\"EVENT") {
            continue;
        }
        // Try to parse as JSON array ["EVENT", sub_id, event_obj]
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(event_obj) = arr.get(2) {
                if event_obj.get("kind").and_then(|k| k.as_u64()) == Some(0) {
                    if let Some(content) = event_obj.get("content").and_then(|c| c.as_str()) {
                        return serde_json::from_str(content).ok();
                    }
                }
            }
        }
    }
    None
}

/// Parse events from relay response.
///
/// The response is a JSON array of frame strings from the host.
fn parse_events_from_relay_response(response: &str) -> Vec<NoteInfo> {
    let frames = parse_frames_from_response(response);
    let mut notes = Vec::new();
    for line in frames.iter() {
        let line = line.trim();
        if !line.starts_with("['EVENT") && !line.starts_with("[\"EVENT") {
            continue;
        }
        if let Ok(arr) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(event_obj) = arr.get(2) {
                notes.push(NoteInfo {
                    event_id: event_obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    pubkey: event_obj
                        .get("pubkey")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    created_at: event_obj
                        .get("created_at")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    content: event_obj
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tags: event_obj
                        .get("tags")
                        .and_then(|t| serde_json::from_value(t.clone()).ok())
                        .unwrap_or_default(),
                });
            }
        }
    }
    notes
}

export!(NostrTool);
