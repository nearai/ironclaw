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
//! local-disk) is made at the filesystem layer — the consumer-store level
//! no longer carries per-backend impls.

mod error;
mod filesystem_store;
mod ids;
mod inbound;
mod memory;
mod state_store;
mod traits;
mod trusted_trigger;
mod types;

pub use error::InboundTurnError;
pub use filesystem_store::{
    FilesystemConversationStateStore, RebornFilesystemConversationServices,
};
pub use ids::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, ExternalConversationIdentity,
    ExternalConversationRef, ExternalEventId, InboundMessageContentRef,
};
pub use inbound::{InboundTurnService, trusted_trigger_fire_submitter};
pub use memory::InMemoryConversationServices;
pub use traits::{
    ConversationActorPairingService, ConversationBindingService, ConversationBindingServiceExt,
    SessionThreadService,
};
pub use types::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageLookup,
    AcceptedInboundMessageReplay, ConversationBindingResolution, ConversationRouteKind,
    InboundTurnRequest, InboundTurnResponse, LinkConversationRequest, LinkedConversationBinding,
    MessageIdempotencyStatus, ReplyTargetBinding, ResolveConversationRequest, ThreadAccessDecision,
    ThreadMessageRecord, ValidateReplyTargetRequest,
};
