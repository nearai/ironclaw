//! Integration tests for `DeferredBusyDrainObserver`.
//!
//! These tests drive the drain observer end-to-end by wiring the concrete
//! composition together at the product-workflow level — using the
//! in-memory implementations of every collaborator — and asserting the
//! observable thread-state changes.
//!
//! # Scenario A — happy path (cascade)
//! 1. Submit message A → run A `Queued` (thread lock held).
//! 2. Accept message B → coordinator returns `ThreadBusy` → mark `DeferredBusy`.
//! 3. Cancel run A → terminal event fires → drain observer resubmits B.
//! 4. Assert message B status is now `Submitted`.
//!
//! # Scenario B — idempotency
//! 1. Perform steps 1–3 from Scenario A so message B is already `Submitted`.
//! 2. Fire the terminal event a second time.
//! 3. Assert message B is still `Submitted` (coordinator idempotency key prevents
//!    double submission and `mark_message_submitted` is idempotent on status).
//!
//! NOTE: Because the observer's `new_unbound` / `bind_coordinator` are
//! `pub(crate)`, the tests must live inside the crate.  The actual
//! `#[tokio::test]` impls are in:
//!   `crates/ironclaw_reborn_composition/src/deferred_busy_drain.rs`
//!   (inline `#[cfg(test)] mod tests { ... }`)
//!
//! This file intentionally left as a no-op redirect to keep the test
//! surface discoverable.
