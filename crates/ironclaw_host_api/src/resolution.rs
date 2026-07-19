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
    DenyReason, DenyRef, FailureKind, GateRef, LoopRef, ModelFailureDiagnostic, ModelResultPreview,
    OutputDigest, ProcessRef, ResultProgress, ResultRef, ResumeToken, RunId, SafeSummary,
    TerminateHint,
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

/// The dependent child's staged result, carried **inline** on
/// [`Suspension::DependentRun`] so the loop observes the child's output on
/// resume without reading host storage — the loop cannot read the durable
/// [`GateRecord::DependentRun`](crate::GateRecord) sidecar. This mirrors the way
/// [`Resolution::Done`] carries a spawned child run's content on its [`Outcome`]:
/// the record is the durable copy, this is the loop-visible one.
///
/// Every field is plain redacted vocabulary per the host_api charter — bounded
/// metadata (`byte_len`), a redacted model-visible [`SafeSummary`], a redacted
/// model-visible observation preview, and the preserved originating loop result
/// ref. The full child bytes stay host-owned behind the record's [`ResultRef`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependentRunResult {
    /// Size of the staged child output in bytes — pure metadata, no PII. Feeds
    /// the per-capability byte-cap strategy (was
    /// `CapabilityResultMessage::byte_len`).
    pub byte_len: u64,
    /// Redacted, model-visible summary of the child result.
    pub summary: SafeSummary,
    /// Redacted, model-visible observation preview of the child result (was
    /// `model_observation`, previously dropped on this channel). Redacted by
    /// construction; the full bytes stay host-owned. `None` when no preview is
    /// staged or the raw observation failed redaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<SafeSummary>,
    /// The preserved originating loop result ref, so child output the loop
    /// staged under its own ref stays reachable once the host [`ResultRef`]
    /// handle is minted. `None` when there was no originating loop result ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<LoopRef>,
}

impl DependentRunResult {
    /// A staged result carrying only the byte length and redacted summary. Use
    /// the `with_*` setters to add the optional observation preview and
    /// preserved origin (default-backed builder).
    pub fn new(byte_len: u64, summary: SafeSummary) -> Self {
        Self {
            byte_len,
            summary,
            observation: None,
            origin: None,
        }
    }

    /// Carry the redacted, model-visible observation preview.
    pub fn with_observation(mut self, observation: SafeSummary) -> Self {
        self.observation = Some(observation);
        self
    }

    /// Preserve the originating loop result ref.
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
    /// A dependent child run this invocation awaits. Carries the gate
    /// `waypoint` it parks on **and** the child's staged [`DependentRunResult`]
    /// inline, so the loop observes the child's output on resume without
    /// reading host storage (mirrors how [`Resolution::Done`] carries a spawned
    /// child run's content; the durable copy is the
    /// [`GateRecord::DependentRun`](crate::GateRecord) sidecar).
    DependentRun {
        waypoint: GateWaypoint,
        result: DependentRunResult,
    },
    /// A client-supplied tool the host does not execute; control returns to the
    /// API client until it submits the output.
    ExternalTool(GateWaypoint),
}

