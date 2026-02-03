//! Telegram channel integration.
//!
//! TODO: Implement full Telegram bot integration using teloxide.

use async_trait::async_trait;

use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use crate::config::TelegramConfig;
use crate::error::ChannelError;

/// Telegram channel for Telegram bot integration.
pub struct TelegramChannel {
    #[allow(dead_code)]
    config: TelegramConfig,
}

impl TelegramChannel {
    /// Create a new Telegram channel.
    pub fn new(config: TelegramConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // TODO: Implement Telegram long polling or webhook
        // 1. Use teloxide to connect to Telegram Bot API
        // 2. Handle incoming messages
        // 3. Convert to IncomingMessage format
        Err(ChannelError::StartupFailed {
            name: "telegram".to_string(),
            reason: "Telegram channel not yet implemented".to_string(),
        })
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // TODO: Use Telegram Bot API to send message
        // - Reply to the same chat
        // - Support reply_to_message_id for threaded replies
        Err(ChannelError::SendFailed {
            name: "telegram".to_string(),
            reason: "Telegram channel not yet implemented".to_string(),
        })
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Err(ChannelError::HealthCheckFailed {
            name: "telegram".to_string(),
        })
    }
}
