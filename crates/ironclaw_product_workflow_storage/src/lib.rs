//! Durable storage implementations for the Reborn product workflow.
//!
//! Backs the `IdempotencyLedger` port with a single
//! [`FilesystemIdempotencyLedger`] written against the universal
//! `RootFilesystem` surface. The version returned by `RootFilesystem::put`
//! is the natural ownership token for the saga state machine, so the stale
//! `action_id` race that produced two separate per-backend SQL fixes
//! (libSQL + Postgres) becomes structurally impossible — CAS at every
//! transition (`begin_or_replay` reclaim, `settle`, `release`) is what the
//! `ironclaw_filesystem` invariant ("CAS is the floor") explicitly
//! requires for claim/consume/transition operations.
//!
//! The conversation-binding port is owned by the shared
//! `ProductConversationBindingService` in `ironclaw_product_workflow`
//! (PR #3727); its durable state lives in `ironclaw_conversations`'s
//! filesystem store over the unified-FS dispatch fabric (PR #3679). The
//! outbound state store uses `FilesystemOutboundStateStore` from
//! `ironclaw_outbound`. With the ledger now on the same fabric, no
//! per-table SQL schema is needed for product workflow persistence.
//!
//! This crate intentionally does not own:
//! - Workflow orchestration (lives in `ironclaw_product_workflow`).
//! - Conversation binding / external-actor pairing (lives in
//!   `ironclaw_conversations` + the shared `ProductConversationBindingService`).
//! - Adapter contracts (live in `ironclaw_product_adapters`).
//! - Outbound delivery state (lives in `ironclaw_outbound`).
//!
//! Its single responsibility is mapping the workflow ledger port onto the
//! universal filesystem fabric plus the outbound state-store delivery
//! sink used by the runner.

#![forbid(unsafe_code)]

mod error;
mod recovery;

pub use recovery::DEFAULT_RECOVERY_LEASE;

mod ledger_filesystem;
mod outbound_sink;

pub use ledger_filesystem::FilesystemIdempotencyLedger;
pub use outbound_sink::OutboundStateStoreDeliverySink;
