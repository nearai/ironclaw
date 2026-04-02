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

mod attachments;
mod channel;
mod http;
mod manager;
pub mod relay;
mod repl;
mod signal;
#[cfg(feature = "tui")]
pub mod tui;
mod matrix;
pub mod wasm;
pub mod web;
mod webhook_server;

#[cfg(feature = "tui")]
pub use self::tui::TuiChannel;
pub(crate) use attachments::{
    MAX_INLINE_ATTACHMENT_BYTES, MAX_INLINE_ATTACHMENTS, MAX_INLINE_TOTAL_ATTACHMENT_BYTES,
    attachment_extension_for_mime,
};
pub use channel::{
    AttachmentKind, Channel, ChannelSecretUpdater, ChatApprovalPrompt, EngineThreadSummary,
    HistoryMessage, IncomingAttachment, IncomingMessage, MessageStream, OutgoingResponse,
    StatusUpdate, ThreadSummary, ToolDecision, routing_target_from_metadata,
};
pub use http::{HttpChannel, HttpChannelState};
pub use manager::ChannelManager;
pub use repl::ReplChannel;
pub use signal::SignalChannel;
pub use matrix::MatrixChannel;
pub use web::GatewayChannel;
pub use webhook_server::{WebhookServer, WebhookServerConfig};
