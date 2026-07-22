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

use serde::{Deserialize, Serialize};

use crate::{
    ActivityId, Actor, Blocked, CapabilityId, CorrelationId, DenyRef, Invocation, InvocationOrigin,
    MountView, ProcessId, ResourceEstimate, ResourceReservation, ResourceScope, RuntimeLane,
    Timestamp,
};

/// Proof of authority to mint an [`Authorized`]. The kernel authorizer implements
/// this trait on its own type; no one else legitimately does (enforced by an
/// `ironclaw_architecture` test restricting implementors to
/// `ironclaw_capabilities`).
///
/// This trait is intentionally NOT sealed to `host_api` — sealing it here would
/// stop the kernel crate from implementing it. Its teeth are: (1) the only way to
/// obtain an [`AuthorizationGrant`] is through it, and (2) the architecture test.
///
/// **Boundary scope (be precise about what this defends):** the seal enforces
/// *workspace layering* — no crate in this repository other than the kernel may
/// mint authority — via private fields + grant-gated construction (compiler) and
/// the implementor ratchet (CI). An *external* embedder that implements this
/// trait is outside the boundary by definition: it already composes the host
/// (wires `dispatch()`, owns the stores), so a forged witness grants it nothing
/// it does not already have. The ratchet therefore scans this workspace, and the
/// real guarantee lands when `authorize()` inlines the four policy checks (§9's
/// explicit security milestone).
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

/// The dispatch-lane parts a consumed [`Authorized`] yields: the exact
/// invocation, the descriptor-resolved lane, the fold's `Option<MountView>`, and
/// the fold's `Option<ResourceReservation>` — all verbatim, never re-derived.
pub type AuthorizedParts = (
    Invocation,
    RuntimeLane,
    Option<MountView>,
    Option<ResourceReservation>,
);

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
    /// The filesystem mounts the fold's `UseScopedMounts` obligation produced
    /// (`Some`), or `None` when the capability declares no mount obligation.
    /// Kept as an `Option` — never collapsed to a default `MountView` — so the
    /// witness carries the *exact* value the fold's `obligation_outcome`
    /// produced: `dispatch()` routes it byte-for-byte, and the `None`-vs-empty
    /// distinction the filesystem resolver relies on (a `None` mount fails a
    /// `ScopedVirtual`-backed capability closed, an empty one does not) is not
    /// silently erased. Single-use, lane-bound, deadline-bounded like the
    /// witness that carries it.
    mounts: Option<MountView>,
    /// The real reservation the fold's resource obligation produced (`Some`), or
    /// `None` when the capability declares no resource obligation. This is the
    /// authoritative reservation held for this invocation — never a synthesized
    /// placeholder. Single-use, lane-bound, deadline-bounded like the witness
    /// that carries it.
    reservation: Option<ResourceReservation>,
    deadline: Timestamp,
}

