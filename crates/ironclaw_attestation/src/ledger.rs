//! Signing / broadcast idempotency ledger.
//!
//! One ledger row per `gate_ref`, created at [`SigningLedgerState::Approved`]
//! and advanced through a strict state machine. The machine encodes the
//! broadcast-idempotency guard that prevents re-signing or double-submitting a
//! transaction: once a row reaches [`SigningLedgerState::BroadcastSubmitted`]
//! it may only move to a terminal state, NEVER back to `Signing`/`Signed`.
//! This holds even under a `Stuck -> InProgress` job recovery — recovery sees
//! the broadcast already submitted and cannot re-sign with a fresh
//! nonce/blockhash.
//!
//! Durable PG / libSQL backends are stacked follow-ups gated by the canonical
//! [`signing_ledger_contract_cases!`] suite.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use ironclaw_signing_provider::GateRef;

/// State of a single signing/broadcast flow, keyed by `gate_ref`.
///
/// Wire-stable, snake_case serde (see `.claude/rules/types.md`). The legal
/// forward path is:
///
/// ```text
/// Approved -> Signing -> Signed -> BroadcastSubmitted -> Finalized
///                                                      \-> Unknown
///                                                      \-> ManualReview
/// ```
///
/// `Finalized`, `Unknown`, and `ManualReview` are terminal. `Unknown` and
/// `ManualReview` are NEVER auto-retried with a fresh nonce/blockhash — they
/// require out-of-band resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningLedgerState {
    /// The transaction has been approved at the gate; signing not yet started.
    Approved,
    /// Signing is in progress.
    Signing,
    /// The transaction is signed but not yet broadcast.
    Signed,
    /// The signed transaction has been submitted to the network. Past this
    /// point re-signing is forbidden (broadcast-idempotency guard).
    BroadcastSubmitted,
    /// Confirmed on-chain. Terminal.
    Finalized,
    /// Broadcast outcome is unknown (e.g. submit timed out). Terminal; needs
    /// out-of-band resolution, never an automatic fresh-nonce retry.
    Unknown,
    /// Flagged for human resolution. Terminal.
    ManualReview,
}

impl SigningLedgerState {
    /// Whether this state is terminal (no further transitions allowed).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            SigningLedgerState::Finalized
                | SigningLedgerState::Unknown
                | SigningLedgerState::ManualReview
        )
    }

    /// Whether the transaction has been broadcast (i.e. is at or past
    /// [`SigningLedgerState::BroadcastSubmitted`]).
    pub fn is_broadcast(self) -> bool {
        matches!(
            self,
            SigningLedgerState::BroadcastSubmitted
                | SigningLedgerState::Finalized
                | SigningLedgerState::Unknown
                | SigningLedgerState::ManualReview
        )
    }

    /// Validate a transition from `self` to `to`.
    ///
    /// Encodes: the single legal forward edge between non-broadcast states, the
    /// fan-out from `BroadcastSubmitted` to the three terminals, no regression,
    /// no skipping, and the broadcast-idempotency guard (a broadcast row can
    /// only reach a terminal).
    pub fn can_advance_to(self, to: SigningLedgerState) -> bool {
        use SigningLedgerState::*;
        match self {
            Approved => to == Signing,
            Signing => to == Signed,
            Signed => to == BroadcastSubmitted,
            BroadcastSubmitted => matches!(to, Finalized | Unknown | ManualReview),
            // Terminal states never advance.
            Finalized | Unknown | ManualReview => false,
        }
    }
}

/// Errors a [`SigningLedger`] can surface.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    /// The requested transition is not permitted by the state machine.
    #[error("invalid signing-ledger transition from {from:?} to {to:?}")]
    InvalidTransition {
        /// Current state.
        from: SigningLedgerState,
        /// Attempted target state.
        to: SigningLedgerState,
    },

    /// A concurrent advance won the conditional-update (CAS) race: the row's
    /// observed `from` state changed between the read and the conditional
    /// `UPDATE`, so this advance matched zero rows. Distinct from
    /// [`LedgerError::InvalidTransition`], which means the *caller's* requested
    /// move is illegal for the current state; a lost CAS means another writer
    /// moved the row first (e.g. a `Stuck -> InProgress` recovery racing the
    /// original turn). Durable backends only — the in-memory ledger performs
    /// read-validate-write under one lock and can never lose this race.
    #[error(
        "signing-ledger advance lost a concurrent CAS race (observed {observed:?}, target {to:?})"
    )]
    ConcurrentAdvance {
        /// State observed when the lost-CAS path re-read the row.
        observed: SigningLedgerState,
        /// Target state this advance attempted.
        to: SigningLedgerState,
    },

    /// No ledger row exists for the given `gate_ref`.
    #[error("no signing-ledger row for this gate_ref")]
    NotFound,

    /// A row already exists for this `gate_ref` (one-shot create).
    #[error("signing-ledger row already exists for this gate_ref")]
    AlreadyExists,

    /// A backend-internal failure with an opaque description.
    #[error("signing-ledger store error: {reason}")]
    Backend {
        /// Human-readable description of the backend failure.
        reason: String,
    },
}