impl Suspension {
    /// Stable discriminant (matches the serde tag) for logs/routing.
    pub fn kind(&self) -> &'static str {
        match self {
            Suspension::Process(_) => "process",
            Suspension::DependentRun { .. } => "dependent_run",
            Suspension::ExternalTool(_) => "external_tool",
        }
    }

    /// The gate this suspension parks on, when it is gate-shaped
    /// (`DependentRun`/`ExternalTool`). `Process` suspensions track a process
    /// record instead — see [`Suspension::process_ref`].
    pub fn gate_ref(&self) -> Option<&GateRef> {
        match self {
            Suspension::DependentRun { waypoint, .. } => Some(&waypoint.gate),
            Suspension::ExternalTool(w) => Some(&w.gate),
            Suspension::Process(_) => None,
        }
    }

    /// The process record this suspension tracks, when it is process-shaped.
    pub fn process_ref(&self) -> Option<&ProcessRef> {
        match self {
            Suspension::Process(w) => Some(&w.process),
            Suspension::DependentRun { .. } | Suspension::ExternalTool(_) => None,
        }
    }

    /// The preserved originating loop ref (gate or process), regardless of kind.
    pub fn origin(&self) -> Option<&LoopRef> {
        match self {
            Suspension::Process(w) => w.origin.as_ref(),
            Suspension::DependentRun { waypoint, .. } => waypoint.origin.as_ref(),
            Suspension::ExternalTool(w) => w.origin.as_ref(),
        }
    }

    /// The dependent child's staged result, present exactly on
    /// [`Suspension::DependentRun`]. This is the loop-visible copy the executor
    /// reads on resume; the durable copy lives in the
    /// [`GateRecord::DependentRun`](crate::GateRecord) sidecar.
    pub fn dependent_result(&self) -> Option<&DependentRunResult> {
        match self {
            Suspension::DependentRun { result, .. } => Some(result),
            Suspension::Process(_) | Suspension::ExternalTool(_) => None,
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
    /// Carries the [`FailureKind`] recovery classification, and — additively — the
    /// redacted, model-visible [`ModelFailureDiagnostic`] the model corrects from
    /// (the structured `InvalidInput` issues or a redacted free-text cause), so a
    /// later slice can render the tool error without reading host storage. `None`
    /// when the producer supplied no structured diagnostic.
    RecoverableFailure {
        error_kind: FailureKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diagnostic: Option<ModelFailureDiagnostic>,
    },
    /// The capability spawned a child run; non-suspending (§5.3 table). Carries
    /// the child's [`RunId`] — a correlation ref, safe on the sanitized boundary.
    ChildSpawned { child_run: RunId },
}

impl ToolVerdict {
    /// A recoverable failure carrying only its recovery classification (no
    /// structured diagnostic). Use [`ToolVerdict::recoverable_failure_with_diagnostic`]
    /// to attach the model-visible [`ModelFailureDiagnostic`].
    pub fn recoverable_failure(error_kind: FailureKind) -> Self {
        Self::RecoverableFailure {
            error_kind,
            diagnostic: None,
        }
    }

    /// A recoverable failure carrying its recovery classification and the redacted,
    /// model-visible diagnostic the model corrects from.
    pub fn recoverable_failure_with_diagnostic(
        error_kind: FailureKind,
        diagnostic: ModelFailureDiagnostic,
    ) -> Self {
        Self::RecoverableFailure {
            error_kind,
            diagnostic: Some(diagnostic),
        }
    }

    /// Whether the capability completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self, ToolVerdict::Success)
    }

    /// The recovery classification, present exactly on
    /// [`ToolVerdict::RecoverableFailure`].
    pub fn error_kind(&self) -> Option<&FailureKind> {
        match self {
            ToolVerdict::RecoverableFailure { error_kind, .. } => Some(error_kind),
            _ => None,
        }
    }

    /// The redacted, model-visible diagnostic, present only on a
    /// [`ToolVerdict::RecoverableFailure`] whose producer supplied one.
    pub fn diagnostic(&self) -> Option<&ModelFailureDiagnostic> {
        match self {
            ToolVerdict::RecoverableFailure { diagnostic, .. } => diagnostic.as_ref(),
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
    /// Bounded, model-visible result CONTENT preview — the #5838 first-look
    /// inline preview (was the `ResultReference` observation's `detail.preview`).
    /// A [`ModelResultPreview`], NOT a [`SafeSummary`]: it carries the tool's own
    /// output (delimiters, JSON, newlines), credential-redacted at a word
    /// boundary, up to 24 KiB — so the model sees the result inline without a
    /// follow-up `result_read`. The full bytes stay host-owned behind `result`;
    /// `None` when no preview is staged or the content failed the credential
    /// redaction contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<ModelResultPreview>,
    /// Continuation metadata for a TRUNCATED first-look preview so the model can
    /// read the full result (`result_read`, large results): the referenced ref,
    /// full byte size, next offset, and JSON-array element count. Empty (all
    /// `None`) for a complete inline preview or no preview.
    #[serde(default, skip_serializing_if = "ResultPreviewMeta::is_empty")]
    pub preview_meta: ResultPreviewMeta,
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

/// Continuation metadata for a truncated first-look result preview (§5838). All
/// fields default to `None` — an empty value means the preview (if any) is the
/// complete result and needs no `result_read` follow-up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResultPreviewMeta {
    /// The result ref the preview is OF. A `result_read` reading ANOTHER result
    /// presents that original's ref (so the model continues reading the original,
    /// not the read's own chunk output); for a normal completed result it equals
    /// the outcome's own result ref. `None` => the outcome's own result
    /// (`OutcomeRefs::origin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referenced_result_ref: Option<LoopRef>,
    /// Full byte size of the referenced result when the preview is a truncated
    /// chunk; `None` for a complete inline preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    /// Byte offset to continue reading from for a truncated preview; `None` when
    /// the preview is the complete result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    /// Element count when the full result is a top-level JSON array (truncated
    /// previews only), so the model does not misread a byte-sliced array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    /// The observation's own model-visible summary caption — the host-authored
    /// text describing the preview (e.g. "preview truncated, use result_read …").
    /// It is DISTINCT from the completed [`Outcome::summary`] caption (the result
    /// message's `safe_summary`, often a generic "capability completed"): the
    /// producer authors a richer summary on the `ResultReference` observation, and
    /// the collapse would otherwise drop it, so the reconstructed observation would
    /// fall back to the generic caption and lose the truncation/continuation hint.
    /// Carried here so the executor rebuilds the observation with the producer's
    /// exact summary. `None` when the observation carried none or it failed the
    /// caption redaction contract (best-effort; the outcome caption stands in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SafeSummary>,
}

impl ResultPreviewMeta {
    /// True when no continuation metadata is present (the preview is complete or
    /// absent) — the `skip_serializing_if` predicate keeping the wire clean.
    pub fn is_empty(&self) -> bool {
        self.referenced_result_ref.is_none()
            && self.total_bytes.is_none()
            && self.next_offset.is_none()
            && self.item_count.is_none()
            && self.summary.is_none()
    }
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

/// A terminal policy denial's channel payload: the opaque [`DenyRef`] plus the
/// additive, model-visible denial content the loop renders to the model (§5.2.9 /
/// §5.3 flip prep). The full denial record stays host-owned behind `deny`; the
/// `reason_kind` + `summary` here are the redacted subset the loop needs so it can
/// render the denial WITHOUT reading the host-persisted `DenyRecord` (which it
/// cannot reach).
///
/// Both additive fields are plain redacted vocabulary: [`DenyReason`] is already a
/// model-visible closed enum, and [`SafeSummary`] is redacted by construction.
/// They mirror the sibling `DenyRecord` the same ref points at — the channel is a
/// model-visible projection of the host-owned record, not a second source of
/// truth. `None` on both when a producer supplied only the bare ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Denial {
    /// The opaque handle to the host-owned denial record.
    pub deny: DenyRef,
    /// The structured, model-visible reason the action was denied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<DenyReason>,
    /// A bounded, redacted, model-visible summary of the denial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SafeSummary>,
}

impl Denial {
    /// A bare denial carrying only the opaque ref. Use the `with_*` setters to
    /// attach the model-visible reason and summary (default-backed builder).
    pub fn new(deny: DenyRef) -> Self {
        Self {
            deny,
            reason_kind: None,
            summary: None,
        }
    }

    /// Attach the model-visible denial reason.
    pub fn with_reason_kind(mut self, reason_kind: DenyReason) -> Self {
        self.reason_kind = Some(reason_kind);
        self
    }

    /// Attach the redacted, model-visible denial summary.
    pub fn with_summary(mut self, summary: SafeSummary) -> Self {
        self.summary = Some(summary);
        self
    }
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
    /// every gate). `AuthorizeResult::Denied` folds into this (§3). Carries the
    /// [`Denial`] channel payload: the opaque ref plus the redacted reason/summary
    /// the loop renders from.
    Denied(Denial),
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
    ///
    /// This is the *narrow* suspension question ("is this the parked-work
    /// channel?"). A batch loop that stops early on the first invocation that
    /// cannot be followed by more work must use [`Resolution::parks`] instead —
    /// it also stops on a re-entrant gate, which `is_suspension` deliberately
    /// excludes.
    pub fn is_suspension(&self) -> bool {
        matches!(self, Resolution::Suspended(_))
    }

    /// Whether the batch loop *parks* on this resolution — the loop-semantics
    /// suspension predicate the §5.3 flip's batch loops must gate on, a strict
    /// SUPERSET of [`Resolution::is_suspension`].
    ///
    /// A batch that stops "on first suspension" must also stop on a re-entrant
    /// gate ([`Resolution::Blocked`] — approval/auth/resource): the gated
    /// invocation did not run, so nothing after it in the batch can proceed
    /// until it is resolved, exactly as parked work ([`Resolution::Suspended`] —
    /// process/dependent-run/external-tool) blocks the batch. `parks()` answers
    /// the *loop* question "does the batch stop here?"; `is_suspension()` answers
    /// the narrower "is this the parked-work channel?".
    ///
    /// `parks()` ⊋ `is_suspension()`: every `Suspended` parks, but a
    /// `Blocked::Approval` parks while it is NOT a suspension. Using
    /// `is_suspension()` where `parks()` is meant silently lets a gate fall
    /// through as if the call had completed — the verified hazard §5.3 Stage 1
    /// closes ahead of the atomic flip. `Done` (ran, including the non-suspending
    /// `ChildSpawned`) and terminal `Denied` do not park.
    pub fn parks(&self) -> bool {
        // Exhaustive match, not `matches!`: a new `Resolution` variant must be
        // a compile error here (§11.9 no-wildcard) so its parking behavior is
        // decided deliberately, never silently defaulted to `false`.
        match self {
            Resolution::Blocked(_) | Resolution::Suspended(_) => true,
            Resolution::Done(_) | Resolution::Denied(_) => false,
        }
    }

    /// Whether this is a re-entrant gate (resolving it re-enters `authorize()`).
    /// A `Denied` is terminal and is deliberately excluded.
    pub fn is_reentrant_gate(&self) -> bool {
        matches!(self, Resolution::Blocked(_))
    }

    /// The terminal denial payload, present exactly on [`Resolution::Denied`].
    pub fn denial(&self) -> Option<&Denial> {
        match self {
            Resolution::Denied(denial) => Some(denial),
            _ => None,
        }
    }
}

/// The loop-facing result of a *batch* of capability invocations — the §5.3
/// flip's [`Resolution`]-over-`CapabilityOutcome` replacement for the loop's
/// current `CapabilityBatchOutcome` (`ironclaw_turns`). It mirrors that type's
/// shape and semantics exactly: the per-invocation [`Resolution`]s in call
/// order, plus whether the executor stopped early because one of them *parked*.
///
/// The stop flag keeps its established name, `stopped_on_suspension`, to match
/// `CapabilityBatchOutcome`; note the flip's loop must set it whenever an
/// invocation [`Resolution::parks`] — a re-entrant gate as well as a suspension —
/// not only on the narrower [`Resolution::is_suspension`]. That is precisely the
/// predicate this slice provides so the flip has one canonical, tested site to
/// call.
///
/// Additive (§9): nothing produces or consumes a `ResolutionBatch` yet — the
/// flip's batch loops adopt it. Wire-stable serde (a bounded `Vec` + `bool`, per
/// the `host_api` charter) so a persisted or transported batch keeps its
/// contract if it is ever serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionBatch {
    /// The resolution for each invocation, in call order.
    pub resolutions: Vec<Resolution>,
    /// True when the batch stopped early because an invocation parked
    /// ([`Resolution::parks`]) and the batch was configured to stop on the first
    /// such park.
    pub stopped_on_suspension: bool,
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

    fn dep_result() -> DependentRunResult {
        DependentRunResult::new(256, SafeSummary::new("child staged 4 rows").unwrap())
    }

    fn dep_run() -> Suspension {
        Suspension::DependentRun {
            waypoint: gate_wp(),
            result: dep_result(),
        }
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
            (dep_run(), "dependent_run"),
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
        let failure = ToolVerdict::recoverable_failure(FailureKind::InvalidInput);
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
    fn recoverable_failure_carries_its_model_visible_diagnostic() {
        // The structured, model-visible diagnostic rides the verdict so a later
        // slice can render the correction hint without reading host storage.
        let diagnostic = ModelFailureDiagnostic::Diagnostic {
            text: SafeSummary::new("tool input rejected").unwrap(),
        };
        let verdict = ToolVerdict::RecoverableFailure {
            error_kind: FailureKind::InvalidInput,
            diagnostic: Some(diagnostic.clone()),
        };
        assert_eq!(verdict.diagnostic(), Some(&diagnostic));
        let back: ToolVerdict =
            serde_json::from_value(serde_json::to_value(&verdict).unwrap()).unwrap();
        assert_eq!(back, verdict);
        assert_eq!(back.diagnostic(), Some(&diagnostic));
        // The additive diagnostic is omitted from the wire when absent, so the
        // pre-diagnostic wire shape still rehydrates.
        let bare = ToolVerdict::recoverable_failure(FailureKind::Network);
        assert_eq!(bare.diagnostic(), None);
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            serde_json::json!({ "recoverable_failure": { "error_kind": "network" } }),
            "absent diagnostic must not appear on the wire"
        );
    }

    #[test]
    fn denial_carries_reason_kind_and_summary_and_roundtrips() {
        let deny = DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap();
        let denial = Denial::new(deny)
            .with_reason_kind(DenyReason::PolicyDenied)
            .with_summary(SafeSummary::new("blocked by policy").unwrap());
        assert_eq!(denial.deny, deny);
        assert_eq!(denial.reason_kind, Some(DenyReason::PolicyDenied));
        let resolution = Resolution::Denied(denial.clone());
        let back: Resolution =
            serde_json::from_value(serde_json::to_value(&resolution).unwrap()).unwrap();
        assert_eq!(back, resolution);
        // A bare denial (ref only) still serializes/rehydrates additively.
        let bare = Resolution::Denied(Denial::new(deny));
        assert_eq!(
            serde_json::to_value(&bare).unwrap(),
            serde_json::json!({ "denied": { "deny": "018f6a00-0000-7000-8000-000000000002" } })
        );
        assert_eq!(
            serde_json::from_value::<Resolution>(serde_json::to_value(&bare).unwrap()).unwrap(),
            bare
        );
    }

    #[test]
    fn recoverable_failure_carries_its_error_kind_across_the_wire() {
        // The recovery classification (retry-vs-terminal) survives round-trip —
        // the field the old mapping dropped as "G1".
        for kind in [
            FailureKind::Network,
            FailureKind::unknown("quota_exceeded").unwrap(),
        ] {
            let verdict = ToolVerdict::recoverable_failure(kind.clone());
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
                preview_meta: ResultPreviewMeta::default(),
                origin: None,
                output_digest: None,
            },
            verdict: ToolVerdict::recoverable_failure(FailureKind::InvalidInput),
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
                preview_meta: ResultPreviewMeta::default(),
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
            // Content preview carries delimiters/structure (not a caption).
            preview: Some(ModelResultPreview::new("{\"rows\": 3, \"ok\": true}").unwrap()),
            preview_meta: ResultPreviewMeta::default(),
            origin: None,
            output_digest: None,
        };
        let back: OutcomeRefs =
            serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
        assert_eq!(back, full);
        assert_eq!(
            back.preview.as_ref().map(ModelResultPreview::as_str),
            Some("{\"rows\": 3, \"ok\": true}")
        );

        // Absent: None omitted from the wire (skip_serializing_if), and a
        // legacy payload without the field rehydrates to None.
        let bare = OutcomeRefs {
            result,
            byte_len: 128,
            preview: None,
            preview_meta: ResultPreviewMeta::default(),
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
        ToolVerdict::recoverable_failure(FailureKind::InvalidInput)
    }

    fn outcome(verdict: ToolVerdict) -> Outcome {
        Outcome {
            refs: OutcomeRefs {
                result: ResultRef::parse("018f6a00-0000-7000-8000-000000000001").unwrap(),
                byte_len: 1,
                preview: None,
                preview_meta: ResultPreviewMeta::default(),
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
        // (today's CapabilityOutcome variant, the Resolution channel,
        //  is_suspension, parks). `parks` ⊇ `is_suspension`: it adds the three
        // re-entrant gate variants (Approval/Auth/Resource) the batch loop must
        // also stop on — see `parks_is_a_strict_superset_of_is_suspension`.
        let rows: [(&str, Resolution, bool, bool); 10] = [
            (
                "Completed",
                Resolution::Done(outcome(ToolVerdict::Success)),
                false,
                false,
            ),
            (
                "Failed",
                Resolution::Done(outcome(recoverable_failure())),
                false,
                false,
            ),
            (
                "Denied",
                Resolution::Denied(Denial::new(
                    DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap(),
                )),
                false,
                false,
            ),
            (
                "ApprovalRequired",
                Resolution::Blocked(Blocked::Approval(GateWaypoint::new(gate()))),
                false,
                true,
            ),
            (
                "AuthRequired",
                Resolution::Blocked(Blocked::Auth(GateWaypoint::new(gate()))),
                false,
                true,
            ),
            (
                "ResourceBlocked",
                Resolution::Blocked(Blocked::Resource(GateWaypoint::new(gate()))),
                false,
                true,
            ),
            (
                "SpawnedProcess",
                Resolution::Suspended(Suspension::Process(ProcessWaypoint::new(proc()))),
                true,
                true,
            ),
            // Non-suspending — the one that has bitten before (#6137, §5.3 table).
            (
                "SpawnedChildRun",
                Resolution::Done(outcome(child_spawned())),
                false,
                false,
            ),
            (
                "AwaitDependentRun",
                Resolution::Suspended(Suspension::DependentRun {
                    waypoint: GateWaypoint::new(gate()),
                    // A fully-populated staged result so the generic round-trip
                    // below proves byte_len, summary, observation, AND origin
                    // all survive the wire.
                    result: DependentRunResult::new(
                        256,
                        SafeSummary::new("child staged 4 rows").unwrap(),
                    )
                    .with_observation(SafeSummary::new("child preview: 4 rows").unwrap())
                    .with_origin(LoopRef::new("result:child-1").unwrap()),
                }),
                true,
                true,
            ),
            (
                "ExternalToolPending",
                Resolution::Suspended(Suspension::ExternalTool(GateWaypoint::new(gate()))),
                true,
                true,
            ),
        ];
        assert_eq!(rows.len(), 10, "all ten CapabilityOutcome variants covered");
        for (variant, resolution, suspends, parks) in rows {
            assert_eq!(
                resolution.is_suspension(),
                suspends,
                "{variant}: suspension semantics"
            );
            assert_eq!(resolution.parks(), parks, "{variant}: park semantics");
            // Round-trips through the wire like every channel.
            let json = serde_json::to_value(&resolution).unwrap();
            let back: Resolution = serde_json::from_value(json).unwrap();
            assert_eq!(back, resolution, "{variant}: round-trip");
        }
    }

    /// `parks()` is the loop-suspension predicate the §5.3 batch loops must gate
    /// on — a STRICT superset of `is_suspension()`. The inner match is
    /// exhaustive by construction: a new `Resolution` variant added later will
    /// fail to compile here until its park semantics are declared (§11.9).
    #[test]
    fn parks_is_a_strict_superset_of_is_suspension() {
        fn expected_parks(resolution: &Resolution) -> bool {
            match resolution {
                // Ran (incl. the non-suspending ChildSpawned) or terminally
                // denied: the batch continues past it.
                Resolution::Done(_) | Resolution::Denied(_) => false,
                // Re-entrant gates AND parked work: the batch stops here.
                Resolution::Blocked(_) | Resolution::Suspended(_) => true,
            }
        }

        let samples = [
            Resolution::Done(outcome(ToolVerdict::Success)),
            Resolution::Done(outcome(child_spawned())),
            Resolution::Denied(Denial::new(
                DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap(),
            )),
            Resolution::Blocked(Blocked::Approval(gate_wp())),
            Resolution::Blocked(Blocked::Auth(gate_wp())),
            Resolution::Blocked(Blocked::Resource(gate_wp())),
            Resolution::Suspended(Suspension::Process(proc_wp())),
            Resolution::Suspended(dep_run()),
            Resolution::Suspended(Suspension::ExternalTool(gate_wp())),
        ];
        for resolution in &samples {
            assert_eq!(
                resolution.parks(),
                expected_parks(resolution),
                "parks() disagrees for {}",
                resolution.kind()
            );
            // Superset direction: every suspension parks.
            if resolution.is_suspension() {
                assert!(
                    resolution.parks(),
                    "{}: is_suspension ⊆ parks",
                    resolution.kind()
                );
            }
        }

        // STRICT: a re-entrant approval gate parks but is NOT a suspension — the
        // exact variant a naive `is_suspension()` batch guard would mis-handle
        // as if the call had completed (the hazard §5.3 Stage 1 closes).
        let gate = Resolution::Blocked(Blocked::Approval(gate_wp()));
        assert!(gate.parks(), "an approval gate parks");
        assert!(
            !gate.is_suspension(),
            "an approval gate is not a suspension"
        );
    }

    #[test]
    fn resolution_batch_mirrors_capability_batch_outcome_and_roundtrips() {
        let batch = ResolutionBatch {
            resolutions: vec![
                Resolution::Done(outcome(ToolVerdict::Success)),
                Resolution::Blocked(Blocked::Approval(GateWaypoint::new(gate()))),
            ],
            stopped_on_suspension: true,
        };
        // Order and the stop flag survive a wire round-trip.
        let back: ResolutionBatch =
            serde_json::from_value(serde_json::to_value(&batch).unwrap()).unwrap();
        assert_eq!(back, batch);
        assert!(back.stopped_on_suspension);
        assert_eq!(back.resolutions.len(), 2);
        assert_eq!(back.resolutions[0].kind(), "done");
        // The batch stopped on a variant that parks (a gate) — exactly why the
        // flip's loop must gate on parks(), not the narrower is_suspension().
        assert!(back.resolutions[1].parks());
        assert!(!back.resolutions[1].is_suspension());
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
        let denied = Resolution::Denied(Denial::new(
            DenyRef::parse("018f6a00-0000-7000-8000-000000000002").unwrap(),
        ));
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
                Resolution::Denied(Denial::new(deny)),
                serde_json::json!({ "denied": { "deny": "018f6a00-0000-7000-8000-000000000002" } }),
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
            // DependentRun is a struct variant: the parked-on `waypoint` plus the
            // inline staged `result` (byte_len + redacted summary; observation and
            // origin omitted here, exercising the skip_serializing_if defaults).
            (
                Resolution::Suspended(Suspension::DependentRun {
                    waypoint: GateWaypoint::new(gate),
                    result: DependentRunResult::new(
                        256,
                        SafeSummary::new("child staged 4 rows").unwrap(),
                    ),
                }),
                serde_json::json!({
                    "suspended": {
                        "dependent_run": {
                            "waypoint": { "gate": "01890a5d-ac96-774b-bcce-b302099a8057" },
                            "result": { "byte_len": 256, "summary": "child staged 4 rows" }
                        }
                    }
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

    /// The inline staged result carries the child's byte_len, redacted summary,
    /// redacted observation preview, AND the preserved loop result origin across
    /// the wire — the fields that were previously unreachable on this channel
    /// (byte_len/summary folded into the host-only record, model_observation
    /// dropped). Reached through the [`Suspension::dependent_result`] accessor.
    #[test]
    fn dependent_run_carries_the_staged_result_inline() {
        let staged =
            DependentRunResult::new(2048, SafeSummary::new("child staged 7 rows").unwrap())
                .with_observation(SafeSummary::new("child preview: 7 rows").unwrap())
                .with_origin(LoopRef::new("result:child-9").unwrap());
        let suspension = Suspension::DependentRun {
            waypoint: gate_wp(),
            result: staged.clone(),
        };
        // The accessor answers only for DependentRun.
        assert_eq!(suspension.dependent_result(), Some(&staged));
        assert_eq!(Suspension::ExternalTool(gate_wp()).dependent_result(), None);
        assert_eq!(Suspension::Process(proc_wp()).dependent_result(), None);
        // The gate the suspension parks on stays reachable alongside the result.
        assert_eq!(suspension.gate_ref(), Some(&gate()));

        // Every staged field survives a wire round-trip.
        let back: Suspension =
            serde_json::from_value(serde_json::to_value(&suspension).unwrap()).unwrap();
        assert_eq!(back, suspension);
        let result = back.dependent_result().expect("staged result");
        assert_eq!(result.byte_len, 2048);
        assert_eq!(result.summary.as_str(), "child staged 7 rows");
        assert_eq!(
            result.observation.as_ref().map(SafeSummary::as_str),
            Some("child preview: 7 rows")
        );
        assert_eq!(
            result.origin.as_ref().map(LoopRef::as_str),
            Some("result:child-9")
        );
    }

    /// A hostile persisted staged result cannot rehydrate: the redacted
    /// [`SafeSummary`] fields reject sensitive-marker / path-shaped content on the
    /// wire, so the model-visible child text stays plain redacted vocabulary.
    #[test]
    fn dependent_run_result_rejects_an_unsafe_summary_on_the_wire() {
        // A leaked secret in the summary is rejected at the SafeSummary boundary.
        let json = serde_json::json!({
            "byte_len": 1,
            "summary": "api key: sk-ant-leak"
        });
        let err = serde_json::from_value::<DependentRunResult>(json)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("sensitive marker"),
            "rejection must be for the summary redaction contract: {err}"
        );

        // A path-shaped observation is likewise rejected on the wire.
        let json = serde_json::json!({
            "byte_len": 1,
            "summary": "ok",
            "observation": "leaked path /etc/passwd"
        });
        assert!(
            serde_json::from_value::<DependentRunResult>(json).is_err(),
            "a path-shaped observation must not rehydrate into the staged result"
        );
    }
}
