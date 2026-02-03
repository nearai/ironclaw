//! Multi-channel input system.
//!
//! Channels receive messages from external sources (CLI, Slack, Telegram, HTTP)
//! and convert them to a unified message format for the agent to process.

mod channel;
pub mod cli;
mod http;
mod manager;
mod slack;
mod telegram;

pub use channel::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
pub use cli::{SimpleCliChannel as CliChannel, TuiChannel};
pub use http::HttpChannel;
pub use manager::ChannelManager;
pub use slack::SlackChannel;
pub use telegram::TelegramChannel;
