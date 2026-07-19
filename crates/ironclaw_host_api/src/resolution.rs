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

use crate::{
    DenyRef, FailureKind, GateRef, LoopRef, OutputDigest, ProcessRef, ResultProgress, ResultRef,
    ResumeToken, RunId, SafeSummary, TerminateHint,
};

/// A pending-gate handle plus the additive context needed to resume and correlate
/// it (§5.3 Stage 1). Three parts, each plain redacted vocabulary:
///
/// - `gate` — the opaque kernel [`GateRef`] the pending record is keyed by;
/// - `origin` — the preserved *originating* loop gate ref, so loop/evidence state
///   keyed under it stays reachable once the uuid handle is minted;
/// - `resume` — the opaque [`ResumeToken`] the loop echoes back to re-enter
///   `authorize()`. Populated only for approval/auth gates; a resource gate
///   resumes against *then-current* budget (§5.3.3), not a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateWaypoint {
    /// The opaque kernel handle to the pending gate record.
    pub gate: GateRef,
    /// The preserved originating loop gate ref, when one was carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<LoopRef>,
    /// The opaque gate-resume identity the loop echoes back (approval/auth only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeToken>,
}

impl GateWaypoint {
    /// A bare waypoint carrying only the kernel handle. Use the `with_*` setters
    /// to add the preserved origin and/or resume token (default-backed builder).
    pub fn new(gate: GateRef) -> Self {
        Self {
            gate,
            origin: None,
            resume: None,
        }
    }

    /// Preserve the originating loop gate ref.
    pub fn with_origin(mut self, origin: LoopRef) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Carry the gate-resume token the loop echoes back.
    pub fn with_resume(mut self, resume: ResumeToken) -> Self {
        self.resume = Some(resume);
        self
    }
}

/// A spawned-process handle plus the preserved originating loop process ref
/// (§5.3 Stage 1). Parked-work suspensions resume when the awaited process
/// completes, so — unlike [`GateWaypoint`] — there is no resume token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessWaypoint {
    /// The opaque kernel handle to the spawned-process record.
    pub process: ProcessRef,
    /// The preserved originating loop process ref, when one was carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<LoopRef>,
}

impl ProcessWaypoint {
    /// A bare waypoint carrying only the kernel handle.
    pub fn new(process: ProcessRef) -> Self {
        Self {
            process,
            origin: None,
        }
    }

    /// Preserve the originating loop process ref.
    pub fn with_origin(mut self, origin: LoopRef) -> Self {
        self.origin = Some(origin);
        self
    }
}

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
    Approval(GateWaypoint),
    /// Needs a credential the caller has not supplied (auth gate). The only kind
    /// `dispatch()` may surface (§5.3.1).
    Auth(GateWaypoint),
    /// Needs resource budget currently unavailable.
    Resource(GateWaypoint),
}

impl Blocked {
    /// The gate waypoint (kernel handle + preserved origin + resume token),
    /// regardless of kind.
    pub fn waypoint(&self) -> &GateWaypoint {
        match self {
            Blocked::Approval(w) | Blocked::Auth(w) | Blocked::Resource(w) => w,
        }
    }

    /// The handle to the pending gate record, regardless of kind.
    pub fn gate_ref(&self) -> &GateRef {
        &self.waypoint().gate
    }

    /// The preserved originating loop gate ref, when one was carried.
    pub fn origin(&self) -> Option<&LoopRef> {
        self.waypoint().origin.as_ref()
    }

