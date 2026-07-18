//! Slice-C kernel vocabulary — the capability result channels.
//!
//! Part of the capability-path result collapse
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md` §3,
//! §5.3). Today a single overloaded ten-variant `CapabilityOutcome`
//! (`ironclaw_turns`) carries every non-happy path, and a recoverable
//! `Ok(Failed)` is structurally indistinguishable from a run-terminating `Err`
//! (§1.2). The target model replaces that with **five distinct channels** — one
//! per real outcome kind:
//!
//! - [`crate::HostFailure`] — infrastructure failure (the `Err` arm; already landed).
//! - `Outcome` — tool success or recoverable failure (a later slice).
//! - a terminal `Denied` — model-visible policy denial, **not** re-entrant.
//! - [`Blocked`] — re-entrant gates (this module).
//! - [`Suspension`] — parked work (this module).
//!
//! and folds them into one `Resolution` (a later slice, once `Outcome` exists).
//! This module lands the two gate/suspension channels first, additively (§9) —
//! nothing produces them yet.
//!
//! ## Blocked vs. Suspension — the distinction that matters
//!
//! Both pause the invocation, but they resume differently:
//!
//! - **[`Blocked`]** is a *re-entrant gate*: the call did not run, and once the
//!   gate is resolved (approval granted, credential supplied, resource freed) the
//!   invocation re-enters `authorize()` and may run. It is the request asking for
//!   a decision.
//! - **[`Suspension`]** is *parked work*: the effect is already in flight (a
//!   spawned process) or has been handed off (a dependent run, a client-executed
//!   external tool), and control either continues elsewhere or returns to the API
//!   client until the awaited thing completes.
//!
//! Getting these confused is the #6137 bug class; keeping them separate types
//! makes the confusion a compile error rather than a runtime mis-route.

use serde::{Deserialize, Serialize};

use crate::{GateRef, ProcessRef, ResultRef, SafeSummary};

/// A re-entrant gate: the invocation did not run and is waiting on a decision.
/// Resolving the gate re-enters `authorize()` (§5.3.3: a resolved gate reserves
/// against *then-current* budget — approval is consent, not an execution
/// guarantee).
///
/// Per §5.3.1, `authorize()` can raise any of these pre-flight; `dispatch()` may
/// raise **only** [`Blocked::Auth`] (a lane discovers a credential demand only by
/// calling the thing — an MCP 401, a WASM credential fault). A lane-originated
/// Approval or Resource gate is a `HostFailure::Permanent`, never a gate,
/// enforced by conformance test (§11.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocked {
    /// Needs human approval before it may run.
    Approval(GateRef),
    /// Needs a credential the caller has not supplied (auth gate). The only kind
    /// `dispatch()` may surface (§5.3.1).
    Auth(GateRef),
    /// Needs resource budget currently unavailable.
    Resource(GateRef),
}

impl Blocked {
    /// The handle to the pending gate record, regardless of kind.
    pub fn gate_ref(&self) -> &GateRef {
        match self {
            Blocked::Approval(g) | Blocked::Auth(g) | Blocked::Resource(g) => g,
        }
    }

    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            Blocked::Approval(_) => "approval",
            Blocked::Auth(_) => "auth",
            Blocked::Resource(_) => "resource",
        }
    }

    /// Whether this gate kind may be surfaced by `dispatch()` (only `Auth`,
    /// §5.3.1). Approval/Resource are authorize-time-only; a dispatch-time one is
    /// a contract violation.
    pub fn is_dispatch_time_permitted(&self) -> bool {
        matches!(self, Blocked::Auth(_))
    }
}

/// Parked work: the effect is in flight or handed off, and the invocation
/// yields until it completes. Unlike [`Blocked`], the call is not waiting for a
/// decision — it is waiting for a *result*.
///
/// `Process` suspends the turn to §11.1 `WaitingProcess`; `DependentRun` awaits a
/// child run; `ExternalTool` returns control to the API client, which resumes by
/// submitting the tool output (the host never dispatches it). Note
/// `SpawnedChildRun` is deliberately **not** here — it is non-suspending
/// (§5.3 table): the executor appends the child result and continues.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suspension {
    /// A spawned OS process the turn now waits on.
    Process(ProcessRef),
    /// A dependent child run this invocation awaits.
    DependentRun(GateRef),
    /// A client-supplied tool the host does not execute; control returns to the
    /// API client until it submits the output.
    ExternalTool(GateRef),
}

impl Suspension {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            Suspension::Process(_) => "process",
            Suspension::DependentRun(_) => "dependent_run",
            Suspension::ExternalTool(_) => "external_tool",
        }
    }

    /// The gate this suspension parks on, when it is gate-shaped
    /// (`DependentRun`/`ExternalTool`). `Process` suspensions track a process
    /// record instead — see [`Suspension::process_ref`].
    pub fn gate_ref(&self) -> Option<&GateRef> {
        match self {
            Suspension::DependentRun(gate) | Suspension::ExternalTool(gate) => Some(gate),
            Suspension::Process(_) => None,
        }
    }

    /// The process record this suspension tracks, when it is process-shaped.
    pub fn process_ref(&self) -> Option<&ProcessRef> {
        match self {
            Suspension::Process(process) => Some(process),
            Suspension::DependentRun(_) | Suspension::ExternalTool(_) => None,
        }
    }
}

/// The typed verdict of a dispatched capability — success or a recoverable
/// failure — carried by [`Outcome`]. This is the fix for §1.2/§3's
/// "summary-sniffed" outcome: the loop reads a typed verdict, never inspects a
/// prose summary string to guess whether the call succeeded.
///
/// Maps the §5.3 acceptance table: `Completed` → [`ToolVerdict::Success`],
/// `Failed` → [`ToolVerdict::RecoverableFailure`] (model-visible, correctable),
/// `SpawnedChildRun` → [`ToolVerdict::ChildSpawned`] (non-suspending — the
/// executor appends the child result and continues).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVerdict {
    /// The capability ran and succeeded.
    Success,
    /// The capability ran and failed in a model-visible, correctable way — NOT a
    /// `HostFailure` (that is infrastructure, §1.2). The model may retry or adapt.
    RecoverableFailure,
    /// The capability spawned a child run; non-suspending (§5.3 table).
    ChildSpawned,
}

impl ToolVerdict {
    /// Whether the capability completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self, ToolVerdict::Success)
    }

    /// Stable discriminant (matches the serde tag).
    pub fn kind(&self) -> &'static str {
        match self {
            ToolVerdict::Success => "success",
            ToolVerdict::RecoverableFailure => "recoverable_failure",
            ToolVerdict::ChildSpawned => "child_spawned",
        }
    }
}

/// References to a completed capability's durably-stored output. The full bytes
/// stay host-owned and are fetched only through [`ResultRef`]; only bounded
/// metadata rides here (§3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRefs {
    /// Handle to the full stored output.
    pub result: ResultRef,
    /// Size of the staged output in bytes — pure metadata, no PII. Used by the
    /// per-capability byte-cap strategy.
    pub byte_len: u64,
}

/// A dispatched capability's result — tool success OR recoverable failure (§3).
///
/// This is the `Resolution::Done` payload (a later slice adds the `Resolution`
/// umbrella). It pairs with [`crate::Invocation`] as the two ends of a capability
/// call: the request in, the outcome out. The typed [`ToolVerdict`] replaces
/// variant-matching / summary-sniffing; the [`SafeSummary`] is model-visible and
/// redacted; the full output is reached through [`OutcomeRefs`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub refs: OutcomeRefs,
    pub verdict: ToolVerdict,
    pub summary: SafeSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATE_UUID: &str = "01890a5d-ac96-774b-bcce-b302099a8057";
    const PROC_UUID: &str = "0f0e0d0c-0b0a-4908-8706-050403020100";

    fn gate() -> GateRef {
        GateRef::parse(GATE_UUID).unwrap()
    }

    fn proc_ref() -> ProcessRef {
        ProcessRef::parse(PROC_UUID).unwrap()
    }

    #[test]
    fn blocked_serde_is_snake_case_tagged_and_roundtrips() {
        let blocked = Blocked::Approval(gate());
        let json = serde_json::to_value(&blocked).unwrap();
        assert_eq!(json, serde_json::json!({ "approval": GATE_UUID }));
        assert_eq!(serde_json::from_value::<Blocked>(json).unwrap(), blocked);
    }

    #[test]
    fn blocked_kind_matches_serde_tag_and_gate_ref_is_reachable() {
        for (blocked, tag) in [
            (Blocked::Approval(gate()), "approval"),
            (Blocked::Auth(gate()), "auth"),
            (Blocked::Resource(gate()), "resource"),
        ] {
            let wire = serde_json::to_value(&blocked).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(blocked.kind(), tag);
            assert_eq!(tag_on_wire, tag);
            assert_eq!(blocked.gate_ref(), &gate());
        }
    }

    #[test]
    fn only_auth_may_be_surfaced_at_dispatch_time() {
        // §5.3.1: dispatch() may raise only Blocked::Auth. This pins the contract
        // the conformance test (§11.7) will enforce against lanes.
        assert!(Blocked::Auth(gate()).is_dispatch_time_permitted());
        assert!(!Blocked::Approval(gate()).is_dispatch_time_permitted());
        assert!(!Blocked::Resource(gate()).is_dispatch_time_permitted());
    }

    #[test]
    fn suspension_serde_tags_and_kinds_agree() {
        let process = Suspension::Process(proc_ref());
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            serde_json::json!({ "process": PROC_UUID })
        );
        for (suspension, tag) in [
            (Suspension::Process(proc_ref()), "process"),
            (Suspension::DependentRun(gate()), "dependent_run"),
            (Suspension::ExternalTool(gate()), "external_tool"),
        ] {
            let wire = serde_json::to_value(&suspension).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(suspension.kind(), tag);
            assert_eq!(tag_on_wire, tag);
            // Exactly one of the two accessors answers, matching the shape.
            assert_eq!(
                suspension.gate_ref().is_some(),
                tag != "process",
                "gate_ref answers iff gate-shaped: {tag}"
            );
            assert_eq!(suspension.process_ref().is_some(), tag == "process");
        }
    }

    #[test]
    fn tool_verdict_serde_tags_and_kinds_agree() {
        for (verdict, tag) in [
            (ToolVerdict::Success, "success"),
            (ToolVerdict::RecoverableFailure, "recoverable_failure"),
            (ToolVerdict::ChildSpawned, "child_spawned"),
        ] {
            assert_eq!(
                serde_json::to_value(verdict).unwrap(),
                serde_json::Value::String(tag.to_string())
            );
            assert_eq!(verdict.kind(), tag);
        }
        assert!(ToolVerdict::Success.is_success());
        assert!(!ToolVerdict::RecoverableFailure.is_success());
    }

    #[test]
    fn outcome_roundtrips_and_carries_typed_verdict_not_a_sniffed_summary() {
        let outcome = Outcome {
            refs: OutcomeRefs {
                result: ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap(),
                byte_len: 4096,
            },
            verdict: ToolVerdict::RecoverableFailure,
            summary: SafeSummary::new("tool input rejected").unwrap(),
        };
        let json = serde_json::to_value(&outcome).unwrap();
        let back: Outcome = serde_json::from_value(json).unwrap();
        assert_eq!(back, outcome);
        // The verdict is read from the type, never inferred from the summary text.
        assert!(!back.verdict.is_success());
        assert_eq!(back.refs.byte_len, 4096);
    }

    #[test]
    fn outcome_rejects_an_unsafe_summary_on_the_wire() {
        // A hostile persisted summary cannot rehydrate into an Outcome.
        let json = serde_json::json!({
            "refs": { "result": "018f6a00-0000-7000-8000-000000000001", "byte_len": 1 },
            "verdict": "success",
            "summary": "api key: sk-ant-leak"
        });
        let err = serde_json::from_value::<Outcome>(json)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("sensitive marker"),
            "rejection must be for the summary, not an id-shape artifact: {err}"
        );
    }
}
