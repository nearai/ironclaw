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
mod repl;
mod signal;
pub mod wasm;
pub mod web;
mod webhook_server;

pub use channel::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, ReasoningDecisionUpdate,
    StatusUpdate,
};
pub use http::HttpChannel;
pub use manager::ChannelManager;
pub use repl::ReplChannel;
pub use signal::SignalChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};
