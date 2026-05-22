//! Adapter/UI-safe approval and auth interaction services for IronClaw Reborn.
//!
//! `ironclaw_interactions` translates scoped blocked run-state into redacted
//! product-facing summaries and routes user decisions back to the canonical
//! Reborn resolution paths. It composes — never replaces — the durable run-state
//! / approval / auth-flow stores.
//!
//! See [`approval`] for the approval surface and [`auth`] for the auth surface.
//! Both surfaces:
//!
//! - return only redacted DTOs that omit raw tool input, approval reasons,
//!   invocation fingerprints, lease IDs, host paths, secrets, and runtime
//!   output;
//! - validate tenant/user/agent/project/thread scope on every read/write;
//! - route resolution through a typed decision/flow port — products may
//!   never reach the durable stores or the runtime/capability layer
//!   directly.

pub mod approval;
