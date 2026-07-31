//! Producer-facing constructors that emit a host_api [`Resolution`] directly
//! (arch-simplification §3/§5.3 Stage 2b — the collapse complete).
//!
//! This module replaces the transitional `CapabilityOutcome` → `Resolution`
//! mapping artifact. Instead of building an intermediate ten-variant
//! `CapabilityOutcome` and mapping it, every capability producer calls the
//! constructor for the channel it means:
//!
//! - Non-gate channels return a bare [`Resolution`]: [`completed`], [`failed`],
//!   [`spawned_process`], [`spawned_child_run`].
//! - Gate/suspension channels return a [`GatedResolution`] (the [`Resolution`]
//!   plus the durable [`GateRecord`] its opaque ref renders from, §5.2.9), which
//!   the loop-host seam persists before returning the resolution:
//!   [`approval_required`], [`auth_required`], [`resource_blocked`],
//!   [`await_dependent_run`], [`external_tool_pending`].
//! - A terminal denial returns a [`DeniedResolution`] (the [`Resolution`] plus a
//!   sibling [`DenyRecord`]; a denial is terminal and same-turn, so the record
//!   is NOT persisted — the model-visible reason/summary ride the channel): [`denied`].
//!
//! These are free functions in `ironclaw_turns` (NOT methods on
//! `host_api::Resolution`): the non-lossy redaction below consumes loop-facing
//! vocabulary (`CapabilityFailureDetail`, `ModelVisibleToolObservation`,
//! `LoopGateRef`, …) that lives in this crate, and `host_api` sits below it.
//! `turns` is the lowest crate that sees both sides.
//!
//! ## Non-lossy carry (§5.3)
//!
//! `host_api::Resolution` carries **every recoverable field** the old
//! `CapabilityOutcome` variants held, via the vocabulary in
//! [`ironclaw_host_api::result_meta`]:
//!
//! - the failure recovery class ([`FailureKind`]) carried unchanged onto
//!   [`ToolVerdict::RecoverableFailure`], plus its structured `detail`
//!   ([`CapabilityFailureDetail`]) as a redacted [`ModelFailureDiagnostic`] on
//!   the same verdict (the model-visible correction hint): `InvalidInput` schema
//!   issues carry their [`DispatchInputIssueCode`](ironclaw_host_api::dispatch::DispatchInputIssueCode)
//!   plus redacted [`SafeSummary`] fields, and a free-text `Diagnostic` is
//!   carried as a bounded [`ModelDiagnostic`] after producer-side secret
//!   scrubbing (paths and payload delimiters remain available for recovery).
//! - `progress`/`terminate_hint`/`output_digest` →
//!   [`Outcome::progress`]/[`Outcome::terminate_hint`]/[`OutcomeRefs::output_digest`].
//! - the `resume_token` inside `approval_resume`/`auth_resume` → the
//!   [`ResumeToken`] on the gate [`GateWaypoint`]. Only the token crosses; the
//!   raw input/estimate stay host-side (the host reconstitutes them from storage
//!   keyed by the token).
//!
//! A spawned-process suspension carries only a [`ProcessRef`] (its summary has no
//! host channel). A completed result's `model_observation` rides the [`Outcome`]
//! result preview; a dependent-run child's rides the inline [`DependentRunResult`]
//! observation on the [`Suspension::DependentRun`] channel.
//!
//! ## Loop refs: minted kernel handle + preserved origin
//!
//! The loop's refs are opaque prefixed strings (`result:*`/`gate:*`/`process:*`);
//! host_api's kernel refs are opaque uuids by design, so they cannot carry the
//! loop's own ref identity. Each constructor mints a fresh kernel handle **and**
//! preserves the originating loop ref on the channel's `origin` (a [`LoopRef`]),
//! so loop/evidence state keyed under the loop ref stays reachable. The only
//! identity that crosses directly is [`TurnRunId`](crate::TurnRunId) → [`RunId`]
//! (both wrap a `Uuid`, preserved via `RunId::from_uuid`). Auth gate records are
//! keyed DETERMINISTICALLY from the `gate:auth-{gate_id}` ref via
//! [`GateRef::for_auth_gate`] so the persist seam and the runner's blocked-exit
//! read derive the same key (byte-stable resume).

use ironclaw_host_api::{
    decision::{DenyReason, RuntimeCredentialAuthRequirement},
    gate_record::{DenyRecord, GateRecord},
    ids::{DenyRef, GateRef, ProcessRef, ResultRef, RunId},
    model_result_preview::ModelResultPreview,
    resolution::{
        Blocked, Denial, DependentRunResult, GateWaypoint, Outcome, OutcomeRefs, ProcessWaypoint,
        Resolution, ResultPreviewMeta, Suspension, ToolVerdict,
    },
    result_meta::{
        FailureKind, LoopRef, ModelDiagnostic, ModelFailureDiagnostic, ModelInputIssue,
        ModelInputIssues, OutputDigest, ResultProgress, ResumeToken, TerminateHint,
    },
    safe_summary::SafeSummary,
};

use super::content_digest::ContentDigest;
use super::host::{
    CapabilityApprovalResume, CapabilityAuthResume, CapabilityDeniedReasonKind, CapabilityProgress,
    CapabilityResumeToken, LoopProcessRef,
};
use super::model_observation::{
    CapabilityFailureDetail, CapabilityInputIssue, ModelVisibleToolObservation,
    ToolObservationDetail,
};
use crate::{LoopGateRef, LoopResultRef, TurnRunId};

/// A [`Resolution`] on a gate/suspension channel paired with the durable
/// [`GateRecord`] its opaque ref renders from (§5.2.9).
///
/// `Resolution`'s control-plane arms carry only refs; the model-visible content
/// (pending-gate detail) lives in the referenced record. The loop-host seam
/// persists `gate_record` (keyed by the channel's [`GateRef`]) before returning
/// the resolution to the loop. A non-gate resolution routed through this wrapper
/// (`gate_record: None`) is a no-op for the persist seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatedResolution {
    pub resolution: Resolution,
    pub gate_record: Option<GateRecord>,
}

impl GatedResolution {
    /// A resolution with no durable gate record to persist (the `Done`,
    /// `Denied`, and `Suspended(Process)` channels).
    pub fn bare(resolution: Resolution) -> Self {
        Self {
            resolution,
            gate_record: None,
        }
    }

    /// A gate/suspension resolution paired with the record its ref renders from.
    fn gated(resolution: Resolution, gate_record: GateRecord) -> Self {
        Self {
            resolution,
            gate_record: Some(gate_record),
        }
    }
}

/// A terminal [`Resolution::Denied`] paired with the sibling [`DenyRecord`].
///
/// A denial is terminal and same-turn, so the record is NOT persisted; the
/// model-visible reason/summary ride the [`Denial`] channel itself (a projection
/// of the record). The record is retained so producers/tests can assert on the
/// redacted denial content at the seam without reading host storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeniedResolution {
    pub resolution: Resolution,
    pub deny_record: DenyRecord,
}

// --- Non-gate channel constructors (return a bare `Resolution`) -------------

/// The `Done` channel for a capability that ran and succeeded (verdict
/// `Success`). Loop-derived `progress`/`terminate_hint`/`output_digest` cross
/// onto the [`Outcome`]; a fresh [`ResultRef`] is minted and the loop result ref
/// is preserved on `OutcomeRefs.origin`.
pub fn completed(
    result_ref: LoopResultRef,
    safe_summary: String,
    progress: CapabilityProgress,
    terminate_hint: bool,
    byte_len: u64,
    output_digest: Option<ContentDigest>,
    model_observation: Option<ModelVisibleToolObservation>,
) -> Resolution {
    let (preview, preview_meta) = result_preview_parts(model_observation, &result_ref);
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            result: ResultRef::new(),
            byte_len,
            preview,
            preview_meta,
            origin: preserved_origin(result_ref.as_str()),
            output_digest: output_digest.map(output_digest_of),
        },
        verdict: ToolVerdict::Success,
        summary: safe_summary_or_placeholder(safe_summary),
        progress: result_progress_of(progress),
        terminate_hint: TerminateHint::from_bool(terminate_hint),
    })
}

