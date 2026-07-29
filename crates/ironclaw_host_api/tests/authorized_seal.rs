//! Behavior tests for the sealed `Authorized` witness (arch-simplification §3/§5.3.2).
//!
//! These live under `tests/` (not an inline `#[cfg(test)]` module) on purpose:
//! constructing an `Authorized` requires implementing `CapabilityAuthorizer`, and
//! the `reborn_authorized_seal_ratchet` architecture test bans that impl anywhere
//! but the kernel crate. A test double under `tests/` is not inventoried by that
//! ratchet, so this is where the seal's own test authorizer belongs.

use ironclaw_host_api::{
    ActivityId, Actor, AuthorizeResult, Authorized, Blocked, CapabilityAuthorizer, CapabilityId,
    CorrelationId, DenyRef, GateRef, GateWaypoint, Invocation, InvocationOrigin, MountView,
    ProcessAuthorizedContinuation, ProcessId, ProductKind, ResourceEstimate, ResourceReservation,
    ResourceReservationId, ResourceScope, RuntimeLane, Timestamp, UserId,
};

/// A stand-in kernel authorizer. In production the sole impl lives in
/// `ironclaw_capabilities`, guarded by the seal ratchet.
struct TestAuthorizer;
impl CapabilityAuthorizer for TestAuthorizer {}

fn invocation() -> Invocation {
    Invocation {
        activity_id: ActivityId::new(),
        capability: CapabilityId::new("shell.exec").unwrap(),
        input: serde_json::json!({}),
        scope: ResourceScope::system(),
        actor: Actor::Sealed(UserId::new("user1").unwrap()),
        origin: InvocationOrigin::Product(ProductKind::new("settings").unwrap()),
        estimate: ResourceEstimate::default(),
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
    }
}

fn reservation() -> ResourceReservation {
    ResourceReservation {
        id: ResourceReservationId::new(),
        scope: ResourceScope::system(),
        estimate: ResourceEstimate::default(),
    }
}

fn seal_one(deadline: Timestamp) -> Authorized {
    seal_with_reservation(deadline, Some(reservation()))
}

fn seal_with_reservation(
    deadline: Timestamp,
    reservation: Option<ResourceReservation>,
) -> Authorized {
    seal_with_mounts_and_reservation(deadline, Some(MountView::default()), reservation)
}

fn seal_with_mounts_and_reservation(
    deadline: Timestamp,
    mounts: Option<MountView>,
    reservation: Option<ResourceReservation>,
) -> Authorized {
    let grant = TestAuthorizer.authorization_grant();
    Authorized::seal(
        grant,
        invocation(),
        RuntimeLane::Process,
        mounts,
        reservation,
        deadline,
    )
}

fn seal_invocation(invocation: Invocation) -> Authorized {
    let grant = TestAuthorizer.authorization_grant();
    Authorized::seal(
        grant,
        invocation,
        RuntimeLane::Process,
        Some(MountView::default()),
        Some(reservation()),
        ts(1000),
    )
}

fn ts(secs: i64) -> Timestamp {
    chrono::DateTime::from_timestamp(secs, 0).unwrap()
}

#[test]
fn authorized_is_lane_bound_and_carries_its_invocation() {
    let auth = seal_one(ts(1000));
    assert_eq!(auth.lane(), RuntimeLane::Process);
    assert_eq!(auth.invocation().capability.as_str(), "shell.exec");
}

#[test]
fn deadline_fails_closed_past_the_frozen_facts() {
    let auth = seal_one(ts(1000));
    assert!(!auth.is_expired(ts(999)));
    assert!(!auth.is_expired(ts(1000))); // boundary: not yet expired at the deadline
    assert!(auth.is_expired(ts(1001)));
}

#[test]
fn single_use_consumes_into_parts_before_deadline() {
    let auth = seal_one(ts(1000));
    let (inv, lane, _mounts, res) = auth
        .into_parts(ts(999))
        .expect("unexpired witness must consume");
    // `auth` is moved — a second dispatch is a compile error, not a runtime bug.
    assert_eq!(lane, RuntimeLane::Process);
    assert_eq!(inv.capability.as_str(), "shell.exec");
    // The real obligation-produced reservation flows through consumption.
    assert!(res.is_some());
}

