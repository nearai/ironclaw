use super::*;
use ironclaw_host_api::turn::{LoopGateRef, LoopResultRef};
use ironclaw_loop_contracts::{
    CapabilityInputRef, CapabilityProgress, CapabilitySurfaceVersion, resolution,
};

fn call(input: &str) -> CapabilityCallCandidate {
    let capability_id = ironclaw_host_api::ids::CapabilityId::new("test.cap").unwrap();
    CapabilityCallCandidate {
        activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
        surface_version: CapabilitySurfaceVersion::new("test-v1").unwrap(),
        capability_id: capability_id.clone(),
        effective_capability_ids: vec![capability_id],
        input_ref: CapabilityInputRef::new(format!("input:{input}")).unwrap(),
        provider_replay: None,
    }
}

// The fixtures build the exact `Resolution` the producer constructors
// emit so `shared_await_dependent_gate` sees the flip's channel shape
// (origin preserved on the channel).
fn await_dependent(gate: &str, result: &str) -> Resolution {
    resolution::await_dependent_run(
        LoopGateRef::new(gate).unwrap(),
        LoopResultRef::new(format!("result:{result}")).unwrap(),
        "summary".to_string(),
        0,
        None,
    )
    .resolution
}

fn completed(result: &str) -> Resolution {
    resolution::completed(
        LoopResultRef::new(format!("result:{result}")).unwrap(),
        "summary".to_string(),
        CapabilityProgress::MadeProgress,
        false,
        0,
        None,
        None,
    )
}

#[test]
fn returns_some_for_two_outcomes_sharing_one_gate() {
    let calls = vec![call("a"), call("b")];
    let outcomes = vec![
        await_dependent("gate:batch-1", "r1"),
        await_dependent("gate:batch-1", "r2"),
    ];
    let result = shared_await_dependent_gate(&calls, &outcomes);
    assert!(result.is_some());
    let (gate, first) = result.unwrap();
    assert_eq!(gate.as_str(), "gate:batch-1");
    assert_eq!(first.input_ref.as_str(), "input:a");
}

#[test]
fn returns_none_for_divergent_gate_refs() {
    let calls = vec![call("a"), call("b")];
    let outcomes = vec![
        await_dependent("gate:a", "r1"),
        await_dependent("gate:b", "r2"),
    ];
    assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
}

#[test]
fn returns_none_for_single_await_with_completed_sibling() {
    // Single AwaitDependentRun has no coalescing benefit; fall back to
    // the per-outcome path for completed-first durability ordering.
    let calls = vec![call("a"), call("b")];
    let outcomes = vec![await_dependent("gate:1", "r1"), completed("r2")];
    assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
}

#[test]
fn returns_none_when_non_await_suspension_present() {
    let calls = vec![call("a"), call("b")];
    let outcomes = vec![
        await_dependent("gate:1", "r1"),
        resolution::approval_required(
            LoopGateRef::new("gate:approval").unwrap(),
            "approval".to_string(),
            None,
        )
        .resolution,
    ];
    assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
}

#[test]
fn returns_none_for_empty_outcomes() {
    assert!(shared_await_dependent_gate(&[], &[]).is_none());
}

#[test]
fn returns_some_for_two_awaits_with_completed_between() {
    let calls = vec![call("a"), call("b"), call("c")];
    let outcomes = vec![
        await_dependent("gate:batch-2", "r1"),
        completed("r2"),
        await_dependent("gate:batch-2", "r3"),
    ];
    let result = shared_await_dependent_gate(&calls, &outcomes);
    assert!(result.is_some());
    let (gate, _) = result.unwrap();
    assert_eq!(gate.as_str(), "gate:batch-2");
}

#[test]
fn gate_outcome_kind_maps_every_gate_writing_variant() {
    let cases = [
        (
            resolution::approval_required(
                LoopGateRef::new("gate:kind-approval").unwrap(),
                "approval".to_string(),
                None,
            )
            .resolution,
            Some(GateKind::Approval),
        ),
        (
            resolution::auth_required(
                LoopGateRef::new("gate:kind-auth").unwrap(),
                Vec::new(),
                "auth".to_string(),
                None,
            )
            .resolution,
            Some(GateKind::Auth),
        ),
        (
            resolution::resource_blocked(
                LoopGateRef::new("gate:kind-resource").unwrap(),
                "resource".to_string(),
            )
            .resolution,
            Some(GateKind::Resource),
        ),
        (
            resolution::external_tool_pending(
                LoopGateRef::new("gate:kind-external-tool").unwrap(),
                "external tool".to_string(),
            )
            .resolution,
            Some(GateKind::ExternalTool),
        ),
        (
            await_dependent("gate:kind-dependent", "r"),
            Some(GateKind::AwaitDependentRun),
        ),
        (completed("r-none"), None),
    ];
    for (outcome, expected) in cases {
        assert_eq!(
            gate_outcome_kind(&outcome),
            expected,
            "gate_outcome_kind must agree with gate_outcome_writes_before_block"
        );
        assert_eq!(
            gate_outcome_writes_before_block(&outcome),
            expected.is_some()
        );
    }
}