impl Authorized {
    /// Mint an `Authorized`. Callable only with an [`AuthorizationGrant`], which
    /// only a [`CapabilityAuthorizer`] can produce — i.e. only `authorize()` in
    /// the kernel. The authorization *outputs* (`lane`, `mounts`, `reservation`,
    /// `deadline`) are results the fold computed, not caller-supplied request
    /// fields. `reservation` is the real reservation the fold's resource
    /// obligation produced (`Some`), or `None` when the capability declares no
    /// resource obligation — never a synthesized placeholder. `mounts` is
    /// likewise the fold's `Option<MountView>` verbatim (`None` when the
    /// capability declared no mount obligation), never a collapsed default.
    pub fn seal(
        _grant: AuthorizationGrant,
        invocation: Invocation,
        lane: RuntimeLane,
        mounts: Option<MountView>,
        reservation: Option<ResourceReservation>,
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn seal_for_test(
        invocation: Invocation,
        lane: RuntimeLane,
        mounts: MountView,
        reservation: Option<ResourceReservation>,
        deadline: Timestamp,
    ) -> Self {
        Self {
            invocation,
            lane,
            mounts: Some(mounts),
            reservation,
            deadline,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seal_for_test_with_mounts(
        invocation: Invocation,
        lane: RuntimeLane,
        mounts: Option<MountView>,
        reservation: Option<ResourceReservation>,
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

    /// The filesystem mounts authorized for this invocation — the fold's
    /// `Option<MountView>` verbatim (`None` when the capability declared no
    /// mount obligation), so callers see the same `None`-vs-empty distinction
    /// today's dispatch input carries.
    pub fn mounts(&self) -> Option<&MountView> {
        self.mounts.as_ref()
    }

    /// The real reservation the fold's resource obligation produced (`Some`), or
    /// `None` when the capability declares no resource obligation — never a
    /// synthesized placeholder.
    pub fn reservation(&self) -> Option<&ResourceReservation> {
        self.reservation.as_ref()
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

    /// Consume the witness into its parts for the dispatch lane, failing closed
    /// on expiry: consumption checks the deadline itself (review finding on the
    /// C.7 slice — an optional pre-check can be omitted; the consuming operation
    /// cannot be). On expiry the intact witness comes back as `Err` so the
    /// caller releases its reservation through [`Authorized::abort`]. Single-use
    /// either way: an `Ok` consumes the witness, so it cannot be dispatched
    /// twice.
    pub fn into_parts(self, now: Timestamp) -> Result<AuthorizedParts, Box<Authorized>> {
        if self.is_expired(now) {
            // Boxed: the witness is large and the expiry arm is cold
            // (clippy::result_large_err).
            return Err(Box::new(self));
        }
        Ok((self.invocation, self.lane, self.mounts, self.reservation))
    }

    /// Unwind a not-dispatched witness (cancel between authorize and dispatch,
    /// runner handoff, shutdown). Consumes the witness so its reservation is
    /// explicitly released rather than leaked. Returns the reservation for the
    /// caller to release through the resource port (`None` when the capability
    /// declared no resource obligation, so there is nothing to release); a failed
    /// release does not strand authority (lease-expiry reclaims it, §5.3.2).
    /// Never `Drop`.
    pub fn abort(self) -> Option<ResourceReservation> {
        self.reservation
    }
}

/// Durable process-lifetime continuation of an allowed spawn decision.
///
/// Unlike [`Authorized`], this record is not single-use and is not bounded by a
/// short witness deadline. It persists the authority facts produced at spawn so
/// the process executor can re-mint a fresh witness when detached execution
/// begins, without re-running policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessAuthorizedContinuation {
    pub invocation: ProcessAuthorizedInvocation,
    pub lane: RuntimeLane,
    pub mounts: Option<MountView>,
    pub resource_reservation: Option<ResourceReservation>,
}

/// The invocation facts persisted with a spawned process authority record.
///
/// Raw `input` is intentionally not stored here; the process start path already
/// owns the execution payload and supplies it when re-minting the short-lived
/// [`Invocation`] carried by [`Authorized`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessAuthorizedInvocation {
    pub activity_id: ActivityId,
    pub capability: CapabilityId,
    pub scope: ResourceScope,
    pub actor: Actor,
    pub origin: InvocationOrigin,
    pub estimate: ResourceEstimate,
    pub correlation_id: CorrelationId,
    pub process_id: ProcessId,
    pub parent_process_id: Option<ProcessId>,
}

impl ProcessAuthorizedContinuation {
    /// Convert the one-shot spawn witness into the durable process-lifetime
    /// continuation record. The process id is generated after authorization, so
    /// it is bound here before the process store persists the record.
    pub fn from_authorized(
        authorized: Authorized,
        now: Timestamp,
        process_id: ProcessId,
    ) -> Result<Self, Box<Authorized>> {
        let (invocation, lane, mounts, resource_reservation) = authorized.into_parts(now)?;
        let Invocation {
            activity_id,
            capability,
            input: _, // intentionally not persisted; ProcessStart owns execution input.
            scope,
            actor,
            origin,
            estimate,
            correlation_id,
            // The original process id is the direct spawner for nested process
            // starts; the new spawned process id is bound below.
            process_id: parent_process_id,
            parent_process_id: _,
        } = invocation;
        Ok(Self {
            invocation: ProcessAuthorizedInvocation {
                activity_id,
                capability,
                scope,
                actor,
                origin,
                estimate,
                correlation_id,
                process_id,
                parent_process_id,
            },
            lane,
            mounts,
            resource_reservation,
        })
    }
}

/// The success/deny/block trichotomy `authorize()` returns (§3). `Denied` and
/// `Blocked` fold into [`crate::Resolution::Denied`]/[`crate::Resolution::Blocked`]
/// at the call site; `Authorized` proceeds to `dispatch()`.
///
/// Not serializable: an `Authorized` may hold a live reservation and is a
/// one-shot capability, not wire data.
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