/// Signing/broadcast idempotency ledger, keyed by `gate_ref`.
#[async_trait]
pub trait SigningLedger: Send + Sync {
    /// Create a new ledger row at [`SigningLedgerState::Approved`]. One-shot per
    /// `gate_ref`: a second create fails with [`LedgerError::AlreadyExists`].
    async fn create(&self, gate_ref: &GateRef) -> Result<(), LedgerError>;

    /// Read the current state for `gate_ref`, or [`LedgerError::NotFound`].
    async fn state(&self, gate_ref: &GateRef) -> Result<SigningLedgerState, LedgerError>;

    /// Advance the row for `gate_ref` to `to`, validating the transition.
    /// Fails with [`LedgerError::InvalidTransition`] for any illegal move and
    /// [`LedgerError::NotFound`] if the row does not exist.
    async fn advance(&self, gate_ref: &GateRef, to: SigningLedgerState) -> Result<(), LedgerError>;
}

/// In-memory [`SigningLedger`]. The single [`Mutex`] makes read-validate-write
/// in [`SigningLedger::advance`] a single critical section.
#[derive(Debug, Default)]
pub struct InMemorySigningLedger {
    rows: Mutex<HashMap<GateRef, SigningLedgerState>>,
}

impl InMemorySigningLedger {
    /// Construct an empty ledger.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SigningLedger for InMemorySigningLedger {
    async fn create(&self, gate_ref: &GateRef) -> Result<(), LedgerError> {
        let mut rows = self.rows.lock().map_err(|e| LedgerError::Backend {
            reason: e.to_string(),
        })?;
        if rows.contains_key(gate_ref) {
            return Err(LedgerError::AlreadyExists);
        }
        rows.insert(gate_ref.clone(), SigningLedgerState::Approved);
        Ok(())
    }

    async fn state(&self, gate_ref: &GateRef) -> Result<SigningLedgerState, LedgerError> {
        let rows = self.rows.lock().map_err(|e| LedgerError::Backend {
            reason: e.to_string(),
        })?;
        rows.get(gate_ref).copied().ok_or(LedgerError::NotFound)
    }

    async fn advance(&self, gate_ref: &GateRef, to: SigningLedgerState) -> Result<(), LedgerError> {
        let mut rows = self.rows.lock().map_err(|e| LedgerError::Backend {
            reason: e.to_string(),
        })?;
        let from = rows.get_mut(gate_ref).ok_or(LedgerError::NotFound)?;
        if !from.can_advance_to(to) {
            return Err(LedgerError::InvalidTransition { from: *from, to });
        }
        *from = to;
        Ok(())
    }
}

/// Canonical contract suite for [`SigningLedger`] implementations. Mirrors the
/// grant-store and predicate-state contract pattern.
#[cfg(any(test, feature = "contract-tests"))]
pub mod contract {
    // See the matching note in `grant.rs`: `pub` is for out-of-crate consumers
    // under the `contract-tests` feature; suppress `unreachable_pub` for the
    // crate's own `cargo test` build where the parent module is private.
    #![cfg_attr(not(feature = "contract-tests"), allow(unreachable_pub))]
    use super::*;
    use std::sync::Arc;

    /// The fixed `gate_ref` every ledger contract case operates on.
    pub fn gate() -> GateRef {
        GateRef::new("gate:ledger")
    }

    fn gate_named(name: &str) -> GateRef {
        GateRef::new(name)
    }