/// The `Done` channel for a capability that ran and failed in a model-visible,
/// correctable way (verdict `RecoverableFailure`). The recovery classification
/// AND the redacted structured diagnostic ride the verdict, so the model-visible
/// correction hint crosses without the loop reading host storage.
pub fn failed(
    error_kind: FailureKind,
    safe_summary: String,
    detail: CapabilityFailureDetail,
) -> Resolution {
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            // A recoverable failure stages no durable output beyond its summary;
            // the ref is a minted handle the store may leave unpopulated, and
            // there is no originating loop result ref to preserve.
            result: ResultRef::new(),
            byte_len: 0,
            preview: None,
            preview_meta: ResultPreviewMeta::default(),
            origin: None,
            output_digest: None,
        },
        verdict: ToolVerdict::recoverable_failure_with_diagnostic(
            error_kind,
            model_failure_diagnostic(detail),
        ),
        summary: safe_summary_or_placeholder(safe_summary),
        progress: ResultProgress::default(),
        terminate_hint: TerminateHint::default(),
    })
}

/// The `Suspended(Process)` channel: parked work on a spawned OS process the turn
/// now waits on. A process suspension tracks only a [`ProcessRef`]; the loop
/// process ref is preserved on the waypoint origin (the process's safe summary
/// has no host channel).
pub fn spawned_process(process_ref: LoopProcessRef) -> Resolution {
    let waypoint = process_waypoint(ProcessRef::new(), &process_ref);
    Resolution::Suspended(Suspension::Process(waypoint))
}

/// The `Done`/`ChildSpawned` channel: a NON-suspending child run whose result the
/// executor appends before continuing (the #6137 bug class). Carries the child's
/// [`RunId`] on the verdict (identity preserved via `RunId::from_uuid`); the loop
/// result ref is replaced by a fresh [`ResultRef`] and preserved on the origin.
pub fn spawned_child_run(
    child_run_id: TurnRunId,
    result_ref: LoopResultRef,
    safe_summary: String,
    byte_len: u64,
    model_observation: Option<ModelVisibleToolObservation>,
) -> Resolution {
    let (preview, preview_meta) = result_preview_parts(model_observation, &result_ref);
    Resolution::Done(Outcome {
        refs: OutcomeRefs {
            result: ResultRef::new(),
            byte_len,
            preview,
            preview_meta,
            origin: preserved_origin(result_ref.as_str()),
            output_digest: None,
        },
        verdict: ToolVerdict::ChildSpawned {
            child_run: RunId::from_uuid(child_run_id.as_uuid()),
        },
        summary: safe_summary_or_placeholder(safe_summary),
        progress: ResultProgress::default(),
        terminate_hint: TerminateHint::default(),
    })
}

// --- Gate/suspension channel constructors (return a `GatedResolution`) -------

/// The `Blocked(Approval)` re-entrant gate: needs human approval before it may
/// run. The gate-render summary rides the [`GateRecord`]; the resume token and
/// preserved loop gate ref ride the waypoint (never the model-visible record).
pub fn approval_required(
    gate_ref: LoopGateRef,
    safe_summary: String,
    approval_resume: Option<CapabilityApprovalResume>,
) -> GatedResolution {
    let waypoint = gate_waypoint(
        GateRef::new(),
        &gate_ref,
        approval_resume_token(approval_resume),
    );
    GatedResolution::gated(
        Resolution::Blocked(Blocked::Approval(waypoint)),
        GateRecord::Approval {
            summary: safe_summary_or_placeholder(safe_summary),
        },
    )
}

/// The `Blocked(Auth)` re-entrant gate: needs a credential the caller has not
/// supplied. The host-owned `credential_requirements` ride the record (never the
/// model-visible channel); the resume token and preserved loop gate ref ride the
/// waypoint. The record is keyed DETERMINISTICALLY from the `gate:auth-{gate_id}`
/// ref (see [`GateRef::for_auth_gate`]) so the persist seam and the runner's
/// blocked-exit read derive the same key.
pub fn auth_required(
    gate_ref: LoopGateRef,
    credential_requirements: Vec<RuntimeCredentialAuthRequirement>,
    safe_summary: String,
    auth_resume: Option<CapabilityAuthResume>,
) -> GatedResolution {
    let minted = auth_gate_record_ref(&gate_ref);
    let waypoint = gate_waypoint(minted, &gate_ref, auth_resume_token(auth_resume));
    GatedResolution::gated(
        Resolution::Blocked(Blocked::Auth(waypoint)),
        GateRecord::Auth {
            summary: safe_summary_or_placeholder(safe_summary),
            credential_requirements,
        },
    )
}

/// The `Blocked(Resource)` re-entrant gate: needs resource budget currently
/// unavailable. No resume token — a resource gate resumes against then-current
/// budget (§5.3.3).
pub fn resource_blocked(gate_ref: LoopGateRef, safe_summary: String) -> GatedResolution {
    let waypoint = gate_waypoint(GateRef::new(), &gate_ref, None);
    GatedResolution::gated(
        Resolution::Blocked(Blocked::Resource(waypoint)),
        GateRecord::Resource {
            summary: safe_summary_or_placeholder(safe_summary),
        },
    )
}

/// The `Suspended(DependentRun)` channel: parked work awaiting a dependent child
/// run. The durable [`GateRecord`] holds the staged result handle + byte length;
/// the channel ALSO carries the staged result inline ([`DependentRunResult`]) so
/// the loop observes the child's output on resume without reading host storage.
/// `model_observation` rides the inline observation caption.
pub fn await_dependent_run(
    gate_ref: LoopGateRef,
    result_ref: LoopResultRef,
    safe_summary: String,
    byte_len: u64,
    model_observation: Option<ModelVisibleToolObservation>,
) -> GatedResolution {
    let waypoint = gate_waypoint(GateRef::new(), &gate_ref, None);
    let minted_result = ResultRef::new();
    let mut staged =
        DependentRunResult::new(byte_len, safe_summary_or_placeholder(safe_summary.clone()));
    // The dependent-child observation channel is a bounded [`SafeSummary`]
    // caption: a child suspension carries the summary caption, not the inline
    // first-look content (that is the completed-`Outcome` preview).
    if let Some(observation) = observation_summary_caption(model_observation) {
        staged = staged.with_observation(observation);
    }
    if let Some(origin) = preserved_origin(result_ref.as_str()) {
        staged = staged.with_origin(origin);
    }
    GatedResolution::gated(
        Resolution::Suspended(Suspension::DependentRun {
            waypoint,
            result: staged,
        }),
        GateRecord::DependentRun {
            summary: safe_summary_or_placeholder(safe_summary),
            result: minted_result,
            byte_len,
            result_origin: preserved_origin(result_ref.as_str()),
        },
    )
}

/// The `Suspended(ExternalTool)` channel: parked work on a client-executed
/// external tool the host does not run.
pub fn external_tool_pending(gate_ref: LoopGateRef, safe_summary: String) -> GatedResolution {
    let waypoint = gate_waypoint(GateRef::new(), &gate_ref, None);
    GatedResolution::gated(
        Resolution::Suspended(Suspension::ExternalTool(waypoint)),
        GateRecord::ExternalTool {
            summary: safe_summary_or_placeholder(safe_summary),
        },
    )
}

// --- Terminal denial constructor (returns a `DeniedResolution`) --------------

/// The terminal `Denied` channel — model-visible, not re-entrant. The
/// model-visible reason + redacted summary ride the [`Denial`] channel (a
/// projection of the sibling [`DenyRecord`]), so the loop can render the denial
/// without reading host storage.
pub fn denied(reason_kind: CapabilityDeniedReasonKind, safe_summary: String) -> DeniedResolution {
    let reason = deny_reason_from_kind(&reason_kind);
    let summary = safe_summary_or_placeholder(safe_summary);
    DeniedResolution {
        resolution: Resolution::Denied(
            Denial::new(DenyRef::new())
                .with_reason_kind(reason)
                .with_summary(summary.clone()),
        ),
        deny_record: DenyRecord { reason, summary },
    }
}