    /// The gate-resume token the loop echoes back (approval/auth only).
    pub fn resume_token(&self) -> Option<&ResumeToken> {
        self.waypoint().resume.as_ref()
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
    Process(ProcessWaypoint),
    /// A dependent child run this invocation awaits.
    DependentRun(GateWaypoint),
    /// A client-supplied tool the host does not execute; control returns to the
    /// API client until it submits the output.
    ExternalTool(GateWaypoint),
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
            Suspension::DependentRun(w) | Suspension::ExternalTool(w) => Some(&w.gate),
            Suspension::Process(_) => None,
        }
    }

    /// The process record this suspension tracks, when it is process-shaped.
    pub fn process_ref(&self) -> Option<&ProcessRef> {
        match self {
            Suspension::Process(w) => Some(&w.process),
            Suspension::DependentRun(_) | Suspension::ExternalTool(_) => None,
        }
    }

    /// The preserved originating loop ref (gate or process), regardless of kind.
    pub fn origin(&self) -> Option<&LoopRef> {
        match self {
            Suspension::Process(w) => w.origin.as_ref(),
            Suspension::DependentRun(w) | Suspension::ExternalTool(w) => w.origin.as_ref(),
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
///
/// `ChildSpawned` carries the spawned [`RunId`] *on the variant* (was
/// `SpawnedChildRun.child_run_id`) so the invariant "a child ref exists exactly
/// when a child was spawned" is unrepresentable to violate — no optional field
/// to validate at construction or deserialization.
///
/// `RecoverableFailure` carries the recovery classification ([`FailureKind`], was
/// `CapabilityFailure::error_kind`) *on the variant* for the same reason: the
/// class that drives retry-vs-terminal exists exactly when the verdict is a
/// recoverable failure. `FailureKind` is a bounded taxonomy, never the raw backend
/// cause — that stays host-side.
///
/// Not `Copy` (unlike the earlier slice): `FailureKind::Unknown` owns a `String`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVerdict {
    /// The capability ran and succeeded.
    Success,
    /// The capability ran and failed in a model-visible, correctable way — NOT a
    /// `HostFailure` (that is infrastructure, §1.2). The model may retry or adapt.
    /// Carries the [`FailureKind`] recovery classification.
    RecoverableFailure { error_kind: FailureKind },
    /// The capability spawned a child run; non-suspending (§5.3 table). Carries
    /// the child's [`RunId`] — a correlation ref, safe on the sanitized boundary.
    ChildSpawned { child_run: RunId },
}

impl ToolVerdict {
    /// Whether the capability completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self, ToolVerdict::Success)
    }

    /// The recovery classification, present exactly on
    /// [`ToolVerdict::RecoverableFailure`].
    pub fn error_kind(&self) -> Option<&FailureKind> {
        match self {
            ToolVerdict::RecoverableFailure { error_kind } => Some(error_kind),
            _ => None,
        }
    }

    /// The spawned child run, present exactly on [`ToolVerdict::ChildSpawned`].
    pub fn child_run(&self) -> Option<RunId> {
        match self {
            ToolVerdict::ChildSpawned { child_run } => Some(*child_run),
            _ => None,
        }
    }

    /// Stable discriminant (matches the serde tag).
    pub fn kind(&self) -> &'static str {
        match self {
            ToolVerdict::Success => "success",
            ToolVerdict::RecoverableFailure { .. } => "recoverable_failure",
            ToolVerdict::ChildSpawned { .. } => "child_spawned",
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
    /// Bounded, model-visible result preview (was `model_observation`). Redacted
    /// by construction; the full bytes stay host-owned behind `result`. `None`
    /// when no preview is staged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<SafeSummary>,
    /// The preserved originating loop result ref, so output the loop staged under
    /// its own ref stays reachable once `result` (a uuid handle) is minted. `None`
    /// when the outcome had no originating loop result ref (e.g. a recoverable
    /// failure stages nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<LoopRef>,
    /// Stable digest over the normalized output content (was
    /// `CapabilityResultMessage::output_digest`) — a fixed-width hash, never the
    /// content. `None` for synthetic results that stage no real output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_digest: Option<OutputDigest>,
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
    /// Loop-derived signal describing whether this result advanced the loop's
    /// evidence/state (was `CapabilityResultMessage::progress`). Defaults to
    /// [`ResultProgress::Unknown`] for outcomes that carry no progress signal
    /// (a recoverable failure, a spawned child).
    #[serde(default, skip_serializing_if = "is_default_progress")]
    pub progress: ResultProgress,
    /// Host hint that the loop should end naturally after the current batch (was
    /// `CapabilityResultMessage::terminate_hint`). Defaults to
    /// [`TerminateHint::Continue`].
    #[serde(default, skip_serializing_if = "is_default_terminate_hint")]
    pub terminate_hint: TerminateHint,
}

fn is_default_progress(progress: &ResultProgress) -> bool {
    *progress == ResultProgress::default()
}

fn is_default_terminate_hint(hint: &TerminateHint) -> bool {
    *hint == TerminateHint::default()
}

/// The composed answer of one capability invocation — the single value
/// `AgentLoopHost::invoke` returns in its `Ok` arm (§3, §5.4); the `Err` arm is
/// [`crate::HostFailure`]. This is the **five-channel** replacement for today's
/// overloaded ten-variant `CapabilityOutcome` (§1.2): each channel is a distinct
/// type, so a recoverable result, a terminal denial, a re-entrant gate, and
/// parked work can never be confused for one another.
///
/// The §5.3 acceptance table (the definition of done) maps every one of today's
/// ten `CapabilityOutcome` variants to exactly one channel here — see the
/// `resolution_covers_the_full_acceptance_table` test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    /// The capability ran: success or recoverable failure (typed by the
    /// [`Outcome`]'s [`ToolVerdict`]).
    Done(Outcome),
    /// Terminal policy denial — model-visible, **not** re-entrant (distinct from
    /// every gate). `AuthorizeResult::Denied` folds into this (§3).
    Denied(DenyRef),
    /// A re-entrant gate: resolve it and the invocation may run.
    Blocked(Blocked),
    /// Parked work: the effect is in flight or handed off.
    Suspended(Suspension),
}

