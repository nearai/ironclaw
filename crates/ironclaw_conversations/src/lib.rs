//! Conversation binding and inbound-message contracts for IronClaw Reborn.
//!
//! This crate is the adapter-safe boundary between product/channel adapters and
//! turn submission. It resolves external actor/conversation identifiers into
//! canonical tenant/thread/message/binding references without asking the turn
//! coordinator to parse raw channel payloads or store message content.
//!
//! It does **not** hold the turn coordinator. The orchestration reaches it
//! through the one-method [`ConversationTurnSubmitter`] port declared in
//! [`turn_submission`], which `ironclaw_reborn_composition` implements over the
//! real handle — so this crate speaks only `ironclaw_host_api::turn` vocabulary
//! plus its own types.
//!
//! **It is not the transcript.** `ironclaw_threads` owns canonical threads and
//! their message content; this crate owns the *binding* from an external
//! conversation to one of them, plus inbound idempotency. The two used to share
//! five type names — including a `SessionThreadService` on each side with the
//! same `accept_inbound_message` signature and a different store behind it —
//! which is why everything here is spelled `…Conversation…`
//! ([`InboundConversationService`], [`AcceptedConversationMessage`],
//! [`ConversationMessageRecord`], …). Keep it that way:
//! `reborn_conversations_threads_attachments.rs` fails on any new shared name.
//!
//! Durable persistence is provided by [`ConversationStateStore`]
//! over a [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). The
//! `RootFilesystem` choice (libSQL-backed, PostgreSQL-backed, in-memory, or
//! local-disk) is made at the filesystem layer — the consumer-store level
//! no longer carries per-backend impls.

mod conversation_state_store;
mod error;
mod ids;
mod inbound;
mod memory;
mod state_store;
mod stored_refs;
mod traits;
mod trusted_trigger;
mod turn_submission;
mod types;

pub use conversation_state_store::{ConversationStateStore, RebornFilesystemConversationServices};
pub use error::InboundTurnError;
// `ExternalActorRef` / `ExternalConversationRef` are deliberately **not**
// re-exported. Their one home is
// `ironclaw_extension_contracts::external`; consumers import them from there,
// so this crate never becomes a second import path for channel vocabulary
// (PROPOSAL §11.2.4, and the rule `reborn_extension_contract_location_scan`
// enforces).
pub use ids::{
    AdapterInstallationId, AdapterKind, ExternalConversationIdentity, ExternalEventId,
    InboundMessageContentRef,
};
pub use inbound::{InboundTurnService, trusted_trigger_fire_submitter};
pub use memory::InMemoryConversationServices;
pub use traits::{
    ConversationActorPairingService, ConversationBindingService, InboundConversationService,
};
pub use turn_submission::{
    ConversationInboundClassification, ConversationTurnSubmission, ConversationTurnSubmitter,
    TurnSubmissionError, TurnSubmissionErrorCategory, TurnSubmissionRetry,
};
pub use types::{
    AcceptConversationMessageRequest, AcceptedConversationMessage,
    AcceptedConversationMessageLookup, AcceptedConversationMessageReplay, ConditionalUnpairOutcome,
    ConversationBindingResolution, ConversationMessageRecord, ConversationRouteKind,
    ExpectedExternalActorOwner, ExternalActorBindingEpoch, InboundTurnRequest, InboundTurnResponse,
    LinkConversationRequest, LinkedConversationBinding, MessageIdempotencyStatus,
    ReplyTargetBinding, ResolveConversationRequest, ResolveStoredReplyTargetRequest,
    StoredReplyTargetAccess, StoredReplyTargetBinding, ThreadAccessDecision,
    ValidateReplyTargetRequest,
};
