//! Session event transport internals shared by every WebUI event stream.
//!
//! Two seams live here, per the 2026-08-13 session-transport design:
//!
//! - [`driver::ProductStreamDriver`] opens, resumes, drains, and cancels one
//!   typed `ProductSurface::stream_events` subscription, owning the lifetime
//!   budget, idle-poll cadence, and cursor advance.
//! - [`codec`] renders typed product stream events into the redacted
//!   browser-safe frame vocabulary.
//!
//! The compatibility per-thread SSE route and the app-wide session WebSocket
//! both ride these two modules, so the two transports cannot drift on event
//! shape or resume behavior. Transport framing (SSE `id:`/`event:` lines,
//! WebSocket control frames, capacity slots, socket backpressure) stays with
//! each transport handler; selector authorization, replay, rebase, redaction,
//! and lag stay behind the product stream interface.

pub(crate) mod codec;
pub(crate) mod driver;
pub(crate) mod protocol;