// --- Private redaction/mapping helpers (moved verbatim from the mapping) -----

/// Redact a loop-facing [`CapabilityFailureDetail`] into the host_api
/// [`ModelFailureDiagnostic`] carried on the verdict.
///
/// The loop's `InvalidInput` schema issues cross with their structured
/// [`DispatchInputIssueCode`](ironclaw_host_api::dispatch::DispatchInputIssueCode) and
/// every free-text field re-validated through the [`SafeSummary`] redaction
/// contract (a field that fails is dropped; an issue whose required `path` fails
/// is dropped whole). The loop's producer-scrubbed free-text `Diagnostic` keeps
/// paths and payload delimiters in a bounded [`ModelDiagnostic`].
fn model_failure_diagnostic(detail: CapabilityFailureDetail) -> ModelFailureDiagnostic {
    match detail {
        CapabilityFailureDetail::InvalidInput { issues } => {
            let issues =
                ModelInputIssues::truncating(issues.into_iter().filter_map(model_input_issue));
            ModelFailureDiagnostic::InvalidInput { issues }
        }
        CapabilityFailureDetail::Diagnostic { text } => {
            let text = match ModelDiagnostic::truncating(text) {
                Ok(text) => text,
                // silent-ok: the model-visible diagnostic boundary fails closed
                // to a fixed sentence rather than failing the turn — the
                // producer is responsible for scrubbing before text reaches
                // here. Reaching this arm means it did not, which an operator
                // wants to see; `debug!` because `info!`/`warn!` corrupt the
                // REPL/TUI (repo CLAUDE.md).
                Err(error) => {
                    tracing::debug!(
                        %error,
                        "model-visible diagnostic rejected at the sanitized \
                         resolution boundary; substituting the fixed fallback"
                    );
                    ModelDiagnostic::unavailable()
                }
            };
            ModelFailureDiagnostic::Diagnostic { text }
        }
        // The TRUSTED channel: the payload is already a validated
        // `HostRemediation`, so it crosses the charter as-is. Deliberately NOT
        // squeezed through `SafeSummary` — doing so is the #6299 regression
        // this arm exists to prevent, because every host-authored remediation
        // names a `config set` key or a console URL and would collapse to the
        // placeholder. The guard that makes this safe is upstream (only host
        // code constructs `HostRemediation`, and its value guard rejects
        // credential shapes), not a second redaction pass here.
        CapabilityFailureDetail::HostRemediation { text } => {
            ModelFailureDiagnostic::HostRemediation { text }
        }
    }
}

/// Redact one loop-facing [`CapabilityInputIssue`] into a host_api
/// [`ModelInputIssue`], routing every free-text field through [`SafeSummary`].
/// Returns `None` when the required `path` fails the redaction contract (a
/// path-shaped or secret-shaped path — which a safe producer never emits — is
/// dropped rather than carried raw); optional fields that fail are individually
/// dropped. `.ok()` here converts a pure text-to-safe-text validation failure
/// into an absent field, never a swallowed I/O error.
fn model_input_issue(issue: CapabilityInputIssue) -> Option<ModelInputIssue> {
    let CapabilityInputIssue {
        path,
        code,
        expected,
        received,
        schema_path,
    } = issue;
    let mut model = ModelInputIssue::new(SafeSummary::new(path).ok()?, code);
    if let Some(expected) = expected.and_then(|value| SafeSummary::new(value).ok()) {
        model = model.with_expected(expected);
    }
    if let Some(received) = received.and_then(|value| SafeSummary::new(value).ok()) {
        model = model.with_received(received);
    }
    if let Some(schema_path) = schema_path.and_then(|value| SafeSummary::new(value).ok()) {
        model = model.with_schema_path(schema_path);
    }
    Some(model)
}

/// The canonical host [`GateRef`] key for an auth gate's [`GateRecord`], derived
/// deterministically (name-based v5) from the auth gate id encoded in the loop
/// `gate:auth-{gate_id}` ref. Mirrors [`GateRef::for_approval_request`] so the
/// loop-host persist seam and the runner's blocked-exit render-from-record read
/// agree on the key (§5.2.9 / §5.3 Stage 2). A loop ref that is not a
/// `gate:auth-{gate_id}` (which the normal producer never emits) falls back to a
/// fresh handle — the record is still persisted, only not re-derivable.
fn auth_gate_record_ref(loop_gate: &LoopGateRef) -> GateRef {
    loop_gate
        .as_str()
        .strip_prefix("gate:auth-")
        .map(GateRef::for_auth_gate)
        // silent-ok: pure string reconstruction; a loop ref without the
        // `gate:auth-` prefix is never emitted for an auth gate.
        .unwrap_or_default()
}

/// A gate waypoint: the minted kernel handle plus the preserved originating loop
/// gate ref and (for approval/auth) the opaque resume token the loop echoes back.
fn gate_waypoint(
    minted: GateRef,
    loop_gate: &LoopGateRef,
    resume: Option<ResumeToken>,
) -> GateWaypoint {
    let mut waypoint = GateWaypoint::new(minted);
    if let Some(origin) = preserved_origin(loop_gate.as_str()) {
        waypoint = waypoint.with_origin(origin);
    }
    if let Some(resume) = resume {
        waypoint = waypoint.with_resume(resume);
    }
    waypoint
}

/// A process waypoint: the minted kernel handle plus the preserved originating
/// loop process ref.
fn process_waypoint(minted: ProcessRef, loop_process: &LoopProcessRef) -> ProcessWaypoint {
    match preserved_origin(loop_process.as_str()) {
        Some(origin) => ProcessWaypoint::new(minted).with_origin(origin),
        None => ProcessWaypoint::new(minted),
    }
}

/// Preserve a loop ref as a redacted host_api [`LoopRef`] when it satisfies the
/// host redaction contract (bounded, control-free, no path delimiters). A loop
/// ref that fails — which a safe production ref never does — falls back to `None`;
/// `.ok()` here converts a pure text-to-safe-text validation failure into an
/// absent origin, never a swallowed I/O error.
fn preserved_origin(loop_ref: &str) -> Option<LoopRef> {
    LoopRef::new(loop_ref).ok()
}

/// The opaque approval resume token, when the producer carried one.
fn approval_resume_token(resume: Option<CapabilityApprovalResume>) -> Option<ResumeToken> {
    resume.and_then(|resume| resume_token_of(&resume.resume_token))
}

/// The opaque auth resume token, when the producer carried one.
fn auth_resume_token(resume: Option<CapabilityAuthResume>) -> Option<ResumeToken> {
    resume.and_then(|resume| resume.resume_token.as_ref().and_then(resume_token_of))
}

/// Convert a loop-facing [`CapabilityResumeToken`] to a host_api [`ResumeToken`].
/// Both are bounded/control-free, so a valid loop token always crosses; `.ok()`
/// drops a token that fails the host bound rather than panic (the mapping is
/// total).
fn resume_token_of(token: &CapabilityResumeToken) -> Option<ResumeToken> {
    ResumeToken::new(token.as_str()).ok()
}

/// Map the loop's [`ContentDigest`] onto host_api's [`OutputDigest`]; both wrap
/// the same truncated Blake3 `u64`.
fn output_digest_of(digest: ContentDigest) -> OutputDigest {
    OutputDigest::new(digest.0)
}

/// Map the loop's [`CapabilityProgress`] onto host_api's [`ResultProgress`]; the
/// variants correspond one-to-one.
fn result_progress_of(progress: CapabilityProgress) -> ResultProgress {
    match progress {
        CapabilityProgress::Unknown => ResultProgress::Unknown,
        CapabilityProgress::MadeProgress => ResultProgress::MadeProgress,
        CapabilityProgress::NoChange => ResultProgress::NoChange,
        CapabilityProgress::Blocked => ResultProgress::Blocked,
    }
}

