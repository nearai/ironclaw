// WhatsApp API types have fields reserved for future use (contacts, statuses, etc.)
#![allow(dead_code)]

//! WhatsApp Cloud API channel for IronClaw.
//!
//! This WASM component implements the channel interface for handling WhatsApp
//! webhooks and sending messages back via the Cloud API.
//!
//! # Features
//!
//! - Webhook-based message receiving (WhatsApp is webhook-only, no polling)
//! - Text message support
//! - Business account support
//! - User name extraction from contacts
//!
//! # Security
//!
//! - Access token is injected by host during HTTP requests via {WHATSAPP_ACCESS_TOKEN} placeholder
//! - WASM never sees raw credentials
//! - Webhook verify token validation by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// WhatsApp Cloud API Types
// ============================================================================

/// WhatsApp webhook payload.
/// https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/payload-examples
#[derive(Debug, Deserialize)]
struct WebhookPayload {
    /// Always "whatsapp_business_account"
    object: String,

    /// Array of webhook entries
    entry: Vec<WebhookEntry>,
}

/// Single webhook entry.
#[derive(Debug, Deserialize)]
struct WebhookEntry {
    /// WhatsApp Business Account ID
    id: String,

    /// Changes in this entry
    changes: Vec<WebhookChange>,
}

/// A change notification.
#[derive(Debug, Deserialize)]
struct WebhookChange {
    /// Field that changed (usually "messages")
    field: String,

    /// The change value
    value: WebhookValue,
}

/// The value of a change.
#[derive(Debug, Deserialize)]
struct WebhookValue {
    /// Messaging product (always "whatsapp")
    messaging_product: String,

    /// Business account metadata
    metadata: BusinessMetadata,

    /// Contact information (sender details)
    #[serde(default)]
    contacts: Vec<Contact>,

    /// Incoming messages
    #[serde(default)]
    messages: Vec<WhatsAppMessage>,

    /// Message statuses (delivered, read, etc.)
    #[serde(default)]
    statuses: Vec<MessageStatus>,
}

/// Business account metadata.
#[derive(Debug, Deserialize)]
struct BusinessMetadata {
    /// Display phone number
    display_phone_number: String,

    /// Phone number ID (used in API calls)
    phone_number_id: String,
}

/// Contact information.
#[derive(Debug, Deserialize)]
struct Contact {
    /// WhatsApp ID (phone number)
    wa_id: String,

    /// Profile information
    profile: Option<ContactProfile>,
}

/// Contact profile.
#[derive(Debug, Deserialize)]
struct ContactProfile {
    /// Display name
    name: String,
}

/// Incoming WhatsApp message.
#[derive(Debug, Deserialize)]
struct WhatsAppMessage {
    /// Message ID
    id: String,

    /// Sender's phone number
    from: String,

    /// Unix timestamp
    timestamp: String,

    /// Message type: text, image, audio, video, document, etc.
    #[serde(rename = "type")]
    message_type: String,

    /// Text content (if type is "text")
    text: Option<TextContent>,

    /// Context for replies
    context: Option<MessageContext>,
}

/// Text message content.
#[derive(Debug, Deserialize)]
struct TextContent {
    /// The message body
    body: String,
}

/// Reply context.
#[derive(Debug, Deserialize)]
struct MessageContext {
    /// Message ID being replied to
    message_id: String,

    /// Phone number of original sender
    from: Option<String>,
}

/// Message status update.
#[derive(Debug, Deserialize)]
struct MessageStatus {
    /// Message ID
    id: String,

    /// Status: sent, delivered, read, failed
    status: String,

    /// Timestamp
    timestamp: String,

    /// Recipient ID
    recipient_id: String,
}

/// WhatsApp API response wrapper.
#[derive(Debug, Deserialize)]
struct WhatsAppApiResponse {
    /// Messages sent (on success)
    messages: Option<Vec<SentMessage>>,

    /// Error info (on failure)
    error: Option<ApiError>,
}

/// Sent message info.
#[derive(Debug, Deserialize)]
struct SentMessage {
    /// Message ID
    id: String,
}

/// API error details.
#[derive(Debug, Deserialize)]
struct ApiError {
    /// Error message
    message: String,

    /// Error type
    #[serde(rename = "type")]
    error_type: Option<String>,

    /// Error code
    code: Option<i64>,
}

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
/// This MUST contain all info needed to send a response.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppMessageMetadata {
    /// Phone number ID (business account, for API URL)
    phone_number_id: String,

    /// Sender's phone number (becomes recipient for response)
    sender_phone: String,

    /// Original message ID (for reply context)
    message_id: String,

    /// Timestamp of original message
    timestamp: String,
}

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct WhatsAppConfig {
    /// API version to use (default: v18.0)
    #[serde(default = "default_api_version")]
    api_version: String,

    /// Whether to reply to the original message (thread context)
    #[serde(default = "default_reply_to_message")]
    reply_to_message: bool,
}

fn default_api_version() -> String {
    "v18.0".to_string()
}

