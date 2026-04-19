//! Multi-channel input system.
//!
//! Channels receive messages from external sources (CLI, HTTP, etc.)
//! and convert them to a unified message format for the agent to process.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         ChannelManager                              │
//! │                                                                     │
//! │   ┌──────────────┐   ┌─────────────┐   ┌─────────────┐             │
//! │   │ ReplChannel  │   │ HttpChannel │   │ WasmChannel │   ...       │
//! │   └──────┬───────┘   └──────┬──────┘   └──────┬──────┘             │
//! │          │                 │                 │                      │
//! │          └─────────────────┴─────────────────┘                      │
//! │                            │                                        │
//! │                   select_all (futures)                              │
//! │                            │                                        │
//! │                            ▼                                        │
//! │                     MessageStream                                   │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # WASM Channels
//!
//! WASM channels allow dynamic loading of channel implementations at runtime.
//! See the [`wasm`] module for details.

mod channel;
mod http;
mod manager;
pub mod relay;
mod repl;
mod signal;
#[cfg(unix)]
mod unix_socket_repl;
pub mod wasm;
pub mod web;
mod webhook_server;
mod xmpp;

pub use channel::{
    AttachmentKind, Channel, ChannelSecretUpdater, IncomingAttachment, IncomingMessage,
    MessageStream, OutgoingResponse, StatusUpdate, ToolDecision, routing_target_from_metadata,
};
pub use http::{HttpChannel, HttpChannelState};
pub use manager::ChannelManager;
pub use repl::ReplChannel;
pub use signal::SignalChannel;
#[cfg(unix)]
pub use unix_socket_repl::UnixSocketReplChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};
pub use xmpp::XmppChannel;