/// The #5838 first-look inline CONTENT preview and its continuation metadata from
/// a loop tool observation, when present.
///
/// The inline content the model reads without a follow-up `result_read` lives on
/// the `ResultReference` detail's `preview` — NOT the generic `summary` caption
/// (routing content through `SafeSummary` dropped every delimiter-bearing/JSON
/// result and scrubbed `Secretary`, forcing a re-read amnesia loop). It is carried
/// as a [`ModelResultPreview`]: delimiters/newlines retained, credential-redacted
/// at a word boundary, up to 24 KiB. The paired [`ResultPreviewMeta`] carries the
/// TRUNCATED-preview continuation info (`result_read` / large results): the
/// referenced result ref, full byte size, next offset, and JSON-array element
/// count, so the model reads the full result. This metadata is preserved even
/// when the preview text is rejected: continuation authority belongs to the
/// durable source result, not to the ephemeral invocation presenting it. Detail
/// kinds other than `ResultReference` have no inline content.
///
/// `own_result_ref` is this outcome's own loop result ref: the referenced ref is
/// carried only when it DIFFERS (a `result_read` presenting another result's ref);
/// otherwise the reconstruction uses the outcome's own ref, keeping the wire clean.
fn result_preview_parts(
    observation: Option<ModelVisibleToolObservation>,
    own_result_ref: &LoopResultRef,
) -> (Option<ModelResultPreview>, ResultPreviewMeta) {
    let empty = (None, ResultPreviewMeta::default());
    let Some(observation) = observation else {
        return empty;
    };
    // Capture the observation's own model-visible summary before destructuring
    // `detail`; it is DISTINCT from the outcome caption and must survive the
    // collapse so the reconstructed observation keeps the producer's exact
    // truncation/continuation hint (best-effort caption via `.ok()`).
    let ModelVisibleToolObservation {
        summary, detail, ..
    } = observation;
    let summary = SafeSummary::new(summary).ok();
    let ToolObservationDetail::ResultReference {
        result_ref,
        preview,
        total_bytes,
        next_offset,
        item_count,
        ..
    } = detail
    else {
        return empty;
    };
    // Credential material is masked instead of discarding the whole preview.
    // The result ref and paging metadata are independent host-authored authority:
    // keep them even when no preview was supplied or the remaining content is
    // rejected, so replay can continue against the durable source.
    let preview = preview.and_then(|text| ModelResultPreview::redacted(text).ok());
    let referenced_result_ref = if result_ref == own_result_ref.as_str() {
        None
    } else {
        LoopRef::new(result_ref).ok()
    };
    (
        preview,
        ResultPreviewMeta {
            referenced_result_ref,
            total_bytes,
            next_offset,
            item_count,
            summary,
        },
    )
}

/// The observation's generic `summary` as a bounded [`SafeSummary`] caption — the
/// dependent-child observation channel, which carries a caption rather than the
/// inline first-look content the completed-`Outcome` preview does.
///
/// `.ok()` degrades a caption that fails the caption redaction contract to `None`;
/// a pure text-to-safe-text conversion, and the caption is best-effort.
fn observation_summary_caption(
    observation: Option<ModelVisibleToolObservation>,
) -> Option<SafeSummary> {
    observation.and_then(|observation| SafeSummary::new(observation.summary).ok())
}

/// Convert a loop-facing `safe_summary: String` to a host_api [`SafeSummary`].
///
/// The redaction rule is the same on both sides (#6236), so a value the producer
/// already redacted normally passes. If it somehow fails validation, fall back to
/// the infallible [`SafeSummary::placeholder`] rather than panic — this mapping is
/// total.
fn safe_summary_or_placeholder(raw: String) -> SafeSummary {
    SafeSummary::new(raw).unwrap_or_else(|_| SafeSummary::placeholder())
}

/// Map the loop-side denial vocabulary onto host_api's [`DenyReason`].
///
/// The loop's [`CapabilityDeniedReasonKind`] is an evolving open set
/// (`EmptySurface` plus free-form `Unknown(..)` strings like `hook_denied`,
/// `model_view_denied`); host_api's [`DenyReason`] is a fixed closed enum whose
/// variants originate on the host authorize path, not the loop. There is no
/// faithful 1:1, so this is a best-effort match: a reason string that already
/// Every denial reason string a producer in this workspace can mint.
///
/// Verified by enumerating the mint sites — `CapabilityDeniedReasonKind::unknown`,
/// `capability_denied_reason_kind`, the named `EmptySurface` variant, and
/// `denied_reason_kind_for` (guarded to `Authorization | PolicyDenied`, so it
/// yields exactly `auth_denied` and `policy_denied`). `PRODUCTION_DENY_REASONS`
/// pins that list; [`classify_deny_reason`] must have an answer for each, so a
/// newly minted reason cannot silently inherit the conservative fallback.
///
/// Deliberately excluded: `expected_approval` and `host_policy_denied`, which
/// [`classify_deny_reason`] handles but which appear only in contract-test
/// fixtures. Listing them here would claim producer coverage this crate does
/// not have.
#[cfg(test)]
const PRODUCTION_DENY_REASONS: [&str; 12] = [
    "auth_denied",
    "policy_denied",
    "empty_surface",
    "hook_denied",
    "hook_gate_ref_unavailable",
    "invalid_spawn_input",
    "missing_provider_trust",
    "model_view_denied",
    "network_denied",
    "outbound_delivery_approval_required",
    "outside_visible_surface",
    "surface_profile_denied",
];

/// Classify a denial's producer-side reason, or `None` if nobody has classified
/// it yet.
///
/// Split out from [`deny_reason_from_kind`] so the "unclassified" case stays
/// *visible*. Folded into the function, an unclassified reason is
/// indistinguishable from a deliberate `PolicyDenied`, which is precisely how
/// the original collapse hid: every reason looked like a decision.
fn classify_deny_reason(tag: &str) -> Option<DenyReason> {
    Some(match tag {
        // Named a capability that is not in this caller's view: nothing to
        // unlock, and retrying the same call cannot succeed.
        //
        // `empty_surface` deliberately stays `PolicyDenied` below rather than
        // joining these. An empty surface is normally a profile/policy outcome
        // ("this loop is configured without tools"), and four sites pin that
        // reading — `deny_record_reason_maps_best_effort_with_policy_fallback`
        // names it outright. Reclassifying it is arguable but unproven, so the
        // pinned decision stands.
        "model_view_denied" | "outside_visible_surface" => DenyReason::UnknownCapability,
        // A credential is missing or refused: an auth flow is the unlock.
        "auth_denied" => DenyReason::UnknownSecret,
        // A human approval is pending or was refused — re-asking can succeed.
        // `expected_approval` is a contract-test tag, not a producer tag.
        "outbound_delivery_approval_required" | "expected_approval" => DenyReason::ApprovalDenied,
        // Egress refused by network policy.
        "network_denied" => DenyReason::NetworkDenied,
        // Host-side invariant broke; not the caller's doing and not unlockable
        // by them.
        "hook_gate_ref_unavailable" => DenyReason::InternalInvariantViolation,
        // Model-supplied input the model itself can correct.
        "invalid_spawn_input" => DenyReason::MissingGrant,
        // The provider has no trust decision on record — a grant is what is
        // missing, not a permission the caller already lacks.
        "missing_provider_trust" => DenyReason::MissingGrant,
        // Genuine policy refusals: nothing the caller does unlocks these.
        // `host_policy_denied` is a contract-test tag, not a producer tag.
        "policy_denied"
        | "host_policy_denied"
        | "hook_denied"
        | "surface_profile_denied"
        | "empty_surface" => DenyReason::PolicyDenied,
        _ => return None,
    })
}

/// Map a denial's producer-side reason onto the model-visible [`DenyReason`].
///
/// This was a serde parse over `DenyReason`'s snake_case tags, which silently
/// fell back to `PolicyDenied`. Of the twelve reason strings production
/// actually mints, only `network_denied` and `policy_denied` coincide with a
/// tag; the other ten fell through, so nearly every denial reached the model as
/// a flat "policy denied" — *no capabilities are available to you*, *a hook
/// blocked this*, *you need to authenticate*, and *an approval is waiting for a
/// human* were all indistinguishable.
///
/// The distinction the model needs is **what would unlock the call** (#6284
/// item 4): a permanent block, a missing credential, and a pending approval
/// call for three different next actions, and collapsing them leaves the model
/// guessing.
///
/// An unclassified reason still resolves to `PolicyDenied` — the honest
/// conservative answer — but that path is now reachable only by a reason nobody
/// has classified, and `every_production_deny_reason_is_classified` fails if a
/// producer adds one without choosing.
fn deny_reason_from_kind(kind: &CapabilityDeniedReasonKind) -> DenyReason {
    classify_deny_reason(kind.as_str()).unwrap_or(DenyReason::PolicyDenied)
}