impl Resolution {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            Resolution::Done(_) => "done",
            Resolution::Denied(_) => "denied",
            Resolution::Blocked(_) => "blocked",
            Resolution::Suspended(_) => "suspended",
        }
    }

    /// Whether the turn parks on this resolution. ONLY [`Resolution::Suspended`]
    /// suspends — a `Done` with a `ChildSpawned` verdict is explicitly
    /// non-suspending (§5.3 table: the executor appends the child result and
    /// continues). Getting this wrong is the #6137 bug class.
    pub fn is_suspension(&self) -> bool {
        matches!(self, Resolution::Suspended(_))
    }

    /// Whether this is a re-entrant gate (resolving it re-enters `authorize()`).
    /// A `Denied` is terminal and is deliberately excluded.
    pub fn is_reentrant_gate(&self) -> bool {
        matches!(self, Resolution::Blocked(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATE_UUID: &str = "01890a5d-ac96-774b-bcce-b302099a8057";
    const PROC_UUID: &str = "0f0e0d0c-0b0a-4908-8706-050403020100";

    fn gate() -> GateRef {
        GateRef::parse(GATE_UUID).unwrap()
    }

    fn gate_wp() -> GateWaypoint {
        GateWaypoint::new(gate())
    }

    fn proc_ref() -> ProcessRef {
        ProcessRef::parse(PROC_UUID).unwrap()
    }

    fn proc_wp() -> ProcessWaypoint {
        ProcessWaypoint::new(proc_ref())
    }

    #[test]
    fn blocked_serde_is_snake_case_tagged_and_roundtrips() {
        let blocked = Blocked::Approval(gate_wp());
        let json = serde_json::to_value(&blocked).unwrap();
        // A bare waypoint (no origin/resume) serializes as just the gate handle.
        assert_eq!(
            json,
            serde_json::json!({ "approval": { "gate": GATE_UUID } })
        );
        assert_eq!(serde_json::from_value::<Blocked>(json).unwrap(), blocked);
    }

    #[test]
    fn blocked_kind_matches_serde_tag_and_gate_ref_is_reachable() {
        for (blocked, tag) in [
            (Blocked::Approval(gate_wp()), "approval"),
            (Blocked::Auth(gate_wp()), "auth"),
            (Blocked::Resource(gate_wp()), "resource"),
        ] {
            let wire = serde_json::to_value(&blocked).unwrap();
            let tag_on_wire = wire.as_object().unwrap().keys().next().unwrap().clone();
            assert_eq!(blocked.kind(), tag);
            assert_eq!(tag_on_wire, tag);
            assert_eq!(blocked.gate_ref(), &gate());
        }
    }

    #[test]
    fn blocked_waypoint_carries_preserved_origin_and_resume_token() {
        let waypoint = GateWaypoint::new(gate())
            .with_origin(LoopRef::new("gate:approval-1").unwrap())
            .with_resume(ResumeToken::new("resume-1").unwrap());
        let blocked = Blocked::Approval(waypoint);
        assert_eq!(
            blocked.origin().map(LoopRef::as_str),
            Some("gate:approval-1")
        );
        assert_eq!(
            blocked.resume_token().map(ResumeToken::as_str),
            Some("resume-1")
        );
        // The preserved fields survive a wire round-trip.
        let back: Blocked =
            serde_json::from_value(serde_json::to_value(&blocked).unwrap()).unwrap();
        assert_eq!(back, blocked);
    }

    #[test]
    fn only_auth_may_be_surfaced_at_dispatch_time() {
        // §5.3.1: dispatch() may raise only Blocked::Auth. This pins the contract
        // the conformance test (§11.7) will enforce against lanes.
        assert!(Blocked::Auth(gate_wp()).is_dispatch_time_permitted());
        assert!(!Blocked::Approval(gate_wp()).is_dispatch_time_permitted());
        assert!(!Blocked::Resource(gate_wp()).is_dispatch_time_permitted());
    }

    #[test]
    fn suspension_serde_tags_and_kinds_agree() {
        let process = Suspension::Process(proc_wp());
        assert_eq!(
            serde_json::to_value(&process).unwrap(),
            serde_json::json!({ "process": { "process": PROC_UUID } })
        );
        for (suspension, tag) in [
            (Suspension::Process(proc_wp()), "process"),
            (Suspension::DependentRun(gate_wp()), "dependent_run"),
            (Suspension::ExternalTool(gate_wp()), "external_tool"),
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
        // Success is a unit variant; the other two are struct variants (each
        // carries its invariant field on the variant).
        assert_eq!(
            serde_json::to_value(ToolVerdict::Success).unwrap(),
            serde_json::Value::String("success".to_string())
        );
        assert_eq!(ToolVerdict::Success.kind(), "success");
        let failure = ToolVerdict::RecoverableFailure {
            error_kind: FailureKind::InvalidInput,
        };
        assert_eq!(failure.kind(), "recoverable_failure");
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            serde_json::json!({ "recoverable_failure": { "error_kind": "invalid_input" } })
        );
        assert!(ToolVerdict::Success.is_success());
        assert!(!failure.is_success());
        assert_eq!(ToolVerdict::Success.child_run(), None);
        assert_eq!(ToolVerdict::Success.error_kind(), None);
        assert_eq!(failure.error_kind(), Some(&FailureKind::InvalidInput));
    }

    #[test]
    fn recoverable_failure_carries_its_error_kind_across_the_wire() {
        // The recovery classification (retry-vs-terminal) survives round-trip —
        // the field the old mapping dropped as "G1".
        for kind in [
            FailureKind::Network,
            FailureKind::unknown("quota_exceeded").unwrap(),
        ] {
            let verdict = ToolVerdict::RecoverableFailure {
                error_kind: kind.clone(),
            };
            let back: ToolVerdict =
                serde_json::from_value(serde_json::to_value(&verdict).unwrap()).unwrap();
            assert_eq!(back.error_kind(), Some(&kind));
        }
    }

    #[test]
    fn child_spawned_verdict_carries_the_run_on_the_variant() {
        let run = RunId::parse("018f6a00-0000-7000-8000-0000000000aa").unwrap();
        let verdict = ToolVerdict::ChildSpawned { child_run: run };
        assert_eq!(verdict.kind(), "child_spawned");
        assert_eq!(verdict.child_run(), Some(run));
        // Struct variant: externally tagged with the snake_case tag; the child
        // ref exists exactly when a child was spawned — there is no optional
        // field whose consistency needs validating.
        let wire = serde_json::to_value(&verdict).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "child_spawned": { "child_run": "018f6a00-0000-7000-8000-0000000000aa" }
            })
        );
        let back: ToolVerdict = serde_json::from_value(wire).unwrap();
        assert_eq!(back, verdict);
        // A child_spawned tag WITHOUT the run cannot deserialize at all.
        assert!(
            serde_json::from_value::<ToolVerdict>(serde_json::json!({ "child_spawned": {} }))
                .is_err(),
            "missing child_run must be structurally unrepresentable"
        );
    }

    #[test]
    fn outcome_roundtrips_and_carries_typed_verdict_not_a_sniffed_summary() {
        let outcome = Outcome {
            refs: OutcomeRefs {
                result: ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap(),
                byte_len: 4096,
                preview: None,
                origin: None,
                output_digest: None,
            },
            verdict: ToolVerdict::RecoverableFailure {
                error_kind: FailureKind::InvalidInput,
            },
            summary: SafeSummary::new("tool input rejected").unwrap(),
            progress: ResultProgress::default(),
            terminate_hint: TerminateHint::default(),
        };
        let json = serde_json::to_value(&outcome).unwrap();
        let back: Outcome = serde_json::from_value(json).unwrap();
        assert_eq!(back, outcome);
        // The verdict is read from the type, never inferred from the summary text.
        assert!(!back.verdict.is_success());
        assert_eq!(back.refs.byte_len, 4096);
    }

    #[test]
    fn outcome_carries_progress_terminate_hint_and_digest_when_populated() {
        // The G4 completion signals survive a wire round-trip.
        let outcome = Outcome {
            refs: OutcomeRefs {
                result: ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap(),
                byte_len: 10,
                preview: None,
                origin: Some(LoopRef::new("result:child-1").unwrap()),
                output_digest: Some(OutputDigest::new(0xABCD)),
            },
            verdict: ToolVerdict::Success,
            summary: SafeSummary::new("read 3 files").unwrap(),
            progress: ResultProgress::MadeProgress,
            terminate_hint: TerminateHint::TerminateAfterBatch,
        };
        let back: Outcome =
            serde_json::from_value(serde_json::to_value(&outcome).unwrap()).unwrap();
        assert_eq!(back, outcome);
        assert_eq!(back.progress, ResultProgress::MadeProgress);
        assert!(back.terminate_hint.should_terminate());
        assert_eq!(
            back.refs.output_digest.map(OutputDigest::value),
            Some(0xABCD)
        );
        assert_eq!(
            back.refs.origin.as_ref().map(LoopRef::as_str),
            Some("result:child-1")
        );
    }

    #[test]
    fn outcome_refs_roundtrip_with_optional_preview() {
        let result = ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap();
        // Populated: the preview is carried.
        let full = OutcomeRefs {
            result,
            byte_len: 128,
            preview: Some(SafeSummary::new("staged 3 rows").unwrap()),
            origin: None,
            output_digest: None,
        };
        let back: OutcomeRefs =
            serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back, full);
        assert_eq!(
            back.preview.as_ref().map(SafeSummary::as_str),
            Some("staged 3 rows")
        );

        // Absent: None omitted from the wire (skip_serializing_if), and a
        // legacy payload without the field rehydrates to None.
        let bare = OutcomeRefs {
            result,
            byte_len: 128,
            preview: None,
            origin: None,
            output_digest: None,
        };
        let wire = serde_json::to_value(&bare).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "result": "018f6a00-0000-7000-8000-000000000001",
                "byte_len": 128
            }),
            "None additive fields must not appear on the wire"
        );
        let back: OutcomeRefs = serde_json::from_value(wire).unwrap();
        assert_eq!(back, bare);
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

    fn child_spawned() -> ToolVerdict {
        ToolVerdict::ChildSpawned {
            child_run: RunId::parse("018f6a00-0000-7000-8000-0000000000aa").unwrap(),
        }
    }

    fn recoverable_failure() -> ToolVerdict {
        ToolVerdict::RecoverableFailure {
            error_kind: FailureKind::InvalidInput,
        }
    }

    fn outcome(verdict: ToolVerdict) -> Outcome {
        Outcome {
            refs: OutcomeRefs {
                result: ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap(),
                byte_len: 1,
                preview: None,
                origin: None,
                output_digest: None,
            },
            verdict,
            summary: SafeSummary::new("ok").unwrap(),
            progress: ResultProgress::default(),
            terminate_hint: TerminateHint::default(),
        }
    }

    /// The §5.3 acceptance table is the definition of done: every one of today's
    /// ten `CapabilityOutcome` variants maps to exactly one `Resolution` channel,
    /// with the right suspension semantics. `host_api` cannot import the turns
    /// enum, so this pins the mapping by (today's variant → channel + suspends?).
    #[test]
    fn resolution_covers_the_full_acceptance_table() {
        let proc = || ProcessRef::parse("0f0e0d0c-0b0a-4908-8706-050403020100").unwrap();
        // (today's CapabilityOutcome variant, the Resolution channel, is_suspension)
        let rows: [(&str, Resolution, bool); 10] = [
            (
                "Completed",
                Resolution::Done(outcome(ToolVerdict::Success)),
                false,
            ),
            (
                "Failed",
                Resolution::Done(outcome(recoverable_failure())),
                false,
            ),
            (
                "Denied",
                Resolution::Denied(DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap()),
                false,
            ),
            (
                "ApprovalRequired",
                Resolution::Blocked(Blocked::Approval(GateWaypoint::new(gate()))),
                false,
            ),
            (
                "AuthRequired",
                Resolution::Blocked(Blocked::Auth(GateWaypoint::new(gate()))),
                false,
            ),
            (
                "ResourceBlocked",
                Resolution::Blocked(Blocked::Resource(GateWaypoint::new(gate()))),
                false,
            ),
            (
                "SpawnedProcess",
                Resolution::Suspended(Suspension::Process(ProcessWaypoint::new(proc()))),
                true,
            ),
            // Non-suspending — the one that has bitten before (#6137, §5.3 table).
            (
                "SpawnedChildRun",
                Resolution::Done(outcome(child_spawned())),
                false,
            ),
            (
                "AwaitDependentRun",
                Resolution::Suspended(Suspension::DependentRun(GateWaypoint::new(gate()))),
                true,
            ),
            (
                "ExternalToolPending",
                Resolution::Suspended(Suspension::ExternalTool(GateWaypoint::new(gate()))),
                true,
            ),
        ];
        assert_eq!(rows.len(), 10, "all ten CapabilityOutcome variants covered");
        for (variant, resolution, suspends) in rows {
            assert_eq!(
                resolution.is_suspension(),
                suspends,
                "{variant}: suspension semantics"
            );
            // Round-trips through the wire like every channel.
            let json = serde_json::to_value(&resolution).unwrap();
            let back: Resolution = serde_json::from_value(json).unwrap();
            assert_eq!(back, resolution, "{variant}: round-trip");
        }
    }

    #[test]
    fn resolution_channel_predicates() {
        assert!(
            Resolution::Suspended(Suspension::Process(ProcessWaypoint::new(
                ProcessRef::parse("0f0e0d0c-0b0a-4908-8706-050403020100").unwrap()
            )))
            .is_suspension()
        );
        assert!(Resolution::Blocked(Blocked::Approval(gate_wp())).is_reentrant_gate());
        // Denied is terminal — not a re-entrant gate, not a suspension.
        let denied =
            Resolution::Denied(DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap());
        assert!(!denied.is_reentrant_gate());
        assert!(!denied.is_suspension());
        // A spawned child run completes; it does not suspend.
        assert!(!Resolution::Done(outcome(child_spawned())).is_suspension());
    }

    /// Table-driven wire contract: every `Resolution` channel paired with its
    /// exact serialized shape, so a future serde-attribute change cannot
    /// silently break the API contract for any variant.
    #[test]
    fn resolution_serialization_contract_covers_every_channel() {
        let gate = GateRef::parse("01890a5d-ac96-774b-bcce-b302099a8057").unwrap();
        let deny = DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap();
        let proc = ProcessRef::parse("0f0e0d0c-0b0a-4908-8706-050403020100").unwrap();
        let cases = [
            (
                Resolution::Done(outcome(ToolVerdict::Success)),
                serde_json::json!({
                    "done": {
                        "refs": {
                            "result": "018f6a00-0000-7000-8000-000000000001",
                            "byte_len": 1
                        },
                        "verdict": "success",
                        "summary": "ok"
                    }
                }),
            ),
            (
                Resolution::Denied(deny),
                serde_json::json!({ "denied": "018f6a00-0000-7000-8000-000000000002" }),
            ),
            (
                Resolution::Blocked(Blocked::Approval(GateWaypoint::new(gate))),
                serde_json::json!({
                    "blocked": { "approval": { "gate": "01890a5d-ac96-774b-bcce-b302099a8057" } }
                }),
            ),
            (
                Resolution::Suspended(Suspension::Process(ProcessWaypoint::new(proc))),
                serde_json::json!({
                    "suspended": { "process": { "process": "0f0e0d0c-0b0a-4908-8706-050403020100" } }
                }),
            ),
        ];
        for (resolution, expected_wire) in cases {
            let wire = serde_json::to_value(&resolution).unwrap();
            assert_eq!(wire, expected_wire, "wire drift for {resolution:?}");
            assert_eq!(
                serde_json::from_value::<Resolution>(wire).unwrap(),
                resolution,
                "round-trip must reconstruct the same channel"
            );
        }
    }
}