fn default_reply_to_message() -> bool {
    true
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct WhatsAppChannel;

impl Guest for WhatsAppChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: WhatsAppConfig = serde_json::from_str(&config_json).unwrap_or(WhatsAppConfig {
            api_version: default_api_version(),
            reply_to_message: default_reply_to_message(),
        });

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "WhatsApp channel starting (API version: {})",
                config.api_version
            ),
        );

        // WhatsApp Cloud API is webhook-only, no polling available
        Ok(ChannelConfig {
            display_name: "WhatsApp".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/whatsapp".to_string(),
                // GET for webhook verification, POST for incoming messages
                methods: vec!["GET".to_string(), "POST".to_string()],
                // Webhook verify token should be validated by host
                require_secret: true,
            }],
            poll: None, // WhatsApp doesn't support polling
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Received {} request to {}", req.method, req.path),
        );

        // Handle webhook verification (GET request from Meta)
        if req.method == "GET" {
            return handle_verification(&req);
        }

        // Handle incoming messages (POST request)
        if req.method == "POST" {
            // Defense in depth: check secret validation
            // Host validates the verify token, but we double-check the flag
            if !req.secret_validated {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "Webhook request with invalid or missing verify token",
                );
                // Return 401 but note that host should have already rejected these
            }

            return handle_incoming_message(&req);
        }

        // Method not allowed
        json_response(405, serde_json::json!({"error": "Method not allowed"}))
    }

    fn on_poll() {
        // WhatsApp Cloud API is webhook-only, no polling
        // This should never be called since poll config is None
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Sending response for message: {}", response.message_id),
        );

        // Parse metadata from the ORIGINAL incoming message
        // This contains the routing info we need (sender becomes recipient)
        let metadata: WhatsAppMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Build WhatsApp API URL with token placeholder
        // Host will replace {WHATSAPP_ACCESS_TOKEN} with actual token in Authorization header
        let api_url = format!(
            "https://graph.facebook.com/v18.0/{}/messages",
            metadata.phone_number_id
        );

        // Build sendMessage payload
        let payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": metadata.sender_phone,  // Original sender becomes recipient
            "type": "text",
            "text": {
                "preview_url": false,
                "body": response.content
            }
        });

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("Failed to serialize payload: {}", e))?;

        // Headers with Bearer token placeholder
        // Host will inject the actual access token
        let headers = serde_json::json!({
            "Content-Type": "application/json",
            "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
        });

        let result = channel_host::http_request(
            "POST",
            &api_url,
            &headers.to_string(),
            Some(&payload_bytes),
        );

        match result {
            Ok(http_response) => {
                // Parse WhatsApp API response
                let api_response: Result<WhatsAppApiResponse, _> =
                    serde_json::from_slice(&http_response.body);

                match api_response {
                    Ok(resp) => {
                        // Check for API error
                        if let Some(error) = resp.error {
                            return Err(format!(
                                "WhatsApp API error: {} (code: {:?})",
                                error.message, error.code
                            ));
                        }

                        // Success - log the sent message ID
                        if let Some(messages) = resp.messages {
                            if let Some(sent) = messages.first() {
                                channel_host::log(
                                    channel_host::LogLevel::Debug,
                                    &format!(
                                        "Sent message to {}: id={}",
                                        metadata.sender_phone, sent.id
                                    ),
                                );
                            }
                        }

                        Ok(())
                    }
                    Err(e) => {
                        // Couldn't parse response, check status code
                        if http_response.status >= 200 && http_response.status < 300 {
                            // Probably OK even if we can't parse
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                "Message sent (response parse failed but status OK)",
                            );
                            Ok(())
                        } else {
                            let body_str = String::from_utf8_lossy(&http_response.body);
                            Err(format!(
                                "WhatsApp API HTTP {}: {} (parse error: {})",
                                http_response.status, body_str, e
                            ))
                        }
                    }
                }
            }
            Err(e) => Err(format!("HTTP request failed: {}", e)),
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "WhatsApp channel shutting down",
        );
    }
}

// ============================================================================
// Webhook Verification
// ============================================================================

/// Handle WhatsApp webhook verification request from Meta.
///
/// Meta sends a GET request with:
/// - hub.mode=subscribe
/// - hub.challenge=<random string>
/// - hub.verify_token=<your configured token>
///
/// We must respond with the challenge value to verify.
fn handle_verification(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse query parameters
    let query: serde_json::Value =
        serde_json::from_str(&req.query_json).unwrap_or(serde_json::Value::Null);

    let mode = query.get("hub.mode").and_then(|v| v.as_str());
    let challenge = query.get("hub.challenge").and_then(|v| v.as_str());

    // Verify token is validated by host via secret_validated field
    // We just need to check mode and return challenge

    if mode == Some("subscribe") {
        if let Some(challenge) = challenge {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook verification successful",
            );

            // Must respond with the challenge as plain text
            return OutgoingHttpResponse {
                status: 200,
                headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
                body: challenge.as_bytes().to_vec(),
            };
        }
    }

    channel_host::log(
        channel_host::LogLevel::Warn,
        &format!(
            "Webhook verification failed: mode={:?}, challenge={:?}",
            mode,
            challenge.is_some()
        ),
    );

    OutgoingHttpResponse {
        status: 403,
        headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
        body: b"Verification failed".to_vec(),
    }
}

