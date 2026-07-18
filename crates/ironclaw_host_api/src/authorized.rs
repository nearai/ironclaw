//! Slice-C kernel vocabulary — the sealed `Authorized` witness.
//!
//! The security-critical heart of the capability-path collapse
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` §3,
//! §5.3.2). `authorize()` folds ALL pre-flightable policy (trust, approval,
//! resource reservation, lane resolution) into a single decision; its success
//! value is an [`Authorized`] — the proof that this exact [`Invocation`] passed
//! that fold. `dispatch()` accepts *only* an `Authorized`, so an un-authorized
//! invocation is structurally undispatchable.
//!
//! ## The seal (decision 2026-07-18: `host_api` + witness token)
//!
//! `Authorized` lives here (the bottom crate everyone references), but its fields
//! are **private** and there is no public field constructor. The only way to mint
//! one is [`Authorized::seal`], which consumes an [`AuthorizationGrant`] — a
//! zero-sized witness whose *only* constructor is
//! [`CapabilityAuthorizer::authorization_grant`]. So you cannot build an
//! `Authorized` without holding a `&impl CapabilityAuthorizer`.
//!
//! Pure cross-crate type-sealing is not expressible in Rust: `host_api` defines
//! the type but the *kernel* (`ironclaw_capabilities`) is the sole legitimate
//! minter, and those are different crates. The witness gives the structural
//! barrier (private fields + grant-gated construction); a companion
//! `ironclaw_architecture` test restricts `impl CapabilityAuthorizer` to the
//! kernel crate — type-seal plus test-seal. **Per §9 the seal's *guarantee* is
//! vacuous until `authorize()` inlines the four policy checks** (that later PR is
//! the explicit security milestone); this slice lands the structural witness so
//! `dispatch()`'s signature can demand it.

use crate::{Blocked, DenyRef, Invocation, MountView, ResourceReservation, RuntimeLane, Timestamp};

/// Proof of authority to mint an [`Authorized`]. The kernel authorizer implements
/// this trait on its own type; no one else legitimately does (enforced by an
/// `ironclaw_architecture` test restricting implementors to
/// `ironclaw_capabilities`).
///
/// This trait is intentionally NOT sealed to `host_api` — sealing it here would
/// stop the kernel crate from implementing it. Its teeth are: (1) the only way to
/// obtain an [`AuthorizationGrant`] is through it, and (2) the architecture test.
pub trait CapabilityAuthorizer {
    /// Mint a one-shot grant. Provided (not overridable in effect): the grant's
    /// field is private to `host_api`, so this default body is the sole source of
    /// a grant anywhere.
    fn authorization_grant(&self) -> AuthorizationGrant {
        AuthorizationGrant(())
    }
}

/// A zero-sized witness that an [`Authorized`] is being minted by the kernel
/// authorizer. Its only constructor is
/// [`CapabilityAuthorizer::authorization_grant`] (the field is private to
/// `host_api`), and [`Authorized::seal`] consumes it.
#[derive(Debug)]
pub struct AuthorizationGrant(());

/// The sealed proof that an [`Invocation`] passed `authorize()` — single-use,
/// lane-bound, and deadline-bounded (§3, §5.3.2).
///
/// - **Sealed:** private fields, minted only via [`Authorized::seal`] with an
///   [`AuthorizationGrant`]. No forging, no repairing to a different invocation.
/// - **Lane-bound:** it carries the exact [`RuntimeLane`] resolved from the
///   descriptor; `dispatch()` routes by that, never to a lane the descriptor did
///   not name.
/// - **Single-use:** not `Clone`; `dispatch()` consumes it. The not-dispatched
///   path calls [`Authorized::abort`] explicitly — never `Drop` (destructors do
///   no async I/O; a leaked witness is reclaimed by lease-expiry, §5.3.2).
/// - **Deadline-bounded:** [`Authorized::is_expired`] fails closed past the
///   deadline derived from the shortest-lived fact it froze (approval/credential
///   lease) — a held witness cannot outlive its facts.
#[derive(Debug)]
pub struct Authorized {
    invocation: Invocation,
    lane: RuntimeLane,
    mounts: MountView,
    reservation: ResourceReservation,
    deadline: Timestamp,
}

impl Authorized {
    /// Mint an `Authorized`. Callable only with an [`AuthorizationGrant`], which
    /// only a [`CapabilityAuthorizer`] can produce — i.e. only `authorize()` in
    /// the kernel. The authorization *outputs* (`lane`, `mounts`, `reservation`,
    /// `deadline`) are results the fold computed, not caller-supplied request
    /// fields.
    pub fn seal(
        _grant: AuthorizationGrant,
        invocation: Invocation,
        lane: RuntimeLane,
        mounts: MountView,
        reservation: ResourceReservation,
        deadline: Timestamp,
    ) -> Self {
        Self {
            invocation,
            lane,
            mounts,
            reservation,
            deadline,
        }
    }

    /// The exact invocation this witness authorized.
    pub fn invocation(&self) -> &Invocation {
        &self.invocation
    }

    /// The runtime lane resolved from the descriptor — where `dispatch()` routes.
    pub fn lane(&self) -> RuntimeLane {
        self.lane
    }

    /// The filesystem mounts authorized for this invocation.
    pub fn mounts(&self) -> &MountView {
        &self.mounts
    }

    /// The resource reservation held for this invocation.
    pub fn reservation(&self) -> &ResourceReservation {
        &self.reservation
    }

    /// The deadline past which this witness is no longer valid.
    pub fn deadline(&self) -> Timestamp {
        self.deadline
    }

    /// Whether the witness has outlived its facts. `dispatch()` after this must
    /// fail closed with `HostFailure::Permanent` (§5.3.2).
    pub fn is_expired(&self, now: Timestamp) -> bool {
        now > self.deadline
    }

    /// Consume the witness into its parts for the dispatch lane. Single-use: the
    /// witness is gone afterward, so it cannot be dispatched twice.
    pub fn into_parts(self) -> (Invocation, RuntimeLane, MountView, ResourceReservation) {
        (self.invocation, self.lane, self.mounts, self.reservation)
    }

    /// Unwind a not-dispatched witness (cancel between authorize and dispatch,
    /// runner handoff, shutdown). Consumes the witness so its reservation is
    /// explicitly released rather than leaked. Returns the reservation for the
    /// caller to release through the resource port; a failed release does not
    /// strand authority (lease-expiry reclaims it, §5.3.2). Never `Drop`.
    pub fn abort(self) -> ResourceReservation {
        self.reservation
    }
}

/// The success/deny/block trichotomy `authorize()` returns (§3). `Denied` and
/// `Blocked` fold into [`crate::Resolution::Denied`]/[`crate::Resolution::Blocked`]
/// at the call site; `Authorized` proceeds to `dispatch()`.
///
/// Not serializable: an `Authorized` holds a live reservation and is a one-shot
/// capability, not wire data.
#[derive(Debug)]
pub enum AuthorizeResult {
    /// Passed the fold — proceed to dispatch. Boxed because an `Authorized`
    /// (invocation + reservation + mounts) dwarfs the ref-sized deny/block
    /// variants.
    Authorized(Box<Authorized>),
    /// Terminal policy denial (model-visible, not re-entrant).
    Denied(DenyRef),
    /// A re-entrant gate — resolve and re-authorize.
    Blocked(Blocked),
}

impl AuthorizeResult {
    /// Stable discriminant for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            AuthorizeResult::Authorized(_) => "authorized",
            AuthorizeResult::Denied(_) => "denied",
            AuthorizeResult::Blocked(_) => "blocked",
        }
    }
}
// Tests live in `tests/authorized_seal.rs` (not an inline `#[cfg(test)]` module):
// exercising the seal requires a `CapabilityAuthorizer` impl, and the seal
// enforcement ratchet (`reborn_authorized_seal_ratchet`) bans that impl outside
// the kernel crate. Keeping the test double under `tests/` matches the
// convention the ratchet relies on (test doubles are not inventoried).