#[test]
fn reservation_is_some_when_a_resource_obligation_produced_one() {
    // A capability WITH a resource obligation seals the real reservation.
    let expected = reservation();
    let auth = seal_with_reservation(ts(1000), Some(expected.clone()));
    assert_eq!(auth.reservation(), Some(&expected));
    assert_eq!(auth.abort(), Some(expected));
}

#[test]
fn process_authorized_continuation_preserves_direct_spawner_lineage() {
    let spawner = ProcessId::new();
    let grandparent = ProcessId::new();
    let spawned = ProcessId::new();
    let mut invocation = invocation();
    invocation.process_id = Some(spawner);
    invocation.parent_process_id = Some(grandparent);

    let continuation = ProcessAuthorizedContinuation::from_authorized(
        seal_invocation(invocation),
        ts(999),
        spawned,
    )
    .expect("unexpired process authorization converts");

    assert_eq!(continuation.invocation.process_id, spawned);
    assert_eq!(continuation.invocation.parent_process_id, Some(spawner));
}

#[test]
fn reservation_is_none_when_the_capability_declares_no_resource_obligation() {
    // A capability WITHOUT a resource obligation seals no reservation — never a
    // synthesized placeholder. Consumption and abort surface `None`.
    let auth = seal_with_reservation(ts(1000), None);
    assert!(auth.reservation().is_none());
    let (_inv, _lane, _mounts, res) = auth
        .into_parts(ts(999))
        .expect("unexpired witness must consume");
    assert!(res.is_none());

    let auth = seal_with_reservation(ts(1000), None);
    assert!(auth.abort().is_none());
}

#[test]
fn into_parts_fails_closed_on_expiry_and_returns_the_witness_for_abort() {
    // Regression (review finding on the C.7 slice): consumption itself must
    // check the deadline — an optional is_expired() pre-check can be omitted,
    // the consuming operation cannot be. The expired witness comes back intact
    // so its reservation is released explicitly, not stranded.
    let auth = seal_one(ts(1000));
    let expired = auth
        .into_parts(ts(1001))
        .expect_err("expired witness must not yield dispatch parts");
    assert!(expired.is_expired(ts(1001)));
    let reservation = expired.abort(); // reservation still explicitly releasable
    assert!(reservation.is_some());
}

#[test]
fn abort_returns_the_reservation_for_explicit_release() {
    let auth = seal_one(ts(1000));
    let reservation = auth.abort(); // consumed, not dropped implicitly
    assert!(reservation.is_some());
}

#[test]
fn mounts_are_carried_and_consumed_as_an_option_not_a_collapsed_default() {
    // The witness must preserve the fold's `Option<MountView>` verbatim so
    // `dispatch()` routes the same `None`-vs-empty distinction today's dispatch
    // input carries (a `None` mount fails a `ScopedVirtual` capability closed;
    // an empty one does not). Collapsing `None` to a default would erase that.
    let some = seal_with_mounts_and_reservation(ts(1000), Some(MountView::default()), None);
    assert_eq!(some.mounts(), Some(&MountView::default()));
    let (_inv, _lane, mounts, _res) = some.into_parts(ts(999)).expect("unexpired");
    assert_eq!(mounts, Some(MountView::default()));

    let none = seal_with_mounts_and_reservation(ts(1000), None, None);
    assert!(none.mounts().is_none());
    let (_inv, _lane, mounts, _res) = none.into_parts(ts(999)).expect("unexpired");
    assert!(
        mounts.is_none(),
        "a `None` mount must not become an empty default"
    );
}

#[test]
fn authorize_result_kinds() {
    assert_eq!(
        AuthorizeResult::Authorized(Box::new(seal_one(ts(1000)))).kind(),
        "authorized"
    );
    assert_eq!(AuthorizeResult::Denied(DenyRef::new()).kind(), "denied");
    assert_eq!(
        AuthorizeResult::Blocked(Blocked::Auth(GateWaypoint::new(GateRef::new()))).kind(),
        "blocked"
    );
}
