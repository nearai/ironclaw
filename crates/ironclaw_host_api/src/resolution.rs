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

use crate::{GateRef, ProcessRef};

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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> GateRef {
        GateRef::new("gate-01HZ0000000000000000000000").unwrap()
    }

    #[test]
    fn blocked_serde_is_snake_case_tagged_and_roundtrips() {
        let blocked = Blocked::Approval(gate());
        let json = serde_json::to_value(&blocked).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "approval": "gate-01HZ0000000000000000000000" })
        );
        assert_eq!(
            serde_json::from_value::<Blocked>(json).unwrap(),
            blocked
        );
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
        let process =
            Suspension::Process(ProcessRef::new("proc-01HZ0000000000000000000000").unwrap());
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            serde_json::json!({ "process": "proc-01HZ0000000000000000000000" })
        );
        for (suspension, tag) in [
            (
                Suspension::Process(ProcessRef::new("proc-1").unwrap()),
                "process",
            ),
            (Suspension::DependentRun(gate()), "dependent_run"),
            (Suspension::ExternalTool(gate()), "external_tool"),
        ] {
            let wire = serde_json::to_value(&suspension).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(suspension.kind(), tag);
            assert_eq!(tag_on_wire, tag);
        }
    }
}
