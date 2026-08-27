//! Behavior tests for the sealed `Authorized` witness (arch-simplification §3/§5.3.2).
//!
//! These live under `tests/` (not an inline `#[cfg(test)]` module) on purpose:
//! constructing an `Authorized` requires implementing `CapabilityAuthorizer`, and
//! the `reborn_authorized_seal_ratchet` architecture test bans that impl anywhere
//! but the kernel crate. A test double under `tests/` is not inventoried by that
//! ratchet, so this is where the seal's own test authorizer belongs.

use ironclaw_host_api::{
    Timestamp,
    authorized::{
        AuthorizeResult, Authorized, CapabilityAuthorizer, ProcessAuthorizedContinuation,
    },
    capability::{CapabilityDescriptor, PermissionMode},
    ids::{
        ActivityId, CapabilityId, CorrelationId, DenyRef, ExtensionId, GateRef, ProcessId,
        ProductKind, ResourceReservationId, UserId,
    },
    invocation::{Actor, Invocation, InvocationOrigin},
    lane::RuntimeLane,
    mount::MountView,
    resolution::{Blocked, GateWaypoint},
    resource::{ResourceEstimate, ResourceReservation, ResourceScope},
    runtime::{RuntimeKind, TrustClass},
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

fn descriptor() -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new("shell.exec").unwrap(),
        provider: ExtensionId::new("builtin").unwrap(),
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::FirstParty,
        description: "test shell".to_string(),
        parameters_schema: serde_json::json!({}),
        effects: Vec::new(),
        default_permission: PermissionMode::Ask,
        runtime_credentials: Vec::new(),
        network_targets: Vec::new(),
        max_egress_bytes: None,
        resource_profile: None,
        origin_gate_matrix: None,
        standard_op: None,
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
        descriptor(),
        RuntimeLane::Process,
        mounts,
        reservation,
        deadline,
    )
    .expect("matching capability ids seal")
}

fn seal_invocation(invocation: Invocation) -> Authorized {
    let grant = TestAuthorizer.authorization_grant();
    Authorized::seal(
        grant,
        invocation,
        descriptor(),
        RuntimeLane::Process,
        Some(MountView::default()),
        Some(reservation()),
        ts(1000),
    )
    .expect("matching capability ids seal")
}

#[test]
fn seal_rejects_mismatched_descriptor_in_release_builds() {
    let grant = TestAuthorizer.authorization_grant();
    let mut invocation = invocation();
    invocation.capability = CapabilityId::new("other.capability").unwrap();

    let error = Authorized::seal(
        grant,
        invocation,
        descriptor(),
        RuntimeLane::Process,
        None,
        None,
        ts(1000),
    )
    .expect_err("mismatched capability ids must fail closed");

    assert!(matches!(
        error,
        ironclaw_host_api::authorized::AuthorizedSealError::CapabilityMismatch { .. }
    ));
}

fn ts(secs: i64) -> Timestamp {
    chrono::DateTime::from_timestamp(secs, 0).unwrap()
}

#[test]
fn authorized_is_lane_bound_and_carries_its_invocation() {
    let auth = seal_one(ts(1000));
    assert_eq!(auth.lane(), RuntimeLane::Process);
    assert_eq!(auth.invocation().capability.as_str(), "shell.exec");
    assert_eq!(auth.descriptor(), Some(&descriptor()));
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
    let (inv, frozen_descriptor, lane, _mounts, res) = auth
        .into_parts(ts(999))
        .expect("unexpired witness must consume");
    // `auth` is moved — a second dispatch is a compile error, not a runtime bug.
    assert_eq!(lane, RuntimeLane::Process);
    assert_eq!(inv.capability.as_str(), "shell.exec");
    assert_eq!(frozen_descriptor, Some(descriptor()));
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
    assert_eq!(continuation.descriptor, Some(descriptor()));
}

#[test]
fn process_authorized_continuation_round_trips_host_assigned_descriptor() {
    let continuation = ProcessAuthorizedContinuation::from_authorized(
        seal_invocation(invocation()),
        ts(999),
        ProcessId::new(),
    )
    .expect("unexpired process authorization converts");

    let encoded = serde_json::to_value(&continuation).unwrap();
    let decoded: ProcessAuthorizedContinuation = serde_json::from_value(encoded).unwrap();

    assert_eq!(decoded.descriptor, Some(descriptor()));
}

#[test]
fn process_authorized_continuation_accepts_legacy_records_without_a_descriptor() {
    let continuation = ProcessAuthorizedContinuation::from_authorized(
        seal_invocation(invocation()),
        ts(999),
        ProcessId::new(),
    )
    .expect("unexpired process authorization converts");
    let mut encoded = serde_json::to_value(continuation).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("descriptor")
        .expect("new records include the descriptor");

    let decoded: ProcessAuthorizedContinuation = serde_json::from_value(encoded).unwrap();
    assert!(
        decoded.descriptor.is_none(),
        "legacy records must deserialize into the explicit registry-fallback state"
    );
}

#[test]
fn reservation_is_none_when_the_capability_declares_no_resource_obligation() {
    // A capability WITHOUT a resource obligation seals no reservation — never a
    // synthesized placeholder. Consumption and abort surface `None`.
    let auth = seal_with_reservation(ts(1000), None);
    assert!(auth.reservation().is_none());
    let (_inv, _descriptor, _lane, _mounts, res) = auth
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
    let (_inv, _descriptor, _lane, mounts, _res) = some.into_parts(ts(999)).expect("unexpired");
    assert_eq!(mounts, Some(MountView::default()));

    let none = seal_with_mounts_and_reservation(ts(1000), None, None);
    assert!(none.mounts().is_none());
    let (_inv, _descriptor, _lane, mounts, _res) = none.into_parts(ts(999)).expect("unexpired");
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
