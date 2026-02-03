//! Multi-channel input system.
//!
//! Channels receive messages from external sources (CLI, HTTP, etc.)
//! and convert them to a unified message format for the agent to process.

mod channel;
pub mod cli;
mod http;
mod manager;
mod repl;

pub use channel::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
pub use cli::{AppEvent, TuiChannel};
pub use http::HttpChannel;
pub use manager::ChannelManager;
pub use repl::ReplChannel;
