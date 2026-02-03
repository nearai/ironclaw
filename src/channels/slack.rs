//! Slack channel integration.
//!
//! TODO: Implement full Slack bot integration using slack-morphism.

use async_trait::async_trait;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::config::SlackConfig;
use crate::error::ChannelError;

/// Slack channel for Slack bot integration.
pub struct SlackChannel {
    #[allow(dead_code)]
    config: SlackConfig,
}

impl SlackChannel {
    /// Create a new Slack channel.
    pub fn new(config: SlackConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // TODO: Implement Slack socket mode connection
        // 1. Connect via Slack Socket Mode
        // 2. Listen for app_mention and direct message events
        // 3. Convert Slack events to IncomingMessage
        Err(ChannelError::StartupFailed {
            name: "slack".to_string(),
            reason: "Slack channel not yet implemented".to_string(),
        })
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // TODO: Use Slack Web API to post message
        // - If in thread, reply in thread
        // - Support blocks for rich formatting
        Err(ChannelError::SendFailed {
            name: "slack".to_string(),
            reason: "Slack channel not yet implemented".to_string(),
        })
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Err(ChannelError::HealthCheckFailed {
            name: "slack".to_string(),
        })
    }
}
