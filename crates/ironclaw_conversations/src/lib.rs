//! Conversation binding and session-thread contracts for IronClaw Reborn.
//!
//! This crate is the adapter-safe boundary between product/channel adapters and
//! `ironclaw_turns::TurnCoordinator`. It resolves external actor/conversation
//! identifiers into canonical tenant/thread/message/binding references without
//! asking the turn coordinator to parse raw channel payloads or store message
//! content.

mod error;
mod ids;
mod inbound;
mod memory;
mod traits;
mod types;

pub use error::InboundTurnError;
pub use ids::{
    AdapterInstallationId, AdapterKind, ExternalActorRef, ExternalConversationIdentity,
    ExternalConversationRef, ExternalEventId, InboundMessageContentRef,
};
pub use inbound::InboundTurnService;
pub use memory::InMemoryConversationServices;
pub use traits::{ConversationBindingService, ConversationBindingServiceExt, SessionThreadService};
pub use types::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, ConversationBindingResolution,
    ConversationRouteKind, InboundTurnRequest, InboundTurnResponse, LinkConversationRequest,
    LinkedConversationBinding, MessageIdempotencyStatus, ReplyTargetBinding,
    ResolveConversationRequest, ThreadAccessDecision, ThreadMessageRecord,
};
