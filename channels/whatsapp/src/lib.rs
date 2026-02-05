//! WhatsApp Channel for near-agent
//!
//! Implements the channel interface for WhatsApp Cloud API.
//! Handles incoming webhooks and sends responses via the API.

use serde::{Deserialize, Serialize};

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use exports::near::agent::channel::*;
use near::agent::channel_host::*;

struct WhatsAppChannel;

impl Guest for WhatsAppChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        log(LogLevel::Info, &format!("WhatsApp channel starting with config: {}", config_json));

        Ok(ChannelConfig {
            display_name: "WhatsApp".to_string(),
            http_endpoints: vec![
                HttpEndpointConfig {
                    path: "/webhook/whatsapp".to_string(),
                    methods: vec!["GET".to_string(), "POST".to_string()],
                    require_secret: true,
                },
            ],
            poll: None, // WhatsApp uses webhooks, not polling
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        log(LogLevel::Debug, &format!("Received {} request to {}", req.method, req.path));

        // Handle webhook verification (GET request)
        if req.method == "GET" {
            return handle_verification(&req);
        }

        // Handle incoming messages (POST request)
        if req.method == "POST" {
            return handle_incoming_message(&req);
        }

        // Method not allowed
        OutgoingHttpResponse {
            status: 405,
            headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
            body: b"Method not allowed".to_vec(),
        }
    }

    fn on_poll() {
        // WhatsApp uses webhooks, no polling needed
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        log(LogLevel::Info, &format!("Sending response to WhatsApp: {}", response.message_id));

        // Parse metadata to get phone number
        let metadata: ResponseMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Build WhatsApp API request
        let phone_number_id = metadata.phone_number_id.ok_or("Missing phone_number_id")?;
        let recipient = metadata.recipient.ok_or("Missing recipient")?;

        let api_url = format!(
            "https://graph.facebook.com/v18.0/{}/messages",
            phone_number_id
        );

        let request_body = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": recipient,
            "type": "text",
            "text": {
                "body": response.content
            }
        });

        let body_bytes = serde_json::to_vec(&request_body)
            .map_err(|e| format!("Failed to serialize request: {}", e))?;

        // Make API request (host will inject the access token)
        let result = http_request(
            "POST",
            &api_url,
            r#"{"Content-Type": "application/json"}"#,
            Some(&body_bytes),
        );

        match result {
            Ok(resp) if resp.status >= 200 && resp.status < 300 => {
                log(LogLevel::Info, "Message sent successfully");
                Ok(())
            }
            Ok(resp) => {
                let body_str = String::from_utf8_lossy(&resp.body);
                Err(format!("WhatsApp API error {}: {}", resp.status, body_str))
            }
            Err(e) => Err(format!("HTTP request failed: {}", e)),
        }
    }

    fn on_shutdown() {
        log(LogLevel::Info, "WhatsApp channel shutting down");
    }
}

/// Handle WhatsApp webhook verification request
fn handle_verification(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse query parameters
    let query: serde_json::Value = serde_json::from_str(&req.query_json)
        .unwrap_or(serde_json::Value::Null);

    let mode = query.get("hub.mode").and_then(|v| v.as_str());
    let challenge = query.get("hub.challenge").and_then(|v| v.as_str());

    // WhatsApp sends hub.mode=subscribe for verification
    if mode == Some("subscribe") {
        if let Some(challenge) = challenge {
            log(LogLevel::Info, "Webhook verification successful");
            return OutgoingHttpResponse {
                status: 200,
                headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
                body: challenge.as_bytes().to_vec(),
            };
        }
    }

    OutgoingHttpResponse {
        status: 403,
        headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
        body: b"Verification failed".to_vec(),
    }
}

/// Handle incoming WhatsApp message
fn handle_incoming_message(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse webhook payload
    let payload: WebhookPayload = match serde_json::from_slice(&req.body) {
        Ok(p) => p,
        Err(e) => {
            log(LogLevel::Warn, &format!("Failed to parse webhook payload: {}", e));
            return OutgoingHttpResponse {
                status: 400,
                headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
                body: b"Invalid payload".to_vec(),
            };
        }
    };

    // Process each entry
    for entry in payload.entry.iter() {
        for change in entry.changes.iter() {
            if change.field != "messages" {
                continue;
            }

            let value = &change.value;
            let phone_number_id = value.metadata.phone_number_id.clone();

            // Process messages
            for message in value.messages.iter() {
                // Only handle text messages for now
                if message.r#type != "text" {
                    continue;
                }

                let text = message.text.as_ref().map(|t| t.body.clone()).unwrap_or_default();
                let from = message.from.clone();
                let msg_id = message.id.clone();

                // Build metadata for response routing
                let metadata = MessageMetadata {
                    phone_number_id: phone_number_id.clone(),
                    message_id: msg_id.clone(),
                    timestamp: message.timestamp.clone(),
                };

                // Emit message to the agent
                emit_message(&EmittedMessage {
                    user_id: from.clone(),
                    user_name: None, // Could look up contact name
                    content: text,
                    thread_id: None,
                    metadata_json: serde_json::to_string(&metadata).unwrap_or_default(),
                });

                log(LogLevel::Info, &format!("Emitted message from {}", from));
            }
        }
    }

    // Acknowledge receipt
    OutgoingHttpResponse {
        status: 200,
        headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
        body: b"OK".to_vec(),
    }
}

// ==================== WhatsApp API Types ====================

#[derive(Debug, Deserialize)]
struct WebhookPayload {
    entry: Vec<WebhookEntry>,
}

#[derive(Debug, Deserialize)]
struct WebhookEntry {
    changes: Vec<WebhookChange>,
}

#[derive(Debug, Deserialize)]
struct WebhookChange {
    field: String,
    value: WebhookValue,
}

#[derive(Debug, Deserialize)]
struct WebhookValue {
    metadata: WhatsAppMetadata,
    #[serde(default)]
    messages: Vec<WhatsAppMessage>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppMetadata {
    phone_number_id: String,
}

#[derive(Debug, Deserialize)]
struct WhatsAppMessage {
    id: String,
    from: String,
    timestamp: String,
    r#type: String,
    text: Option<WhatsAppText>,
}

#[derive(Debug, Deserialize)]
struct WhatsAppText {
    body: String,
}

#[derive(Debug, Serialize)]
struct MessageMetadata {
    phone_number_id: String,
    message_id: String,
    timestamp: String,
}

#[derive(Debug, Deserialize)]
struct ResponseMetadata {
    phone_number_id: Option<String>,
    recipient: Option<String>,
}

// Export the channel implementation
export!(WhatsAppChannel);