    pub async fn full_valid_sequence<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;
        let g = gate();
        ledger.create(&g).await.expect("create");
        assert_eq!(ledger.state(&g).await.expect("state"), Approved);
        for to in [Signing, Signed, BroadcastSubmitted, Finalized] {
            ledger.advance(&g, to).await.expect("valid advance");
            assert_eq!(ledger.state(&g).await.expect("state"), to);
        }
    }

    pub async fn second_create_is_already_exists<L: SigningLedger>(ledger: L) {
        let g = gate();
        ledger.create(&g).await.expect("create");
        assert_eq!(ledger.create(&g).await, Err(LedgerError::AlreadyExists));
    }

    pub async fn advance_missing_is_not_found<L: SigningLedger>(ledger: L) {
        assert_eq!(
            ledger.advance(&gate(), SigningLedgerState::Signing).await,
            Err(LedgerError::NotFound)
        );
        assert_eq!(ledger.state(&gate()).await, Err(LedgerError::NotFound));
    }

    pub async fn skip_forward_is_invalid<L: SigningLedger>(ledger: L) {
        let g = gate();
        ledger.create(&g).await.expect("create");
        // Approved -> Signed skips Signing.
        assert_eq!(
            ledger.advance(&g, SigningLedgerState::Signed).await,
            Err(LedgerError::InvalidTransition {
                from: SigningLedgerState::Approved,
                to: SigningLedgerState::Signed,
            })
        );
    }

    pub async fn regression_is_invalid<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;
        let g = gate();
        ledger.create(&g).await.expect("create");
        ledger.advance(&g, Signing).await.expect("to signing");
        ledger.advance(&g, Signed).await.expect("to signed");
        // Signed -> Approved regresses.
        assert_eq!(
            ledger.advance(&g, Approved).await,
            Err(LedgerError::InvalidTransition {
                from: Signed,
                to: Approved
            })
        );
    }

    pub async fn broadcast_idempotency_guard<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;
        let g = gate();
        ledger.create(&g).await.expect("create");
        ledger.advance(&g, Signing).await.expect("signing");
        ledger.advance(&g, Signed).await.expect("signed");
        ledger
            .advance(&g, BroadcastSubmitted)
            .await
            .expect("broadcast");
        // Once broadcast, re-signing / re-submitting is forbidden — this is the
        // guard that survives a Stuck->InProgress job recovery.
        for forbidden in [Signing, Signed, Approved] {
            assert_eq!(
                ledger.advance(&g, forbidden).await,
                Err(LedgerError::InvalidTransition {
                    from: BroadcastSubmitted,
                    to: forbidden
                }),
                "broadcast row must not move back to {forbidden:?}"
            );
        }
        // It may still reach a terminal.
        ledger.advance(&g, Finalized).await.expect("finalize");
    }

    /// Cross-tenant / two-gate isolation: two distinct gates — standing in for
    /// two tenants' independent signing flows — advance with no state bleed
    /// between them. The ledger is keyed purely by `gate_ref` (it carries no
    /// tenant component), so tenant isolation here is inherited from the fact
    /// that two tenants' flows always carry distinct `gate_ref`s. This case
    /// locks that independence: advancing gate A to a terminal must not move,
    /// create, or otherwise perturb gate B, and creating/advancing B must not
    /// touch A.
    pub async fn distinct_gates_advance_independently<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;
        // Two gate_refs as two different tenants' flows would carry.
        let gate_a = GateRef::new("gate:tenant-a:ledger");
        let gate_b = GateRef::new("gate:tenant-b:ledger");

        ledger.create(&gate_a).await.expect("create A");
        // B does not exist yet just because A does.
        assert_eq!(ledger.state(&gate_b).await, Err(LedgerError::NotFound));

        // Drive A all the way to a terminal.
        ledger.create(&gate_b).await.expect("create B");
        for to in [Signing, Signed, BroadcastSubmitted, Finalized] {
            ledger.advance(&gate_a, to).await.expect("advance A");
        }
        // B stayed exactly where it was created (Approved) — A's progression
        // never bled across the tenant/gate boundary.
        assert_eq!(ledger.state(&gate_b).await.expect("state B"), Approved);
        assert_eq!(ledger.state(&gate_a).await.expect("state A"), Finalized);

        // Now advance B independently; A (terminal) is unaffected.
        ledger.advance(&gate_b, Signing).await.expect("advance B");
        assert_eq!(ledger.state(&gate_a).await.expect("state A"), Finalized);
        assert_eq!(ledger.state(&gate_b).await.expect("state B"), Signing);
    }

    /// Every terminal state (`Finalized`, `Unknown`, `ManualReview`) rejects
    /// every possible subsequent transition. Each terminal is driven on its own
    /// `gate_ref` row so we exercise the real persisted terminal value, not a
    /// synthetic one. This proves a durable backend cannot quietly allow an
    /// auto-retry (fresh nonce/blockhash) out of any terminal state.
    pub async fn terminal_states_never_advance<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;

        const ALL_STATES: [SigningLedgerState; 7] = [
            Approved,
            Signing,
            Signed,
            BroadcastSubmitted,
            Finalized,
            Unknown,
            ManualReview,
        ];

        // (terminal, gate-name) — one row per terminal so each is independent.
        let terminals = [
            (Finalized, "gate:term-finalized"),
            (Unknown, "gate:term-unknown"),
            (ManualReview, "gate:term-manual"),
        ];

        for (terminal, name) in terminals {
            let g = gate_named(name);
            ledger.create(&g).await.expect("create");
            ledger.advance(&g, Signing).await.expect("signing");
            ledger.advance(&g, Signed).await.expect("signed");
            ledger
                .advance(&g, BroadcastSubmitted)
                .await
                .expect("broadcast");
            ledger
                .advance(&g, terminal)
                .await
                .unwrap_or_else(|e| panic!("reach terminal {terminal:?}: {e:?}"));
            assert_eq!(
                ledger.state(&g).await.expect("state"),
                terminal,
                "row must rest in {terminal:?}"
            );

            // No state — including the terminal itself — is a legal successor.
            for to in ALL_STATES {
                assert_eq!(
                    ledger.advance(&g, to).await,
                    Err(LedgerError::InvalidTransition { from: terminal, to }),
                    "{terminal:?} is terminal; must not advance to {to:?}"
                );
            }
            // State is unchanged after all the rejected attempts.
            assert_eq!(
                ledger.state(&g).await.expect("state"),
                terminal,
                "rejected transitions must not mutate the terminal row"
            );
        }
    }

    /// Two distinct `gate_ref`s have fully independent state machines. A backend
    /// that effectively stores a single global ledger row (no per-gate keying)
    /// CANNOT pass: advancing one gate must not move the other, and each must
    /// retain and reject according to its own state.
    pub async fn two_gates_are_isolated<L: SigningLedger>(ledger: L) {
        use SigningLedgerState::*;
        let a = gate_named("gate:iso-a");
        let b = gate_named("gate:iso-b");

        ledger.create(&a).await.expect("create a");
        ledger.create(&b).await.expect("create b");

        // Drive A all the way to a broadcast; leave B at Approved.
        ledger.advance(&a, Signing).await.expect("a signing");
        ledger.advance(&a, Signed).await.expect("a signed");
        ledger
            .advance(&a, BroadcastSubmitted)
            .await
            .expect("a broadcast");

        // B is untouched by A's progress.
        assert_eq!(
            ledger.state(&b).await.expect("b state"),
            Approved,
            "advancing gate A must not change gate B"
        );
        assert_eq!(ledger.state(&a).await.expect("a state"), BroadcastSubmitted);

        // B's own machine still works from its own (independent) state: the
        // only legal move from Approved is Signing, and skipping is rejected
        // with B's `from`, not A's.
        assert_eq!(
            ledger.advance(&b, Signed).await,
            Err(LedgerError::InvalidTransition {
                from: Approved,
                to: Signed
            }),
            "gate B must validate against its OWN state, not gate A's"
        );
        ledger.advance(&b, Signing).await.expect("b signing");
        assert_eq!(ledger.state(&b).await.expect("b state"), Signing);

        // And A is still independently at its broadcast state, rejecting a
        // regression with A's own `from`.
        assert_eq!(
            ledger.advance(&a, Signing).await,
            Err(LedgerError::InvalidTransition {
                from: BroadcastSubmitted,
                to: Signing
            })
        );
    }

    /// Many concurrent `advance(&gate, BroadcastSubmitted)` against a row
    /// pre-seeded at `Signed`: EXACTLY ONE must win and the rest must observe
    /// the already-broadcast state and fail. This is the precise double-submit
    /// race the ledger exists to stop — a non-atomic
    /// `SELECT current_state; UPDATE state` backend would let two callers both
    /// read `Signed` and both broadcast.
    pub async fn concurrent_advance_to_broadcast_yields_one_winner<L>(ledger: L)
    where
        L: SigningLedger + 'static,
    {
        use SigningLedgerState::*;
        let ledger = Arc::new(ledger);
        let g = gate();
        ledger.create(&g).await.expect("create");
        ledger.advance(&g, Signing).await.expect("signing");
        ledger.advance(&g, Signed).await.expect("signed");

        let mut handles = Vec::new();
        for _ in 0..32 {
            let ledger = Arc::clone(&ledger);
            let g = g.clone();
            handles.push(tokio::spawn(async move {
                ledger.advance(&g, BroadcastSubmitted).await
            }));
        }

        let mut ok = 0usize;
        let mut rejected = 0usize;
        for h in handles {
            match h.await.expect("task join") {
                Ok(()) => ok += 1,
                Err(LedgerError::InvalidTransition { from, to }) => {
                    assert_eq!(
                        (from, to),
                        (BroadcastSubmitted, BroadcastSubmitted),
                        "losers must observe the already-broadcast state"
                    );
                    rejected += 1;
                }
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one advance to BroadcastSubmitted may win");
        assert_eq!(rejected, 31, "all other advances must be rejected");
        assert_eq!(
            ledger.state(&g).await.expect("state"),
            BroadcastSubmitted,
            "final persisted state must be BroadcastSubmitted"
        );
    }

    /// Same concurrent one-winner property for the `Approved -> Signing` edge:
    /// pre-seed `Approved`, race many `advance(&gate, Signing)`, assert exactly
    /// one `Ok` and all losers see `InvalidTransition { from: Signing, to:
    /// Signing }`, ending at `Signing`.
    pub async fn concurrent_advance_to_signing_yields_one_winner<L>(ledger: L)
    where
        L: SigningLedger + 'static,
    {
        use SigningLedgerState::*;
        let ledger = Arc::new(ledger);
        let g = gate();
        ledger.create(&g).await.expect("create");

        let mut handles = Vec::new();
        for _ in 0..32 {
            let ledger = Arc::clone(&ledger);
            let g = g.clone();
            handles.push(tokio::spawn(
                async move { ledger.advance(&g, Signing).await },
            ));
        }

        let mut ok = 0usize;
        let mut rejected = 0usize;
        for h in handles {
            match h.await.expect("task join") {
                Ok(()) => ok += 1,
                Err(LedgerError::InvalidTransition { from, to }) => {
                    assert_eq!(
                        (from, to),
                        (Signing, Signing),
                        "losers must observe the already-Signing state"
                    );
                    rejected += 1;
                }
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(ok, 1, "exactly one advance to Signing may win");
        assert_eq!(rejected, 31, "all other advances must be rejected");
        assert_eq!(ledger.state(&g).await.expect("state"), Signing);
    }

    /// Many concurrent `create(&gate)` against the SAME `gate_ref`: exactly one
    /// must win (`Ok`) and every other must observe `AlreadyExists`. This is the
    /// concurrent face of the one-shot create — and the in-CI surfacing of the
    /// H1 collision concern: two tenants' flows that ever produced the same
    /// `gate_ref` string would race here, and the ledger guarantees the second
    /// `create` cannot silently win or duplicate the row (it collapses to
    /// `AlreadyExists`). A backend doing a non-atomic `SELECT ... ; INSERT` would
    /// let two creators both observe "absent" and both insert. NOTE: this proves
    /// the *create* CAS is atomic, NOT that two tenants are isolated — gate_ref
    /// uniqueness across tenants remains an unenforced caller obligation (see the
    /// plan doc, H1 follow-up).
    pub async fn concurrent_create_same_gate_ref_yields_one_winner<L>(ledger: L)
    where
        L: SigningLedger + 'static,
    {
        let ledger = Arc::new(ledger);
        let g = gate();

        let mut handles = Vec::new();
        for _ in 0..32 {
            let ledger = Arc::clone(&ledger);
            let g = g.clone();
            handles.push(tokio::spawn(async move { ledger.create(&g).await }));
        }

        let mut ok = 0usize;
        let mut already = 0usize;
        for h in handles {
            match h.await.expect("task join") {
                Ok(()) => ok += 1,
                Err(LedgerError::AlreadyExists) => already += 1,
                Err(other) => panic!("unexpected error under contention: {other:?}"),
            }
        }
        assert_eq!(
            ok, 1,
            "exactly one create may win the one-shot per-gate row"
        );
        assert_eq!(already, 31, "all other creates must be AlreadyExists");
        // The single winning row rests at Approved — no duplicate clobbered it.
        assert_eq!(
            ledger.state(&g).await.expect("state"),
            SigningLedgerState::Approved
        );
    }

    /// Drive every contract case against a fresh ledger from `$factory`.
    #[macro_export]
    macro_rules! signing_ledger_contract_cases {
        ($label:ident, $factory:expr) => {
            mod $label {
                #[tokio::test]
                async fn full_valid_sequence() {
                    $crate::ledger::contract::full_valid_sequence($factory()).await;
                }
                #[tokio::test]
                async fn second_create_is_already_exists() {
                    $crate::ledger::contract::second_create_is_already_exists($factory()).await;
                }
                #[tokio::test]
                async fn advance_missing_is_not_found() {
                    $crate::ledger::contract::advance_missing_is_not_found($factory()).await;
                }
                #[tokio::test]
                async fn skip_forward_is_invalid() {
                    $crate::ledger::contract::skip_forward_is_invalid($factory()).await;
                }
                #[tokio::test]
                async fn regression_is_invalid() {
                    $crate::ledger::contract::regression_is_invalid($factory()).await;
                }
                #[tokio::test]
                async fn broadcast_idempotency_guard() {
                    $crate::ledger::contract::broadcast_idempotency_guard($factory()).await;
                }
                #[tokio::test]
                async fn distinct_gates_advance_independently() {
                    $crate::ledger::contract::distinct_gates_advance_independently($factory())
                        .await;
                }
                #[tokio::test]
                async fn terminal_states_never_advance() {
                    $crate::ledger::contract::terminal_states_never_advance($factory()).await;
                }
                #[tokio::test]
                async fn two_gates_are_isolated() {
                    $crate::ledger::contract::two_gates_are_isolated($factory()).await;
                }
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                async fn concurrent_advance_to_broadcast_yields_one_winner() {
                    $crate::ledger::contract::concurrent_advance_to_broadcast_yields_one_winner(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                async fn concurrent_advance_to_signing_yields_one_winner() {
                    $crate::ledger::contract::concurrent_advance_to_signing_yields_one_winner(
                        $factory(),
                    )
                    .await;
                }
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                async fn concurrent_create_same_gate_ref_yields_one_winner() {
                    $crate::ledger::contract::concurrent_create_same_gate_ref_yields_one_winner(
                        $factory(),
                    )
                    .await;
                }
            }
        };
    }
}

#[cfg(test)]
crate::signing_ledger_contract_cases!(in_memory, crate::ledger::InMemorySigningLedger::new);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips_snake_case() {
        let json = serde_json::to_string(&SigningLedgerState::BroadcastSubmitted).expect("ser");
        assert_eq!(json, "\"broadcast_submitted\"");
        let back: SigningLedgerState = serde_json::from_str(&json).expect("de");
        assert_eq!(back, SigningLedgerState::BroadcastSubmitted);
    }

    #[test]
    fn terminal_and_broadcast_predicates() {
        assert!(SigningLedgerState::Finalized.is_terminal());
        assert!(SigningLedgerState::Unknown.is_terminal());
        assert!(SigningLedgerState::ManualReview.is_terminal());
        assert!(!SigningLedgerState::BroadcastSubmitted.is_terminal());
        assert!(SigningLedgerState::BroadcastSubmitted.is_broadcast());
        assert!(!SigningLedgerState::Signed.is_broadcast());
    }

    // A panic while the `rows` mutex is held poisons it. Every ledger method
    // must surface that as `LedgerError::Backend` rather than panicking, so a
    // poisoned lock degrades to a clean error instead of taking down callers.
    #[tokio::test]
    async fn in_memory_ledger_returns_backend_error_on_poisoned_lock() {
        let ledger = std::sync::Arc::new(InMemorySigningLedger::new());
        let ledger_clone = ledger.clone();
        let _ = std::thread::spawn(move || {
            let _lock = ledger_clone.rows.lock().expect("lock");
            panic!("poisoning lock");
        })
        .join();

        let gate = GateRef::new("gate");
        assert!(matches!(
            ledger.create(&gate).await,
            Err(LedgerError::Backend { .. })
        ));
        assert!(matches!(
            ledger.state(&gate).await,
            Err(LedgerError::Backend { .. })
        ));
        assert!(matches!(
            ledger.advance(&gate, SigningLedgerState::Signed).await,
            Err(LedgerError::Backend { .. })
        ));
    }
}
