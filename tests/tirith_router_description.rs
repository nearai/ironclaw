//! Unit tests for the `EngineError::GatePaused.reason` plumbing through
//! `ThreadOutcome::GatePaused.reason` and into `PendingGate.description`.

use ironclaw_engine::EngineError;
use ironclaw_engine::gate::ResumeKind;
use ironclaw_engine::runtime::messaging::ThreadOutcome;

#[test]
fn engine_error_gate_paused_carries_reason() {
    let err = EngineError::GatePaused {
        gate_name: "approval".into(),
        action_name: "shell".into(),
        call_id: "call-1".into(),
        parameters: Box::new(serde_json::json!({"command": "ls"})),
        resume_kind: Box::new(ResumeKind::Approval {
            allow_always: false,
        }),
        resume_output: None,
        paused_lease: None,
        reason: Some(Box::new(
            "tirith findings: [HIGH] homograph: Cyrillic char".into(),
        )),
    };

    match err {
        EngineError::GatePaused { reason, .. } => {
            assert_eq!(
                reason.as_deref().map(String::as_str),
                Some("tirith findings: [HIGH] homograph: Cyrillic char")
            );
        }
        other => panic!("expected GatePaused, got {other:?}"),
    }
}

#[test]
fn engine_error_gate_paused_reason_defaults_to_none() {
    // The existing non-tirith pause path constructs `GatePaused` with
    // `reason: None`. router.rs:4035's `unwrap_or_else` then falls back to
    // the generic "Tool 'X' requires Y (gate: Z)" string. This test pins
    // that contract — if someone changes the field's default behavior they
    // need to update the router fallback too.
    let err = EngineError::GatePaused {
        gate_name: "approval".into(),
        action_name: "shell".into(),
        call_id: "call-2".into(),
        parameters: Box::new(serde_json::json!({})),
        resume_kind: Box::new(ResumeKind::Approval { allow_always: true }),
        resume_output: None,
        paused_lease: None,
        reason: None,
    };
    match err {
        EngineError::GatePaused { reason, .. } => assert!(reason.is_none()),
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[test]
fn thread_outcome_gate_paused_carries_reason() {
    let outcome = ThreadOutcome::GatePaused {
        gate_name: "approval".into(),
        action_name: "shell".into(),
        call_id: "call-3".into(),
        parameters: serde_json::json!({"command": "rm -rf /"}),
        resume_kind: ResumeKind::Approval {
            allow_always: false,
        },
        resume_output: None,
        paused_lease: None,
        reason: Some(Box::new("tirith blocked".into())),
    };
    match outcome {
        ThreadOutcome::GatePaused { reason, .. } => {
            assert_eq!(
                reason.as_deref().map(String::as_str),
                Some("tirith blocked")
            );
        }
        other => panic!("expected GatePaused, got {other:?}"),
    }
}

/// The shape used by `src/bridge/router.rs:4035`:
/// `reason.as_deref().map(str::to_string).unwrap_or_else(|| format!(...))`.
/// We can't easily call into router.rs from a workspace integration test
/// (it depends on a full `BridgeState`), so this test reproduces the
/// fallback expression locally to lock the contract.
#[test]
fn pending_gate_description_uses_reason_when_present() {
    let reason: Option<Box<String>> = Some(Box::new("tirith finding: rm -rf /".into()));
    let description = reason.as_deref().cloned().unwrap_or_else(|| {
        format!(
            "Tool '{}' requires {} (gate: {})",
            "shell", "Approval", "approval"
        )
    });
    assert_eq!(description, "tirith finding: rm -rf /");
}

#[test]
fn pending_gate_description_falls_back_when_reason_none() {
    let reason: Option<Box<String>> = None;
    let description = reason.as_deref().cloned().unwrap_or_else(|| {
        format!(
            "Tool '{}' requires {} (gate: {})",
            "shell", "Approval", "approval"
        )
    });
    assert_eq!(
        description,
        "Tool 'shell' requires Approval (gate: approval)"
    );
}
