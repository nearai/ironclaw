//! Slack WASM Tool for IronClaw.
//!
//! This is a standalone WASM component that provides Slack integration.
//! It demonstrates how to build external tools that can be dynamically
//! loaded by the agent runtime.
//!
//! # Capabilities Required
//!
//! - HTTP: `slack.com/api/*` (GET, POST)
//! - Secrets: `slack_bot_token` (injected automatically)
//!
//! # Supported Actions
//!
//! - `send_message`: Send a message to a channel
//! - `list_channels`: List channels the bot has access to
//! - `get_channel_history`: Get recent messages from a channel
//! - `post_reaction`: Add an emoji reaction to a message
//! - `get_user_info`: Get information about a Slack user
//!
//! # Example Usage
//!
//! ```json
//! {"action": "send_message", "channel": "#general", "text": "Hello from the agent!"}
//! ```

mod api;
mod types;

use types::SlackAction;

// Generate bindings from the WIT interface.
// This creates the `bindings` module with types and traits.
wit_bindgen::generate!({
    world: "sandboxed-tool",
    path: "../../../wit/tool.wit",
});

/// Implementation of the tool interface.
struct SlackTool;

impl exports::near::agent::tool::Guest for SlackTool {
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
        // JSON Schema for the tool's parameters
        r#"{
            "type": "object",
            "required": ["action"],
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "send_message" },
                        "channel": {
                            "type": "string",
                            "description": "Channel ID or name (e.g., '#general' or 'C1234567890')"
                        },
                        "text": {
                            "type": "string",
                            "description": "Message text (supports Slack mrkdwn formatting)"
                        },
                        "thread_ts": {
                            "type": "string",
                            "description": "Optional thread timestamp to reply in a thread"
                        }
                    },
                    "required": ["action", "channel", "text"]
                },
                {
                    "properties": {
                        "action": { "const": "list_channels" },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of channels to return (default: 100)",
                            "default": 100
                        }
                    },
                    "required": ["action"]
                },
                {
                    "properties": {
                        "action": { "const": "get_channel_history" },
                        "channel": {
                            "type": "string",
                            "description": "Channel ID (e.g., 'C1234567890')"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of messages to return (default: 20)",
                            "default": 20
                        }
                    },
                    "required": ["action", "channel"]
                },
                {
                    "properties": {
                        "action": { "const": "post_reaction" },
                        "channel": {
                            "type": "string",
                            "description": "Channel ID containing the message"
                        },
                        "timestamp": {
                            "type": "string",
                            "description": "Timestamp of the message to react to"
                        },
                        "emoji": {
                            "type": "string",
                            "description": "Emoji name without colons (e.g., 'thumbsup')"
                        }
                    },
                    "required": ["action", "channel", "timestamp", "emoji"]
                },
                {
                    "properties": {
                        "action": { "const": "get_user_info" },
                        "user_id": {
                            "type": "string",
                            "description": "User ID (e.g., 'U1234567890')"
                        }
                    },
                    "required": ["action", "user_id"]
                }
            ]
        }"#
        .to_string()
    }

    fn description() -> String {
        "Slack integration tool for sending messages, listing channels, reading history, \
         adding reactions, and getting user information. Requires a Slack bot token with \
         appropriate scopes (chat:write, channels:read, channels:history, reactions:write, \
         users:read)."
            .to_string()
    }
}

/// Inner execution logic with proper error handling.
fn execute_inner(params: &str) -> Result<String, String> {
    // Check if the Slack token is configured
    if !crate::near::agent::host::secret_exists("slack_bot_token") {
        return Err(
            "Slack bot token not configured. Please add the 'slack_bot_token' secret.".to_string(),
        );
    }

    // Parse the action from JSON
    let action: SlackAction =
        serde_json::from_str(params).map_err(|e| format!("Invalid parameters: {}", e))?;

    crate::near::agent::host::log(
        crate::near::agent::host::LogLevel::Info,
        &format!("Executing Slack action: {:?}", action),
    );

    // Dispatch to the appropriate handler
    let result = match action {
        SlackAction::SendMessage {
            channel,
            text,
            thread_ts,
        } => {
            let result = api::send_message(&channel, &text, thread_ts.as_deref())?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        SlackAction::ListChannels { limit } => {
            let result = api::list_channels(limit)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        SlackAction::GetChannelHistory { channel, limit } => {
            let result = api::get_channel_history(&channel, limit)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        SlackAction::PostReaction {
            channel,
            timestamp,
            emoji,
        } => {
            let result = api::post_reaction(&channel, &timestamp, &emoji)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }

        SlackAction::GetUserInfo { user_id } => {
            let result = api::get_user_info(&user_id)?;
            serde_json::to_string(&result).map_err(|e| e.to_string())?
        }
    };

    Ok(result)
}

// Export the tool implementation.
export!(SlackTool);
