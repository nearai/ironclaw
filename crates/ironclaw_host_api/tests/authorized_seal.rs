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
    ProductKind, ResourceEstimate, ResourceReservation, ResourceReservationId, ResourceScope,
    RuntimeLane, Timestamp, UserId,
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
    let grant = TestAuthorizer.authorization_grant();
    Authorized::seal(
        grant,
        invocation(),
        RuntimeLane::Process,
        MountView::default(),
        reservation(),
        deadline,
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
    let (inv, lane, _mounts, _res) = auth
        .into_parts(ts(999))
        .expect("unexpired witness must consume");
    // `auth` is moved — a second dispatch is a compile error, not a runtime bug.
    assert_eq!(lane, RuntimeLane::Process);
    assert_eq!(inv.capability.as_str(), "shell.exec");
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
    let _reservation = expired.abort(); // reservation still explicitly releasable
}

#[test]
fn abort_returns_the_reservation_for_explicit_release() {
    let auth = seal_one(ts(1000));
    let _reservation = auth.abort(); // consumed, not dropped implicitly
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
