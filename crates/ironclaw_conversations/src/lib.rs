//! Conversation binding and session-thread contracts for IronClaw Reborn.
//!
//! This crate is the adapter-safe boundary between product/channel adapters and
//! `ironclaw_turns::TurnCoordinator`. It resolves external actor/conversation
//! identifiers into canonical tenant/thread/message/binding references without
//! asking the turn coordinator to parse raw channel payloads or store message
//! content.
//!
//! Durable persistence is provided by [`FilesystemConversationStateStore`]
//! over a [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The
//! `RootFilesystem` choice (libSQL-backed, PostgreSQL-backed, in-memory, or
//! local-disk) is made at the filesystem layer — this crate's state store
//! is structurally one impl over many backends, matching the migration
//! pattern in
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.

mod error;
mod filesystem_store;
mod ids;
mod inbound;
#[cfg(feature = "libsql")]
mod libsql;
mod memory;
#[cfg(feature = "postgres")]
mod postgres;
mod state_store;
mod traits;
mod types;

pub use error::InboundTurnError;
pub use filesystem_store::{
    FilesystemConversationStateStore, RebornFilesystemConversationServices,
};
pub use ids::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, ExternalConversationIdentity,
    ExternalConversationRef, ExternalEventId, InboundMessageContentRef,
};
pub use inbound::InboundTurnService;
#[cfg(feature = "libsql")]
pub use libsql::{RebornLibSqlConversationServices, RebornLibSqlConversationStateStore};
pub use memory::InMemoryConversationServices;
#[cfg(feature = "postgres")]
pub use postgres::{RebornPostgresConversationServices, RebornPostgresConversationStateStore};
pub use traits::{ConversationBindingService, ConversationBindingServiceExt, SessionThreadService};
pub use types::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AcceptedInboundMessageReplay, ConversationBindingResolution, ConversationRouteKind,
    InboundTurnRequest, InboundTurnResponse, LinkConversationRequest, LinkedConversationBinding,
    MessageIdempotencyStatus, ReplyTargetBinding, ResolveConversationRequest, ThreadAccessDecision,
    ThreadMessageRecord, ValidateReplyTargetRequest,
};
