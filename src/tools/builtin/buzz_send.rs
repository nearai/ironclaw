//! Buzz channel messaging tool.
//!
//! Sends messages to Buzz channels via `buzz messages send` CLI.
//! Uses env vars BUZZ_RELAY_URL, BUZZ_PRIVATE_KEY, BUZZ_AUTH_TAG
//! set by buzz-acp when spawning the agent subprocess.

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// Tool for sending messages to Buzz channels.
pub struct BuzzSendTool;

#[async_trait]
impl Tool for BuzzSendTool {
    fn name(&self) -> &str {
        "buzz_send"
    }

    fn description(&self) -> &str {
        "Send a message to a Buzz channel. \
         Use this to publish your response or send a message into the channel. \
         Supports --reply-to for threading and --mention for pubkeys. \
         The channel UUID, reply-to event ID, and other context are available \
         in the [Context] block of the user's message."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "description": "Channel UUID to send the message to."
                },
                "content": {
                    "type": "string",
                    "description": "Message content to send."
                },
                "reply_to": {
                    "type": "string",
                    "description": "Event ID to reply to (for threading). Use the --reply-to value from the [Context] block."
                },
                "mention": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of npub hex pubkeys to mention in the message."
                }
            },
            "required": ["channel", "content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let channel = params
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("channel is required".into()))?;

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("content is required".into()))?;

        if content.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "content must not be empty".into(),
            ));
        }

        let reply_to = params
            .get("reply_to")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let mentions: Vec<String> = params
            .get("mention")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Check env vars
        if std::env::var("BUZZ_RELAY_URL").is_err()
            || std::env::var("BUZZ_PRIVATE_KEY").is_err()
        {
            return Ok(ToolOutput::success(
                serde_json::Value::String(
                    "Error: BUZZ_RELAY_URL or BUZZ_PRIVATE_KEY not set. Cannot send to Buzz channel.".into(),
                ),
                start.elapsed(),
            ));
        }

        let mut cmd = std::process::Command::new("buzz");
        cmd.arg("messages")
            .arg("send")
            .arg("--channel")
            .arg(channel);

        if let Some(event_id) = reply_to {
            cmd.arg("--reply-to").arg(event_id);
        }

        for pubkey in &mentions {
            cmd.arg("--mention").arg(pubkey);
        }

        cmd.arg("--content").arg(content);

        match cmd.output() {
            Ok(output) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Ok(ToolOutput::success(
                        serde_json::Value::String(format!(
                            "Message sent to channel {}: {}",
                            channel,
                            stdout.trim()
                        )),
                        start.elapsed(),
                    ))
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Ok(ToolOutput::success(
                        serde_json::Value::String(format!(
                            "Failed to send message (exit {}): {}",
                            output.status,
                            stderr.trim()
                        )),
                        start.elapsed(),
                    ))
                }
            }
            Err(e) => Ok(ToolOutput::success(
                serde_json::Value::String(format!(
                    "Failed to run buzz messages send: {}",
                    e
                )),
                start.elapsed(),
            )),
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Never
    }
}