#[cfg(test)]
mod deny_reason_tests {
    use super::*;

    fn reason(tag: &str) -> CapabilityDeniedReasonKind {
        CapabilityDeniedReasonKind::unknown(tag).expect("valid reason tag")
    }

    /// Denials must tell the model *what would unlock the call*.
    ///
    /// The previous serde parse matched `DenyReason`'s own tags, and ten of the
    /// twelve reasons production mints are not among them — so a missing
    /// credential, a pending human approval, an absent grant and a genuine
    /// policy block all arrived as `PolicyDenied`. Each of those wants a
    /// different next action from the model, and it had no way to tell them
    /// apart. This pins that they stay distinguishable.
    #[test]
    fn denials_distinguish_what_would_unlock_the_call() {
        // Not your capability — nothing to unlock, but not a policy refusal.
        for tag in ["model_view_denied", "outside_visible_surface"] {
            assert_eq!(
                deny_reason_from_kind(&reason(tag)),
                DenyReason::UnknownCapability,
                "{tag} should read as an absent capability"
            );
        }
        // A credential unlocks this one.
        assert_eq!(
            deny_reason_from_kind(&reason("auth_denied")),
            DenyReason::UnknownSecret,
            "auth_denied should point at an auth flow"
        );
        // A human unlocks these.
        for tag in ["outbound_delivery_approval_required", "expected_approval"] {
            assert_eq!(
                deny_reason_from_kind(&reason(tag)),
                DenyReason::ApprovalDenied,
                "{tag} should read as approvable, not forbidden"
            );
        }
        // Nothing the caller does unlocks these. `empty_surface` is here
        // deliberately: existing tests pin it as a policy outcome, and
        // reclassifying it is arguable but unproven.
        for tag in [
            "host_policy_denied",
            "hook_denied",
            "surface_profile_denied",
            "empty_surface",
        ] {
            assert_eq!(
                deny_reason_from_kind(&reason(tag)),
                DenyReason::PolicyDenied,
                "{tag} is a genuine policy refusal"
            );
        }
        assert_eq!(
            deny_reason_from_kind(&reason("network_denied")),
            DenyReason::NetworkDenied
        );
        assert_eq!(
            deny_reason_from_kind(&reason("hook_gate_ref_unavailable")),
            DenyReason::InternalInvariantViolation
        );
        // A grant is what is missing here, not a permission already refused.
        for tag in ["invalid_spawn_input", "missing_provider_trust"] {
            assert_eq!(
                deny_reason_from_kind(&reason(tag)),
                DenyReason::MissingGrant,
                "{tag} should read as a missing grant"
            );
        }
    }

    /// Every reason a producer in this workspace can mint must be classified
    /// deliberately. Without this, a new producer tag silently inherits the
    /// conservative `PolicyDenied` fallback and is indistinguishable from a
    /// real policy refusal — which is exactly how the original collapse hid.
    ///
    /// If this fails after adding a denial site, classify the new tag in
    /// `classify_deny_reason` and add it to `PRODUCTION_DENY_REASONS`.
    #[test]
    fn every_production_deny_reason_is_classified() {
        for tag in PRODUCTION_DENY_REASONS {
            assert!(
                classify_deny_reason(tag).is_some(),
                "{tag} is minted by a producer but nobody classified it; it \
                 would reach the model as an undifferentiated policy refusal"
            );
        }
    }

    /// Drive the real `denied()` constructor, not the mapper in isolation.
    ///
    /// `denied()` writes the reason to two places the model and the audit trail
    /// read separately — `Resolution::Denied`'s `reason_kind` and the
    /// `DenyRecord` — and a mapper-only test cannot catch either channel losing
    /// the distinction or the two disagreeing. (`.claude/rules/testing.md`:
    /// test through the caller.)
    #[test]
    fn denied_surfaces_the_distinction_on_both_channels() {
        let matrix = [
            ("model_view_denied", DenyReason::UnknownCapability),
            ("auth_denied", DenyReason::UnknownSecret),
            (
                "outbound_delivery_approval_required",
                DenyReason::ApprovalDenied,
            ),
            ("network_denied", DenyReason::NetworkDenied),
            (
                "hook_gate_ref_unavailable",
                DenyReason::InternalInvariantViolation,
            ),
            ("missing_provider_trust", DenyReason::MissingGrant),
            ("hook_denied", DenyReason::PolicyDenied),
            ("some_future_reason", DenyReason::PolicyDenied),
        ];
        for (tag, expected) in matrix {
            let result = denied(reason(tag), format!("denied: {tag}"));
            match &result.resolution {
                Resolution::Denied(denial) => assert_eq!(
                    denial.reason_kind,
                    Some(expected),
                    "{tag}: the model-facing channel lost the distinction"
                ),
                other => panic!("{tag}: expected Denied, got {other:?}"),
            }
            assert_eq!(
                result.deny_record.reason, expected,
                "{tag}: the audit record lost the distinction"
            );
        }
    }

    /// The conservative fallback must remain reachable only by a reason nobody
    /// has classified — not by the whole product, as before.
    #[test]
    fn an_unclassified_reason_still_falls_back_conservatively() {
        assert_eq!(
            deny_reason_from_kind(&reason("some_future_reason")),
            DenyReason::PolicyDenied
        );
    }

    /// Guard against the collapse returning: if every distinct production
    /// reason maps to one value again, this fails.
    #[test]
    fn production_denial_reasons_do_not_collapse_to_one_value() {
        let distinct: std::collections::HashSet<DenyReason> = PRODUCTION_DENY_REASONS
            .iter()
            .map(|tag| deny_reason_from_kind(&reason(tag)))
            .collect();
        assert!(
            distinct.len() >= 6,
            "denial reasons collapsed to {} distinct values; the model cannot \
             tell an auth prompt from a permanent block",
            distinct.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::CapabilityInputRef;
    use super::super::model_observation::ModelVisibleToolObservation;
    use super::super::{CapabilityProgress, MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION};
    use super::*;
    use ironclaw_host_api::{
        capability::RuntimeCredentialAccountSetup,
        dispatch::DispatchInputIssueCode,
        gate_record::GateRecord,
        ids::{ApprovalRequestId, CorrelationId, ExtensionId, VendorId},
    };

    fn result_ref() -> LoopResultRef {
        LoopResultRef::new("result:child-1").unwrap()
    }

    fn gate_ref() -> LoopGateRef {
        LoopGateRef::new("gate:pending-1").unwrap()
    }

    fn auth_gate_ref() -> LoopGateRef {
        LoopGateRef::new("gate:auth-cred-1").unwrap()
    }

    fn credential_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: VendorId::new("github").unwrap(),
            setup: RuntimeCredentialAccountSetup::ManualToken,
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: vec!["repo".to_string()],
        }
    }

    fn completed_ok(summary: &str) -> Resolution {
        completed(
            result_ref(),
            summary.to_string(),
            CapabilityProgress::MadeProgress,
            true,
            4096,
            None,
            None,
        )
    }

    /// A model-visible `ResultReference` tool observation whose inline
    /// `detail.preview` content is `content` (the model-visible CONTENT is on the
    /// detail preview, per #5838; the `summary` is a generic caption).
    fn observation(content: &str) -> ModelVisibleToolObservation {
        use super::super::model_observation::{
            ObservationTrust, ToolObservationDetail, ToolObservationStatus,
        };
        ModelVisibleToolObservation {
            schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
            status: ToolObservationStatus::Success,
            summary: "tool completed".to_string(),
            detail: ToolObservationDetail::ResultReference {
                result_ref: "result:staged".to_string(),
                byte_len: 10,
                preview: Some(content.to_string()),
                total_bytes: None,
                next_offset: None,
                item_count: None,
            },
            artifacts: vec![],
            recovery: None,
            trust: ObservationTrust::UntrustedToolOutput,
        }
    }