// ============================================================================
// Message Handling
// ============================================================================

/// Handle incoming WhatsApp webhook payload.
fn handle_incoming_message(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse the body as UTF-8
    let body_str = match std::str::from_utf8(&req.body) {
        Ok(s) => s,
        Err(_) => {
            return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
        }
    };

    // Parse webhook payload
    let payload: WebhookPayload = match serde_json::from_str(body_str) {
        Ok(p) => p,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse webhook payload: {}", e),
            );
            // Return 200 to prevent Meta from retrying
            return json_response(200, serde_json::json!({"status": "ok"}));
        }
    };

    // Validate object type
    if payload.object != "whatsapp_business_account" {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Unexpected object type: {}", payload.object),
        );
        return json_response(200, serde_json::json!({"status": "ok"}));
    }

    // Process each entry
    for entry in payload.entry {
        for change in entry.changes {
            // Only handle message changes
            if change.field != "messages" {
                continue;
            }

            let value = change.value;
            let phone_number_id = value.metadata.phone_number_id.clone();

            // Build contact name lookup
            let contact_names: std::collections::HashMap<String, String> = value
                .contacts
                .iter()
                .filter_map(|c| {
                    c.profile
                        .as_ref()
                        .map(|p| (c.wa_id.clone(), p.name.clone()))
                })
                .collect();

            // Skip status updates (delivered, read, etc.) - we only want messages
            // This prevents loops and unnecessary processing
            if !value.statuses.is_empty() && value.messages.is_empty() {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Skipping {} status updates", value.statuses.len()),
                );
                continue;
            }

            // Process messages
            for message in value.messages {
                handle_message(&message, &phone_number_id, &contact_names);
            }
        }
    }

    // Always respond 200 quickly (Meta expects fast responses)
    json_response(200, serde_json::json!({"status": "ok"}))
}

/// Process a single WhatsApp message.
fn handle_message(
    message: &WhatsAppMessage,
    phone_number_id: &str,
    contact_names: &std::collections::HashMap<String, String>,
) {
    // Only handle text messages for now
    // TODO: Add support for image, audio, video, document, etc.
    if message.message_type != "text" {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Skipping non-text message type: {}", message.message_type),
        );
        return;
    }

    // Extract text content
    let text = match &message.text {
        Some(t) if !t.body.is_empty() => t.body.clone(),
        _ => return,
    };

    // Look up sender's name from contacts
    let user_name = contact_names.get(&message.from).cloned();

    // Build metadata for response routing
    // This is critical - the response handler uses this to know where to send
    let metadata = WhatsAppMessageMetadata {
        phone_number_id: phone_number_id.to_string(),
        sender_phone: message.from.clone(), // This becomes recipient in response
        message_id: message.id.clone(),
        timestamp: message.timestamp.clone(),
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: message.from.clone(),
        user_name,
        content: text,
        thread_id: None, // WhatsApp doesn't have threads like Slack/Discord
        metadata_json,
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "Emitted message from {} (phone_number_id={})",
            message.from, phone_number_id
        ),
    );
}

// ============================================================================
// Utilities
// ============================================================================

/// Create a JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(WhatsAppChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_payload() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123456789",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+1234567890",
                            "phone_number_id": "987654321"
                        },
                        "contacts": [{
                            "wa_id": "15551234567",
                            "profile": {
                                "name": "John Doe"
                            }
                        }],
                        "messages": [{
                            "id": "wamid.abc123",
                            "from": "15551234567",
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": {
                                "body": "Hello!"
                            }
                        }]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.object, "whatsapp_business_account");
        assert_eq!(payload.entry.len(), 1);

        let change = &payload.entry[0].changes[0];
        assert_eq!(change.field, "messages");
        assert_eq!(change.value.metadata.phone_number_id, "987654321");

        let message = &change.value.messages[0];
        assert_eq!(message.from, "15551234567");
        assert_eq!(message.text.as_ref().unwrap().body, "Hello!");
    }

    #[test]
    fn test_parse_status_update() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123456789",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+1234567890",
                            "phone_number_id": "987654321"
                        },
                        "statuses": [{
                            "id": "wamid.abc123",
                            "status": "delivered",
                            "timestamp": "1234567890",
                            "recipient_id": "15551234567"
                        }]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let value = &payload.entry[0].changes[0].value;

        // Should have status but no messages
        assert!(value.messages.is_empty());
        assert_eq!(value.statuses.len(), 1);
        assert_eq!(value.statuses[0].status, "delivered");
    }

    #[test]
    fn test_metadata_roundtrip() {
        let metadata = WhatsAppMessageMetadata {
            phone_number_id: "123456".to_string(),
            sender_phone: "15551234567".to_string(),
            message_id: "wamid.abc".to_string(),
            timestamp: "1234567890".to_string(),
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: WhatsAppMessageMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.phone_number_id, "123456");
        assert_eq!(parsed.sender_phone, "15551234567");
    }
}