    /// The §5.3 acceptance table is the definition of done: every producer
    /// constructor lands on exactly one `Resolution` channel with the correct
    /// suspension semantics and side record. This mirrors host_api's
    /// `resolution_covers_the_full_acceptance_table` on the producer side.
    #[test]
    fn constructors_cover_the_full_acceptance_table() {
        // (label, resolution, is_suspension, expected gate_record kind, deny present)
        struct Row {
            label: &'static str,
            resolution: Resolution,
            suspends: bool,
            gate_record: Option<&'static str>,
            deny_record: bool,
        }

        let approval = approval_required(gate_ref(), "awaiting approval".to_string(), None);
        let auth = auth_required(
            auth_gate_ref(),
            vec![credential_requirement()],
            "awaiting credential".to_string(),
            None,
        );
        let resource = resource_blocked(gate_ref(), "awaiting budget".to_string());
        let dependent = await_dependent_run(
            gate_ref(),
            result_ref(),
            "awaiting dependent run".to_string(),
            256,
            None,
        );
        let external = external_tool_pending(gate_ref(), "awaiting external tool".to_string());
        let deny = denied(
            CapabilityDeniedReasonKind::EmptySurface,
            "denied by policy".to_string(),
        );

        let rows = vec![
            Row {
                label: "completed",
                resolution: completed_ok("read 3 files"),
                suspends: false,
                gate_record: None,
                deny_record: false,
            },
            Row {
                label: "failed",
                resolution: failed(
                    FailureKind::InputEncode,
                    "tool input rejected".to_string(),
                    CapabilityFailureDetail::Diagnostic {
                        text: "tool input rejected".to_string(),
                    },
                ),
                suspends: false,
                gate_record: None,
                deny_record: false,
            },
            Row {
                label: "denied",
                resolution: deny.resolution.clone(),
                suspends: false,
                gate_record: None,
                deny_record: true,
            },
            Row {
                label: "approval_required",
                resolution: approval.resolution.clone(),
                suspends: false,
                gate_record: approval.gate_record.as_ref().map(GateRecord::kind),
                deny_record: false,
            },
            Row {
                label: "auth_required",
                resolution: auth.resolution.clone(),
                suspends: false,
                gate_record: auth.gate_record.as_ref().map(GateRecord::kind),
                deny_record: false,
            },
            Row {
                label: "resource_blocked",
                resolution: resource.resolution.clone(),
                suspends: false,
                gate_record: resource.gate_record.as_ref().map(GateRecord::kind),
                deny_record: false,
            },
            Row {
                label: "spawned_process",
                resolution: spawned_process(LoopProcessRef::new("process:pid-1").unwrap()),
                suspends: true,
                gate_record: None,
                deny_record: false,
            },
            Row {
                label: "spawned_child_run",
                resolution: spawned_child_run(
                    TurnRunId::new(),
                    result_ref(),
                    "spawned child run".to_string(),
                    128,
                    None,
                ),
                // NON-suspending — the #6137 bug class.
                suspends: false,
                gate_record: None,
                deny_record: false,
            },
            Row {
                label: "await_dependent_run",
                resolution: dependent.resolution.clone(),
                suspends: true,
                gate_record: dependent.gate_record.as_ref().map(GateRecord::kind),
                deny_record: false,
            },
            Row {
                label: "external_tool_pending",
                resolution: external.resolution.clone(),
                suspends: true,
                gate_record: external.gate_record.as_ref().map(GateRecord::kind),
                deny_record: false,
            },
        ];

        // Expected gate-record kinds spelled once, matched against what the gate
        // constructors returned above.
        let expected_gate_kinds = [
            ("approval_required", Some("approval")),
            ("auth_required", Some("auth")),
            ("resource_blocked", Some("resource")),
            ("await_dependent_run", Some("dependent_run")),
            ("external_tool_pending", Some("external_tool")),
        ];

        assert_eq!(rows.len(), 10, "all ten producer channels covered");

        for row in &rows {
            assert_eq!(
                row.resolution.is_suspension(),
                row.suspends,
                "{}: is_suspension",
                row.label
            );
            if let Some((_, expected)) = expected_gate_kinds.iter().find(|(l, _)| *l == row.label) {
                assert_eq!(
                    row.gate_record, *expected,
                    "{}: gate_record kind",
                    row.label
                );
            }
            assert!(
                !(row.gate_record.is_some() && row.deny_record),
                "{}: at most one side record",
                row.label
            );
        }
    }

    /// The suspension split (#6137): Approval/Auth/Resource are re-entrant gates
    /// (`Blocked`, NOT a suspension), Process/DependentRun/ExternalTool are parked
    /// work (`Suspended`), and a spawned child run completes (`Done`, NOT a
    /// suspension).
    #[test]
    fn suspension_split_matches_host_api_semantics() {
        for gated in [
            approval_required(gate_ref(), "a".to_string(), None),
            auth_required(auth_gate_ref(), vec![], "a".to_string(), None),
            resource_blocked(gate_ref(), "a".to_string()),
        ] {
            assert!(gated.resolution.is_reentrant_gate());
            assert!(
                !gated.resolution.is_suspension(),
                "a re-entrant gate must NOT be a host_api suspension"
            );
        }

        assert!(
            spawned_process(LoopProcessRef::new("process:pid-1").unwrap()).is_suspension(),
            "a spawned process is parked work"
        );
        assert!(
            await_dependent_run(gate_ref(), result_ref(), "a".to_string(), 1, None)
                .resolution
                .is_suspension(),
            "a dependent run is parked work"
        );
        assert!(
            external_tool_pending(gate_ref(), "a".to_string())
                .resolution
                .is_suspension(),
            "an external tool is parked work"
        );
        assert!(
            !spawned_child_run(TurnRunId::new(), result_ref(), "a".to_string(), 1, None)
                .is_suspension(),
            "a spawned child run completes, it does not suspend"
        );
    }

    #[test]
    fn completed_carries_success_verdict_and_minted_result_ref() {
        match completed_ok("staged output") {
            Resolution::Done(outcome) => {
                assert_eq!(outcome.verdict, ToolVerdict::Success);
                assert!(outcome.verdict.is_success());
                assert_eq!(outcome.refs.byte_len, 4096);
                assert_eq!(outcome.summary.as_str(), "staged output");
                assert_eq!(outcome.verdict.child_run(), None);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A `completed` result's loop-derived signals (progress, terminate_hint,
    /// output_digest) and its originating loop result ref survive onto
    /// `Resolution::Done`.
    #[test]
    fn completed_carries_progress_terminate_hint_digest_and_origin() {
        let digest =
            ContentDigest::from_json_value(&serde_json::json!({"k": "v"})).expect("digest");
        match completed(
            result_ref(),
            "did work".to_string(),
            CapabilityProgress::MadeProgress,
            true,
            4096,
            Some(digest),
            None,
        ) {
            Resolution::Done(done) => {
                assert_eq!(done.progress, ResultProgress::MadeProgress);
                assert!(done.terminate_hint.should_terminate());
                assert_eq!(
                    done.refs.output_digest.map(OutputDigest::value),
                    Some(digest.0),
                    "output_digest must survive"
                );
                assert_eq!(
                    done.refs.origin.as_ref().map(LoopRef::as_str),
                    Some(result_ref().as_str()),
                    "the originating loop result ref must be preserved on OutcomeRefs.origin"
                );
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A `failed` result's structured `InvalidInput` diagnostic round-trips its
    /// schema issues (path, code, expected/received) onto the verdict.
    #[test]
    fn failed_invalid_input_diagnostic_round_trips_structured_issues() {
        let detail = CapabilityFailureDetail::InvalidInput {
            issues: vec![CapabilityInputIssue {
                path: "schedule.kind".to_string(),
                code: DispatchInputIssueCode::TypeMismatch,
                expected: Some("integer".to_string()),
                received: Some("string".to_string()),
                schema_path: Some("properties.schedule".to_string()),
            }],
        };
        match failed(
            FailureKind::InputEncode,
            "tool input rejected".to_string(),
            detail,
        ) {
            Resolution::Done(done) => match done.verdict.diagnostic() {
                Some(ModelFailureDiagnostic::InvalidInput { issues }) => {
                    assert_eq!(issues.len(), 1);
                    assert_eq!(issues[0].code, DispatchInputIssueCode::TypeMismatch);
                    assert_eq!(issues[0].path.as_str(), "schedule.kind");
                    assert_eq!(
                        issues[0].expected.as_ref().map(SafeSummary::as_str),
                        Some("integer")
                    );
                    assert_eq!(
                        issues[0].received.as_ref().map(SafeSummary::as_str),
                        Some("string")
                    );
                }
                other => panic!("expected InvalidInput diagnostic, got {other:?}"),
            },
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A `failed` result's producer-scrubbed free-text `Diagnostic` rides the
    /// verdict without losing path-shaped recovery context.
    #[test]
    fn failed_free_text_diagnostic_round_trips_and_preserves_paths() {
        match failed(
            FailureKind::Backend,
            "tool failed".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "backend returned an error".to_string(),
            },
        ) {
            Resolution::Done(done) => match done.verdict.diagnostic() {
                Some(ModelFailureDiagnostic::Diagnostic { text }) => {
                    assert_eq!(text.as_str(), "backend returned an error");
                }
                other => panic!("expected Diagnostic, got {other:?}"),
            },
            other => panic!("expected Done, got {other:?}"),
        }

        // Path-shaped recovery context survives the sanitized resolution
        // boundary instead of becoming an opaque placeholder.
        match failed(
            FailureKind::Backend,
            "tool failed".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "failed reading /etc/passwd".to_string(),
            },
        ) {
            Resolution::Done(done) => match done.verdict.diagnostic() {
                Some(ModelFailureDiagnostic::Diagnostic { text }) => {
                    assert_eq!(text.as_str(), "failed reading /etc/passwd");
                }
                other => panic!("expected Diagnostic, got {other:?}"),
            },
            other => panic!("expected Done, got {other:?}"),
        }

        // A known credential-shaped value that reaches this defense-in-depth
        // boundary is replaced with the fixed diagnostic fallback.
        match failed(
            FailureKind::Backend,
            "tool failed".to_string(),
            CapabilityFailureDetail::Diagnostic {
                text: "provider returned ghp_0123456789abcdef".to_string(),
            },
        ) {
            Resolution::Done(done) => match done.verdict.diagnostic() {
                Some(ModelFailureDiagnostic::Diagnostic { text }) => {
                    assert_eq!(text, &ModelDiagnostic::unavailable());
                }
                other => panic!("expected Diagnostic, got {other:?}"),
            },
            other => panic!("expected Done, got {other:?}"),
        }

        // A secret-shaped `received` value is dropped (not carried raw).
        match failed(
            FailureKind::InputEncode,
            "tool input rejected".to_string(),
            CapabilityFailureDetail::InvalidInput {
                issues: vec![CapabilityInputIssue {
                    path: "token".to_string(),
                    code: DispatchInputIssueCode::InvalidValue,
                    expected: Some("opaque string".to_string()),
                    received: Some("sk-ant-abc123def456".to_string()),
                    schema_path: None,
                }],
            },
        ) {
            Resolution::Done(done) => match done.verdict.diagnostic() {
                Some(ModelFailureDiagnostic::InvalidInput { issues }) => {
                    assert_eq!(issues.len(), 1);
                    assert_eq!(issues[0].path.as_str(), "token");
                    assert_eq!(
                        issues[0].received, None,
                        "a secret-shaped issue field must be dropped, never carried raw"
                    );
                    assert_eq!(
                        issues[0].expected.as_ref().map(SafeSummary::as_str),
                        Some("opaque string")
                    );
                }
                other => panic!("expected InvalidInput, got {other:?}"),
            },
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A denial carries its model-visible reason + redacted summary ON THE
    /// CHANNEL (mirroring the sibling record), and a path-shaped summary redacts
    /// to the placeholder.
    #[test]
    fn denied_channel_carries_reason_kind_and_redacted_summary() {
        let denied_res = denied(
            CapabilityDeniedReasonKind::unknown("network_denied").unwrap(),
            "blocked egress".to_string(),
        );
        match &denied_res.resolution {
            Resolution::Denied(denial) => {
                assert_eq!(denial.reason_kind, Some(DenyReason::NetworkDenied));
                assert_eq!(
                    denial.summary.as_ref().map(SafeSummary::as_str),
                    Some("blocked egress")
                );
                assert_eq!(denial.reason_kind, Some(denied_res.deny_record.reason));
                assert_eq!(
                    denial.summary.as_ref(),
                    Some(&denied_res.deny_record.summary)
                );
            }
            other => panic!("expected Denied, got {other:?}"),
        }

        let denied_res = denied(
            CapabilityDeniedReasonKind::EmptySurface,
            "denied reading /secret/path".to_string(),
        );
        match denied_res.resolution {
            Resolution::Denied(denial) => {
                assert_eq!(denial.reason_kind, Some(DenyReason::PolicyDenied));
                assert_eq!(denial.summary, Some(SafeSummary::placeholder()));
            }
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    /// A loop-originated open-set reason buckets into the model-visible catch-all;
    /// a reason string that already spells a `DenyReason` tag is honored.
    #[test]
    fn deny_record_reason_maps_best_effort_with_policy_fallback() {
        for reason_kind in [
            CapabilityDeniedReasonKind::EmptySurface,
            CapabilityDeniedReasonKind::unknown("hook_denied").unwrap(),
        ] {
            assert_eq!(
                denied(reason_kind, "denied".to_string()).deny_record.reason,
                DenyReason::PolicyDenied
            );
        }
        assert_eq!(
            denied(
                CapabilityDeniedReasonKind::unknown("network_denied").unwrap(),
                "blocked egress".to_string()
            )
            .deny_record
            .reason,
            DenyReason::NetworkDenied
        );
    }

    /// An approval gate carries its resume token and preserved loop gate ref; an
    /// auth gate likewise, and its record carries the credential requirements.
    #[test]
    fn approval_and_auth_gates_carry_resume_token_and_preserved_origin() {
        let approval_resume = CapabilityApprovalResume {
            approval_request_id: ApprovalRequestId::new(),
            resume_token: CapabilityResumeToken::new("approval-resume-1").unwrap(),
            correlation_id: CorrelationId::new(),
            input_ref: CapabilityInputRef::new("input:x").unwrap(),
        };
        let gated = approval_required(
            gate_ref(),
            "awaiting approval".to_string(),
            Some(approval_resume),
        );
        match &gated.resolution {
            Resolution::Blocked(blocked @ Blocked::Approval(_)) => {
                assert_eq!(
                    blocked.resume_token().map(ResumeToken::as_str),
                    Some("approval-resume-1")
                );
                assert_eq!(
                    blocked.origin().map(LoopRef::as_str),
                    Some(gate_ref().as_str())
                );
            }
            other => panic!("expected Blocked::Approval, got {other:?}"),
        }

        let auth_resume = CapabilityAuthResume {
            resume_token: Some(CapabilityResumeToken::new("auth-resume-1").unwrap()),
            disposition: None,
            prior_approval: None,
        };
        let gated = auth_required(
            auth_gate_ref(),
            vec![credential_requirement()],
            "awaiting credential".to_string(),
            Some(auth_resume),
        );
        match &gated.resolution {
            Resolution::Blocked(blocked @ Blocked::Auth(_)) => {
                assert_eq!(
                    blocked.resume_token().map(ResumeToken::as_str),
                    Some("auth-resume-1")
                );
                assert_eq!(
                    blocked.origin().map(LoopRef::as_str),
                    Some(auth_gate_ref().as_str())
                );
            }
            other => panic!("expected Blocked::Auth, got {other:?}"),
        }
        match gated.gate_record {
            Some(GateRecord::Auth {
                credential_requirements,
                ..
            }) => assert_eq!(credential_requirements, vec![credential_requirement()]),
            other => panic!("expected GateRecord::Auth, got {other:?}"),
        }
    }

    /// The auth gate record key is derived DETERMINISTICALLY from the
    /// `gate:auth-{gate_id}` ref (byte-stable resume) — the same key
    /// `GateRef::for_auth_gate` produces.
    #[test]
    fn auth_gate_record_key_is_deterministic_from_the_loop_ref() {
        let gated = auth_required(auth_gate_ref(), vec![], "auth".to_string(), None);
        match &gated.resolution {
            Resolution::Blocked(Blocked::Auth(waypoint)) => {
                assert_eq!(waypoint.gate, GateRef::for_auth_gate("cred-1"));
            }
            other => panic!("expected Blocked::Auth, got {other:?}"),
        }
    }

    /// A spawned-process suspension preserves its loop process ref on the channel.
    #[test]
    fn spawned_process_preserves_the_loop_process_ref_on_the_channel() {
        match spawned_process(LoopProcessRef::new("process:pid-7").unwrap()) {
            Resolution::Suspended(suspension @ Suspension::Process(_)) => {
                assert_eq!(
                    suspension.origin().map(LoopRef::as_str),
                    Some("process:pid-7")
                );
            }
            other => panic!("expected Suspended(Process), got {other:?}"),
        }
    }

    #[test]
    fn child_run_identity_is_preserved_on_the_verdict() {
        let child_run_id = TurnRunId::new();
        match spawned_child_run(child_run_id, result_ref(), "spawned".to_string(), 64, None) {
            Resolution::Done(outcome) => {
                assert_eq!(
                    outcome.verdict.child_run().map(|run| run.as_uuid()),
                    Some(child_run_id.as_uuid())
                );
                assert_eq!(outcome.refs.byte_len, 64);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    /// A dependent run carries its staged result + byte length on the durable
    /// record AND inline on the channel, with the loop origin preserved on both.
    #[test]
    fn dependent_run_record_and_channel_carry_staged_result() {
        let gated = await_dependent_run(
            gate_ref(),
            result_ref(),
            "awaiting dependent".to_string(),
            2048,
            None,
        );
        match &gated.gate_record {
            Some(GateRecord::DependentRun {
                byte_len,
                summary,
                result_origin,
                ..
            }) => {
                assert_eq!(*byte_len, 2048);
                assert_eq!(summary.as_str(), "awaiting dependent");
                assert_eq!(
                    result_origin.as_ref().map(LoopRef::as_str),
                    Some(result_ref().as_str())
                );
            }
            other => panic!("expected GateRecord::DependentRun, got {other:?}"),
        }
        match &gated.resolution {
            Resolution::Suspended(suspension @ Suspension::DependentRun { .. }) => {
                let staged = suspension.dependent_result().expect("inline staged result");
                assert_eq!(staged.byte_len, 2048);
                assert_eq!(staged.summary.as_str(), "awaiting dependent");
                assert_eq!(
                    staged.origin.as_ref().map(LoopRef::as_str),
                    Some(result_ref().as_str())
                );
            }
            other => panic!("expected Suspended(DependentRun), got {other:?}"),
        }
    }

    /// `await_dependent_run`'s `model_observation` rides the inline observation
    /// caption and is redacted (a secret/path-shaped observation degrades).
    #[test]
    fn dependent_run_carries_model_observation_inline_and_redacts() {
        let with_summary = |summary: &str| {
            let mut o = observation("child content");
            o.summary = summary.to_string();
            o
        };

        let gated = await_dependent_run(
            gate_ref(),
            result_ref(),
            "child produced 4 rows".to_string(),
            512,
            Some(with_summary("child preview: 4 rows")),
        );
        let staged = match &gated.resolution {
            Resolution::Suspended(s @ Suspension::DependentRun { .. }) => {
                s.dependent_result().expect("staged").clone()
            }
            other => panic!("expected DependentRun, got {other:?}"),
        };
        assert_eq!(
            staged.observation.as_ref().map(SafeSummary::as_str),
            Some("child preview: 4 rows")
        );
        assert_eq!(staged.summary.as_str(), "child produced 4 rows");

        let gated = await_dependent_run(
            gate_ref(),
            result_ref(),
            "leaked path /etc/passwd".to_string(),
            512,
            Some(with_summary("api key: sk-ant-leak")),
        );
        let staged = match &gated.resolution {
            Resolution::Suspended(s @ Suspension::DependentRun { .. }) => {
                s.dependent_result().expect("staged").clone()
            }
            other => panic!("expected DependentRun, got {other:?}"),
        };
        assert_eq!(staged.observation, None);
        assert_eq!(staged.summary, SafeSummary::placeholder());
    }

    #[test]
    fn an_unsafe_summary_falls_back_to_the_placeholder_never_panics() {
        let gated = resource_blocked(gate_ref(), "leaked path /etc/passwd".to_string());
        match gated.gate_record {
            Some(GateRecord::Resource { summary }) => {
                assert_eq!(summary, SafeSummary::placeholder());
            }
            other => panic!("expected GateRecord::Resource, got {other:?}"),
        }
    }

    #[test]
    fn completed_observation_preview_masks_credentials_and_preserves_continuation() {
        let outcome_refs = |content: &str| match completed(
            result_ref(),
            "ok".to_string(),
            CapabilityProgress::Unknown,
            false,
            10,
            None,
            Some(observation(content)),
        ) {
            Resolution::Done(outcome) => outcome.refs,
            other => panic!("expected Done, got {other:?}"),
        };

        // Structured content with delimiters + "Secretary" retained verbatim.
        let content = "{\"office\": \"Secretary of the Treasury\", \"rows\": [1, 2, 3]}";
        assert_eq!(
            outcome_refs(content)
                .preview
                .as_ref()
                .map(ModelResultPreview::as_str),
            Some(content)
        );

        // A genuine credential is MASKED, not allowed through — the security
        // property is unchanged. Masking keeps the surrounding output visible
        // while the secret still never reaches the model.
        // The PRODUCTION trigger is credential VOCABULARY, not a secret value:
        // extension_search / ironhub results carry "Authenticated with an Attio
        // workspace API key presented as a Bearer header", which refused the whole
        // payload and left the model an unreadable reference (1088 bytes, no preview).
        let vocab_refs = outcome_refs(
            "{\"name\": \"attio\", \"description\": \"Authenticated with a workspace API key presented as a Bearer header.\"}",
        );
        let vocab = vocab_refs
            .preview
            .as_ref()
            .expect("credential VOCABULARY must not drop the preview")
            .as_str();
        assert!(
            vocab.contains("attio"),
            "the payload must survive vocabulary redaction: {vocab}"
        );

        let masked_refs = outcome_refs("token sk-ant-abc123def456");
        let masked = masked_refs
            .preview
            .as_ref()
            .expect("preview is masked, not dropped")
            .as_str();
        assert!(
            !masked.contains("sk-ant-abc123def456"),
            "credential material must never reach the model: {masked}"
        );
        assert!(
            masked.contains("token"),
            "surrounding content must survive redaction: {masked}"
        );

        // Continuation authority is independent of the optional preview.
        let mut suppressed_array = observation("unused");
        let ToolObservationDetail::ResultReference {
            preview,
            total_bytes,
            next_offset,
            item_count,
            ..
        } = &mut suppressed_array.detail
        else {
            panic!("test observation must be a result reference");
        };
        *preview = None;
        *total_bytes = Some(4096);
        *next_offset = Some(2048);
        *item_count = Some(600);
        let refs = match completed(
            result_ref(),
            "ok".to_string(),
            CapabilityProgress::Unknown,
            false,
            4096,
            None,
            Some(suppressed_array),
        ) {
            Resolution::Done(outcome) => outcome.refs,
            other => panic!("expected Done, got {other:?}"),
        };
        assert_eq!(refs.preview, None);
        assert_eq!(refs.preview_meta.total_bytes, Some(4096));
        assert_eq!(refs.preview_meta.next_offset, Some(2048));
        assert_eq!(refs.preview_meta.item_count, Some(600));
    }
}
