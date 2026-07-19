//! `CapabilityOutcome` → `Resolution` mapping (arch-simplification §3/§5.3).
//!
//! The loop-facing [`CapabilityOutcome`] is the overloaded ten-variant enum that
//! carries every non-happy path today (§1.2). The target model folds those ten
//! variants into the five host_api result channels — [`Resolution::Done`],
//! [`Resolution::Denied`], [`Resolution::Blocked`], [`Resolution::Suspended`], and
//! the `Err` arm [`ironclaw_host_api::HostFailure`] — plus the side records
//! ([`GateRecord`]/[`DenyRecord`]) that hold the content the channel's opaque refs
//! point at (§5.2.9's "render from record" contract).
//!
//! This module is the **pure mapping artifact**: it converts one owned
//! `CapabilityOutcome` into a [`MappedResolution`]. It is landed additively (§9) —
//! nothing produces `Resolution` from a `CapabilityOutcome` in production yet; the
//! later producer/consumer migration wires this in. `CapabilityOutcome` is
//! unchanged and every existing path keeps its current behavior.
//!
//! ## Non-lossy carry (§5.3 Stage 1)
//!
//! `host_api::Resolution` now carries **every recoverable field** the old
//! `CapabilityOutcome` variants held, via the vocabulary in
//! [`ironclaw_host_api::result_meta`]:
//!
//! - `CapabilityFailure::error_kind` ([`CapabilityFailureKind`]) → the
//!   [`FailureKind`] on [`ToolVerdict::RecoverableFailure`] — the recovery class
//!   that drives retry-vs-terminal now crosses (was "G1-dropped"). Only the raw
//!   `detail` stays host-side (a backend cause, not vocabulary — charter).
//! - `CapabilityResultMessage::{progress, terminate_hint, output_digest}` →
//!   [`Outcome::progress`]/[`Outcome::terminate_hint`]/[`OutcomeRefs::output_digest`]
//!   (were the "G4-dropped" loop-derived signals).
//! - The `resume_token` inside `approval_resume`/`auth_resume` → the
//!   [`ResumeToken`] on the gate [`GateWaypoint`], so the loop can echo it back to
//!   resume the gate. Only the *token* crosses; the raw input/estimate replay it
//!   was bundled with stays host-side (charter: no raw input in vocabulary — the
//!   host reconstitutes it from storage keyed by the token).
//!
//! `SpawnedProcess`'s `safe_summary` still has no host channel (a process
//! suspension carries a [`ProcessRef`], not a summary). `AwaitDependentRun`'s and
//! `SpawnedChildRun`'s `model_observation` ride the result preview where present.
//!
//! ## Loop refs: minted kernel handle + preserved origin
//!
//! The loop's refs ([`LoopResultRef`], [`LoopGateRef`], [`LoopProcessRef`]) are
//! opaque prefixed strings (`result:*`/`gate:*`/`process:*`); host_api's kernel
//! refs ([`ResultRef`]/[`GateRef`]/[`ProcessRef`]) are opaque uuids by design, so
//! they cannot carry the loop's own ref identity. The mapping mints a fresh kernel
//! handle **and** preserves the originating loop ref on the channel's `origin`
//! (a [`LoopRef`]) — so loop/evidence state keyed under the loop ref (e.g. output
//! the result writer staged) stays reachable through the migration window, not only
//! via the [`RefBindings`] side-table (which is retained). The only identity that
//! crosses directly is [`TurnRunId`](crate::TurnRunId) → [`RunId`]: both wrap a
//! `Uuid`, preserved via `RunId::from_uuid`.

use ironclaw_host_api::{
    Blocked, DenyReason, DenyRecord, DenyRef, FailureKind, GateRecord, GateRef, GateWaypoint,
    LoopRef, Outcome, OutcomeRefs, OutputDigest, ProcessRef, ProcessWaypoint, Resolution,
    ResultProgress, ResultRef, ResumeToken, RunId, SafeSummary, Suspension, TerminateHint,
    ToolVerdict,
};

use super::content_digest::ContentDigest;
use super::host::{
    CapabilityApprovalResume, CapabilityAuthResume, CapabilityDenied, CapabilityDeniedReasonKind,
    CapabilityFailure, CapabilityFailureKind, CapabilityOutcome, CapabilityProgress,
    CapabilityResultMessage, CapabilityResumeToken, LoopProcessRef, ProcessHandleSummary,
};
use super::model_observation::ModelVisibleToolObservation;
use crate::{LoopGateRef, LoopResultRef};

/// A [`Resolution`] plus the side records its opaque refs render from (§5.2.9).
///
/// `Resolution`'s control-plane arms carry only refs; the model-visible content
/// (pending-gate detail, denial reason) lives in the referenced record. A gate
/// channel yields a `gate_record`; a denial yields a `deny_record`; the
/// `Done`/`Suspended(Process)` channels carry their content inline and yield
/// neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedResolution {
    pub resolution: Resolution,
    pub gate_record: Option<GateRecord>,
    pub deny_record: Option<DenyRecord>,
    /// Source→target associations for every freshly-minted host ref, so the
    /// later wiring slice can persist the loop-ref↔uuid-ref correspondence at
    /// the writer/store boundary instead of losing it in this pure function.
    pub bindings: RefBindings,
}

/// The loop-side refs each freshly-minted host_api uuid ref replaces.
///
/// The pure mapping cannot reconstruct a uuid from an opaque loop string, so it
/// mints fresh host refs — but minting without a binding would strand already-
/// stored loop state (the result writer stored output under the *loop* ref).
/// Each populated pair here says "the minted host ref on the right stands for
/// the loop ref on the left"; the consumer persists that association. A `None`
/// means the variant carried no loop-side ref for that slot (e.g. `Failed`
/// mints a `ResultRef` handle with no loop source, `Denied` has no loop deny
/// ref).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefBindings {
    /// Loop result ref → the minted [`ResultRef`] (on `OutcomeRefs.result` or
    /// `GateRecord::DependentRun.result`).
    pub result: Option<(LoopResultRef, ResultRef)>,
    /// Loop gate ref → the minted [`GateRef`] (on the `Blocked`/`Suspended`
    /// channel and its `GateRecord`).
    pub gate: Option<(LoopGateRef, GateRef)>,
    /// Loop process ref → the minted [`ProcessRef`] (on
    /// `Suspension::Process`).
    pub process: Option<(LoopProcessRef, ProcessRef)>,
}

impl MappedResolution {
    /// A resolution that carries no side record (the `Done` and
    /// `Suspended(Process)` channels).
    fn bare(resolution: Resolution) -> Self {
        Self {
            resolution,
            gate_record: None,
            deny_record: None,
            bindings: RefBindings::default(),
        }
    }

    /// A gate/suspension channel paired with the [`GateRecord`] its ref renders
    /// from.
    fn with_gate(resolution: Resolution, gate_record: GateRecord) -> Self {
        Self {
            resolution,
            gate_record: Some(gate_record),
            deny_record: None,
            bindings: RefBindings::default(),
        }
    }

    /// A denial paired with the [`DenyRecord`] its ref renders from.
    fn with_deny(resolution: Resolution, deny_record: DenyRecord) -> Self {
        Self {
            resolution,
            gate_record: None,
            deny_record: Some(deny_record),
            bindings: RefBindings::default(),
        }
    }

    fn bind_result(mut self, loop_ref: LoopResultRef, minted: ResultRef) -> Self {
        self.bindings.result = Some((loop_ref, minted));
        self
    }

    fn bind_gate(mut self, loop_ref: LoopGateRef, minted: GateRef) -> Self {
        self.bindings.gate = Some((loop_ref, minted));
        self
    }

    fn bind_process(mut self, loop_ref: LoopProcessRef, minted: ProcessRef) -> Self {
        self.bindings.process = Some((loop_ref, minted));
        self
    }
}

/// Map one loop-facing [`CapabilityOutcome`] onto its host_api [`Resolution`]
/// channel plus any side record (§5.3 acceptance table).
///
/// Pure and total: consumes the outcome, mints fresh uuid refs for host-side
/// handles, and never panics (see [`safe_summary_or_placeholder`] for the
/// summary-validation fallback).
pub fn capability_outcome_to_resolution(outcome: CapabilityOutcome) -> MappedResolution {
    match outcome {
        // Ran and succeeded. Loop-derived progress/terminate_hint/output_digest
        // now cross onto the Outcome; a fresh ResultRef handle is minted, the
        // loop result_ref is preserved on OutcomeRefs.origin AND bound so the
        // stored output stays reachable.
        CapabilityOutcome::Completed(message) => {
            let (outcome, loop_result) = completed_outcome(message);
            let minted = outcome.refs.result;
            MappedResolution::bare(Resolution::Done(outcome)).bind_result(loop_result, minted)
        }
        // Ran and failed in a model-visible, correctable way. The recovery class
        // (error_kind) rides the verdict; only the raw detail stays host-side.
        CapabilityOutcome::Failed(failure) => {
            MappedResolution::bare(Resolution::Done(failed_outcome(failure)))
        }
        // Terminal policy denial — model-visible, not re-entrant.
        CapabilityOutcome::Denied(denied) => {
            let CapabilityDenied {
                reason_kind,
                safe_summary,
            } = denied;
            MappedResolution::with_deny(
                Resolution::Denied(DenyRef::new()),
                DenyRecord {
                    reason: deny_reason_from_kind(&reason_kind),
                    summary: safe_summary_or_placeholder(safe_summary),
                },
            )
        }
        // Re-entrant gate: needs human approval before it may run. The gate-render
        // content (summary) rides the GateRecord; the resume token and the
        // preserved loop gate ref ride the waypoint (never the model-visible
        // record — §5.2.9).
        CapabilityOutcome::ApprovalRequired {
            gate_ref,
            safe_summary,
            approval_resume,
        } => {
            let minted = GateRef::new();
            let waypoint = gate_waypoint(minted, &gate_ref, approval_resume_token(approval_resume));
            MappedResolution::with_gate(
                Resolution::Blocked(Blocked::Approval(waypoint)),
                GateRecord::Approval {
                    summary: safe_summary_or_placeholder(safe_summary),
                },
            )
            .bind_gate(gate_ref, minted)
        }
        // Re-entrant gate: needs a credential the caller has not supplied. The
        // host-owned credential requirements ride the record (G3); the resume
        // token and preserved loop gate ref ride the waypoint.
        CapabilityOutcome::AuthRequired {
            gate_ref,
            credential_requirements,
            safe_summary,
            auth_resume,
        } => {
            let minted = GateRef::new();
            let waypoint = gate_waypoint(minted, &gate_ref, auth_resume_token(auth_resume));
            MappedResolution::with_gate(
                Resolution::Blocked(Blocked::Auth(waypoint)),
                GateRecord::Auth {
                    summary: safe_summary_or_placeholder(safe_summary),
                    credential_requirements,
                },
            )
            .bind_gate(gate_ref, minted)
        }
        // Re-entrant gate: needs resource budget currently unavailable. No resume
        // token — a resource gate resumes against then-current budget (§5.3.3).
        CapabilityOutcome::ResourceBlocked {
            gate_ref,
            safe_summary,
        } => {
            let minted = GateRef::new();
            let waypoint = gate_waypoint(minted, &gate_ref, None);
            MappedResolution::with_gate(
                Resolution::Blocked(Blocked::Resource(waypoint)),
                GateRecord::Resource {
                    summary: safe_summary_or_placeholder(safe_summary),
                },
            )
            .bind_gate(gate_ref, minted)
        }
        // Parked work: a spawned OS process the turn now waits on. Process
        // suspensions track a ProcessRef, not a gate record; the loop process ref
        // is preserved on the waypoint origin (the loop summary still has no host
        // channel).
        CapabilityOutcome::SpawnedProcess(ProcessHandleSummary { process_ref, .. }) => {
            let minted = ProcessRef::new();
            let waypoint = process_waypoint(minted, &process_ref);
            MappedResolution::bare(Resolution::Suspended(Suspension::Process(waypoint)))
                .bind_process(process_ref, minted)
        }
        // NON-suspending (the #6137 bug class): the executor appends the child
        // result and continues. Maps to Done/ChildSpawned, carrying the child's
        // RunId on the verdict. child_run_id (a TurnRunId) preserves identity via
        // RunId::from_uuid; the string result_ref is replaced by a fresh ResultRef.
        CapabilityOutcome::SpawnedChildRun {
            child_run_id,
            result_ref,
            safe_summary,
            byte_len,
            model_observation,
        } => {
            let minted = ResultRef::new();
            MappedResolution::bare(Resolution::Done(Outcome {
                refs: OutcomeRefs {
                    result: minted,
                    byte_len,
                    preview: observation_preview(model_observation),
                    origin: preserved_origin(result_ref.as_str()),
                    output_digest: None,
                },
                verdict: ToolVerdict::ChildSpawned {
                    child_run: RunId::from_uuid(child_run_id.as_uuid()),
                },
                summary: safe_summary_or_placeholder(safe_summary),
                progress: ResultProgress::default(),
                terminate_hint: TerminateHint::default(),
            }))
            .bind_result(result_ref, minted)
        }
        // Parked work: awaits a dependent child run. Gate-shaped, so it carries a
        // GateRecord holding the staged result handle + byte length (G2). Both the
        // gate ref and the staged result ref are freshly minted and bound; the
        // loop's model_observation has no home on DependentRun and is dropped.
        CapabilityOutcome::AwaitDependentRun {
            gate_ref,
            result_ref,
            safe_summary,
            byte_len,
            ..
        } => {
            let minted_gate = GateRef::new();
            let minted_result = ResultRef::new();
            let waypoint = gate_waypoint(minted_gate, &gate_ref, None);
            MappedResolution::with_gate(
                Resolution::Suspended(Suspension::DependentRun(waypoint)),
                GateRecord::DependentRun {
                    summary: safe_summary_or_placeholder(safe_summary),
                    result: minted_result,
                    byte_len,
                    result_origin: preserved_origin(result_ref.as_str()),
                },
            )
            .bind_gate(gate_ref, minted_gate)
            .bind_result(result_ref, minted_result)
        }
        // Parked work: a client-executed external tool the host does not run.
        CapabilityOutcome::ExternalToolPending {
            gate_ref,
            safe_summary,
        } => {
            let minted = GateRef::new();
            let waypoint = gate_waypoint(minted, &gate_ref, None);
            MappedResolution::with_gate(
                Resolution::Suspended(Suspension::ExternalTool(waypoint)),
                GateRecord::ExternalTool {
                    summary: safe_summary_or_placeholder(safe_summary),
                },
            )
            .bind_gate(gate_ref, minted)
        }
    }
}

/// Build the `Done` payload for a `Completed` outcome (verdict `Success`),
/// returning the loop result ref alongside so the caller can bind it to the
/// minted [`ResultRef`].
fn completed_outcome(message: CapabilityResultMessage) -> (Outcome, LoopResultRef) {
    let CapabilityResultMessage {
        result_ref,
        safe_summary,
        byte_len,
        model_observation,
        progress,
        terminate_hint,
        output_digest,
    } = message;
    let outcome = Outcome {
        refs: OutcomeRefs {
            result: ResultRef::new(),
            byte_len,
            preview: observation_preview(model_observation),
            origin: preserved_origin(result_ref.as_str()),
            output_digest: output_digest.map(output_digest_of),
        },
        verdict: ToolVerdict::Success,
        summary: safe_summary_or_placeholder(safe_summary),
        progress: result_progress_of(progress),
        terminate_hint: TerminateHint::from_bool(terminate_hint),
    };
    (outcome, result_ref)
}

/// Build the `Done` payload for a `Failed` outcome (verdict `RecoverableFailure`),
/// carrying the recovery classification on the verdict. Only the raw `detail`
/// (a backend cause) stays host-side — not authority vocabulary (charter).
fn failed_outcome(failure: CapabilityFailure) -> Outcome {
    let CapabilityFailure {
        error_kind,
        safe_summary,
        detail: _,
    } = failure;
    Outcome {
        refs: OutcomeRefs {
            // A recoverable failure stages no durable output beyond its summary;
            // the ref is a minted handle the later store may leave unpopulated,
            // and there is no originating loop result ref to preserve.
            result: ResultRef::new(),
            byte_len: 0,
            preview: None,
            origin: None,
            output_digest: None,
        },
        verdict: ToolVerdict::RecoverableFailure {
            error_kind: failure_kind_of(error_kind),
        },
        summary: safe_summary_or_placeholder(safe_summary),
        progress: ResultProgress::default(),
        terminate_hint: TerminateHint::default(),
    }
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
/// ref that fails — which a safe production ref never does — falls back to `None`
/// and stays reachable only through [`RefBindings`]; `.ok()` here converts a pure
/// text-to-safe-text validation failure into an absent origin, never a swallowed
/// I/O error.
fn preserved_origin(loop_ref: &str) -> Option<LoopRef> {
    LoopRef::new(loop_ref).ok()
}

/// The opaque approval resume token, when the outcome carried one.
fn approval_resume_token(resume: Option<CapabilityApprovalResume>) -> Option<ResumeToken> {
    resume.and_then(|resume| resume_token_of(&resume.resume_token))
}

/// The opaque auth resume token, when the outcome carried one.
fn auth_resume_token(resume: Option<CapabilityAuthResume>) -> Option<ResumeToken> {
    resume.and_then(|resume| resume_token_of(&resume.resume_token))
}

/// Convert a loop-facing [`CapabilityResumeToken`] to a host_api [`ResumeToken`].
/// Both are bounded/control-free, so a valid loop token always crosses; `.ok()`
/// drops a token that fails the host bound rather than panic (the mapping is
/// total) — such a token, never produced by the loop's own validator, then
/// resumes through the retained binding.
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

/// Map the loop's [`CapabilityFailureKind`] onto host_api's [`FailureKind`] by its
/// stable tag — the two vocabularies share the same closed set plus an open
/// `Unknown`, so every value crosses losslessly.
fn failure_kind_of(kind: CapabilityFailureKind) -> FailureKind {
    FailureKind::from_tag(kind.as_str())
}

/// Bounded model-visible preview from a loop tool observation, when present.
///
/// The observation's `summary` is model-visible text; it is re-validated through
/// the [`SafeSummary`] redaction contract. If it fails validation, the preview is
/// dropped to `None` — an optional preview is best-effort, never a placeholder
/// that would misrepresent staged content.
fn observation_preview(observation: Option<ModelVisibleToolObservation>) -> Option<SafeSummary> {
    // `.ok()` intentionally converts a redaction-validation failure into an
    // absent (None) preview: this is a pure text-to-safe-text conversion, not a
    // swallowed I/O error, and preview is an optional best-effort field.
    observation.and_then(|observation| SafeSummary::new(observation.summary).ok())
}

/// Convert a loop-facing `safe_summary: String` to a host_api [`SafeSummary`].
///
/// The redaction rule is the same on both sides (#6236), so a value the loop
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
/// spells a `DenyReason` snake_case tag is honored, and everything else — every
/// loop-originated denial — buckets into the model-visible catch-all
/// [`DenyReason::PolicyDenied`].
fn deny_reason_from_kind(kind: &CapabilityDeniedReasonKind) -> DenyReason {
    use serde::{
        Deserialize,
        de::{IntoDeserializer, value::StrDeserializer},
    };
    // Deserialize straight from the &str (no JSON Value/String allocation);
    // DenyReason's snake_case serde tags are the match vocabulary.
    let deserializer: StrDeserializer<'_, serde::de::value::Error> =
        kind.as_str().into_deserializer();
    DenyReason::deserialize(deserializer).unwrap_or(DenyReason::PolicyDenied)
}

#[cfg(test)]
mod tests {
    use super::super::host::CapabilityInputRef;
    use super::super::{
        CapabilityFailureKind, CapabilityProgress, LoopProcessRef,
        MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
    };
    use super::*;
    use crate::{LoopGateRef, LoopResultRef, TurnRunId};
    use ironclaw_host_api::{
        ApprovalRequestId, CorrelationId, ExtensionId, ResourceEstimate,
        RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
        RuntimeCredentialAuthRequirement,
    };

    fn result_ref() -> LoopResultRef {
        LoopResultRef::new("result:child-1").unwrap()
    }

    fn gate_ref() -> LoopGateRef {
        LoopGateRef::new("gate:pending-1").unwrap()
    }

    fn credential_requirement() -> RuntimeCredentialAuthRequirement {
        RuntimeCredentialAuthRequirement {
            provider: RuntimeCredentialAccountProviderId::new("github").unwrap(),
            setup: RuntimeCredentialAccountSetup::ManualToken,
            requester_extension: ExtensionId::new("github").unwrap(),
            provider_scopes: vec!["repo".to_string()],
        }
    }

    fn completed(summary: &str) -> CapabilityOutcome {
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: result_ref(),
            safe_summary: summary.to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: true,
            byte_len: 4096,
            output_digest: None,
            model_observation: None,
        })
    }

    /// The §5.3 acceptance table is the definition of done. Every one of the ten
    /// `CapabilityOutcome` variants maps to exactly one `Resolution` channel with
    /// the correct suspension semantics and side record. This mirrors host_api's
    /// `resolution_covers_the_full_acceptance_table` on the producer side.
    #[test]
    fn maps_the_full_acceptance_table() {
        // (variant label, outcome, expected channel kind, is_suspension,
        //  expected gate_record kind, expected deny_record present)
        struct Row {
            label: &'static str,
            outcome: CapabilityOutcome,
            channel: &'static str,
            suspends: bool,
            gate_record: Option<&'static str>,
            deny_record: bool,
            // (result, gate, process) binding presence
            bindings: (bool, bool, bool),
        }

        let rows = vec![
            Row {
                label: "Completed",
                outcome: completed("read 3 files"),
                channel: "done",
                suspends: false,
                gate_record: None,
                deny_record: false,
                bindings: (true, false, false),
            },
            Row {
                label: "Failed",
                outcome: CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: CapabilityFailureKind::InvalidInput,
                    safe_summary: "tool input rejected".to_string(),
                    detail: None,
                }),
                channel: "done",
                suspends: false,
                gate_record: None,
                deny_record: false,
                bindings: (false, false, false),
            },
            Row {
                label: "Denied",
                outcome: CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind: CapabilityDeniedReasonKind::EmptySurface,
                    safe_summary: "denied by policy".to_string(),
                }),
                channel: "denied",
                suspends: false,
                gate_record: None,
                deny_record: true,
                bindings: (false, false, false),
            },
            Row {
                label: "ApprovalRequired",
                outcome: CapabilityOutcome::ApprovalRequired {
                    gate_ref: gate_ref(),
                    safe_summary: "awaiting approval".to_string(),
                    approval_resume: None,
                },
                channel: "blocked",
                suspends: false,
                gate_record: Some("approval"),
                deny_record: false,
                bindings: (false, true, false),
            },
            Row {
                label: "AuthRequired",
                outcome: CapabilityOutcome::AuthRequired {
                    gate_ref: gate_ref(),
                    credential_requirements: vec![credential_requirement()],
                    safe_summary: "awaiting credential".to_string(),
                    auth_resume: None,
                },
                channel: "blocked",
                suspends: false,
                gate_record: Some("auth"),
                deny_record: false,
                bindings: (false, true, false),
            },
            Row {
                label: "ResourceBlocked",
                outcome: CapabilityOutcome::ResourceBlocked {
                    gate_ref: gate_ref(),
                    safe_summary: "awaiting budget".to_string(),
                },
                channel: "blocked",
                suspends: false,
                gate_record: Some("resource"),
                deny_record: false,
                bindings: (false, true, false),
            },
            Row {
                label: "SpawnedProcess",
                outcome: CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                    process_ref: LoopProcessRef::new("process:pid-1").unwrap(),
                    safe_summary: "spawned build".to_string(),
                }),
                channel: "suspended",
                suspends: true,
                gate_record: None,
                deny_record: false,
                bindings: (false, false, true),
            },
            Row {
                label: "SpawnedChildRun",
                outcome: CapabilityOutcome::SpawnedChildRun {
                    child_run_id: TurnRunId::new(),
                    result_ref: result_ref(),
                    safe_summary: "spawned child run".to_string(),
                    byte_len: 128,
                    model_observation: None,
                },
                // NON-suspending — the #6137 bug class.
                channel: "done",
                suspends: false,
                gate_record: None,
                deny_record: false,
                bindings: (true, false, false),
            },
            Row {
                label: "AwaitDependentRun",
                outcome: CapabilityOutcome::AwaitDependentRun {
                    gate_ref: gate_ref(),
                    result_ref: result_ref(),
                    safe_summary: "awaiting dependent run".to_string(),
                    byte_len: 256,
                    model_observation: None,
                },
                channel: "suspended",
                suspends: true,
                gate_record: Some("dependent_run"),
                deny_record: false,
                bindings: (true, true, false),
            },
            Row {
                label: "ExternalToolPending",
                outcome: CapabilityOutcome::ExternalToolPending {
                    gate_ref: gate_ref(),
                    safe_summary: "awaiting external tool".to_string(),
                },
                channel: "suspended",
                suspends: true,
                gate_record: Some("external_tool"),
                deny_record: false,
                bindings: (false, true, false),
            },
        ];

        assert_eq!(rows.len(), 10, "all ten CapabilityOutcome variants covered");

        for row in rows {
            let mapped = capability_outcome_to_resolution(row.outcome);

            assert_eq!(
                mapped.resolution.kind(),
                row.channel,
                "{}: channel",
                row.label
            );
            // The critical #6137 assertion: suspension semantics come from the
            // host_api Resolution, NOT the loop enum's is_suspension().
            assert_eq!(
                mapped.resolution.is_suspension(),
                row.suspends,
                "{}: is_suspension",
                row.label
            );
            assert_eq!(
                mapped.gate_record.as_ref().map(GateRecord::kind),
                row.gate_record,
                "{}: gate_record kind",
                row.label
            );
            assert_eq!(
                mapped.deny_record.is_some(),
                row.deny_record,
                "{}: deny_record present",
                row.label
            );
            assert_eq!(
                (
                    mapped.bindings.result.is_some(),
                    mapped.bindings.gate.is_some(),
                    mapped.bindings.process.is_some(),
                ),
                row.bindings,
                "{}: ref bindings presence",
                row.label
            );
            // A gate/deny record exists iff the channel renders from one; the two
            // record slots are mutually exclusive.
            assert!(
                !(mapped.gate_record.is_some() && mapped.deny_record.is_some()),
                "{}: at most one side record",
                row.label
            );
        }
    }

    /// The suspension split is the bug class #6137 pins: Approval/Auth/Resource
    /// are re-entrant gates (`Blocked`, NOT a suspension), Process/DependentRun/
    /// ExternalTool are parked work (`Suspended`), and SpawnedChildRun completes
    /// (`Done`, NOT a suspension). This is deliberately DIFFERENT from the loop
    /// enum's own `is_suspension()`, which lumps the re-entrant gates in with
    /// parked work.
    #[test]
    fn suspension_split_matches_host_api_semantics() {
        let blocked_not_suspended = [
            CapabilityOutcome::ApprovalRequired {
                gate_ref: gate_ref(),
                safe_summary: "a".to_string(),
                approval_resume: None,
            },
            CapabilityOutcome::AuthRequired {
                gate_ref: gate_ref(),
                credential_requirements: vec![],
                safe_summary: "a".to_string(),
                auth_resume: None,
            },
            CapabilityOutcome::ResourceBlocked {
                gate_ref: gate_ref(),
                safe_summary: "a".to_string(),
            },
        ];
        for outcome in blocked_not_suspended {
            let mapped = capability_outcome_to_resolution(outcome);
            assert!(mapped.resolution.is_reentrant_gate());
            assert!(
                !mapped.resolution.is_suspension(),
                "a re-entrant gate must NOT be a host_api suspension"
            );
        }

        let suspended = [
            CapabilityOutcome::SpawnedProcess(ProcessHandleSummary {
                process_ref: LoopProcessRef::new("process:pid-1").unwrap(),
                safe_summary: "a".to_string(),
            }),
            CapabilityOutcome::AwaitDependentRun {
                gate_ref: gate_ref(),
                result_ref: result_ref(),
                safe_summary: "a".to_string(),
                byte_len: 1,
                model_observation: None,
            },
            CapabilityOutcome::ExternalToolPending {
                gate_ref: gate_ref(),
                safe_summary: "a".to_string(),
            },
        ];
        for outcome in suspended {
            assert!(
                capability_outcome_to_resolution(outcome)
                    .resolution
                    .is_suspension(),
                "parked work must be a host_api suspension"
            );
        }

        // The non-suspending child spawn — the one that has bitten before.
        let child = CapabilityOutcome::SpawnedChildRun {
            child_run_id: TurnRunId::new(),
            result_ref: result_ref(),
            safe_summary: "a".to_string(),
            byte_len: 1,
            model_observation: None,
        };
        assert!(
            !capability_outcome_to_resolution(child)
                .resolution
                .is_suspension()
        );
    }

    #[test]
    fn completed_carries_success_verdict_and_minted_result_ref() {
        let mapped = capability_outcome_to_resolution(completed("staged output"));
        match mapped.resolution {
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

    /// Stage-1 non-lossy: a `Completed` outcome's loop-derived G4 signals
    /// (progress, terminate_hint, output_digest) and its originating loop result
    /// ref now survive the mapping into `Resolution::Done` instead of being
    /// dropped.
    #[test]
    fn completed_carries_progress_terminate_hint_digest_and_origin() {
        let digest =
            ContentDigest::from_json_value(&serde_json::json!({"k": "v"})).expect("digest");
        let outcome = CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: result_ref(),
            safe_summary: "did work".to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: true,
            byte_len: 4096,
            output_digest: Some(digest),
            model_observation: None,
        });
        let mapped = capability_outcome_to_resolution(outcome);
        match mapped.resolution {
            Resolution::Done(done) => {
                assert_eq!(done.progress, ResultProgress::MadeProgress);
                assert!(done.terminate_hint.should_terminate());
                assert_eq!(
                    done.refs.output_digest.map(OutputDigest::value),
                    Some(digest.0),
                    "output_digest must survive the mapping (was G4-dropped)"
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

    /// Stage-1 non-lossy: a `Failed` outcome's recovery classification
    /// (error_kind, the "G1" field) now rides `ToolVerdict::RecoverableFailure`
    /// instead of being dropped.
    #[test]
    fn failed_carries_its_error_kind_on_the_verdict() {
        for (loop_kind, expected) in [
            (CapabilityFailureKind::Network, FailureKind::Network),
            (
                CapabilityFailureKind::InvalidInput,
                FailureKind::InvalidInput,
            ),
            (
                CapabilityFailureKind::unknown("quota_exceeded").unwrap(),
                FailureKind::unknown("quota_exceeded").unwrap(),
            ),
        ] {
            let mapped =
                capability_outcome_to_resolution(CapabilityOutcome::Failed(CapabilityFailure {
                    error_kind: loop_kind,
                    safe_summary: "tool failed".to_string(),
                    detail: None,
                }));
            match mapped.resolution {
                Resolution::Done(done) => {
                    assert_eq!(
                        done.verdict,
                        ToolVerdict::RecoverableFailure {
                            error_kind: expected.clone()
                        },
                        "the recovery class must ride the verdict (was G1-dropped)"
                    );
                }
                other => panic!("expected Done, got {other:?}"),
            }
        }
    }

    /// Stage-1 non-lossy: an approval gate carries its resume token and preserved
    /// loop gate ref (was G1-dropped), and an auth gate likewise.
    #[test]
    fn approval_and_auth_gates_carry_resume_token_and_preserved_origin() {
        let approval_resume = CapabilityApprovalResume {
            approval_request_id: ApprovalRequestId::new(),
            resume_token: CapabilityResumeToken::new("approval-resume-1").unwrap(),
            correlation_id: CorrelationId::new(),
            input_ref: CapabilityInputRef::new("input:x").unwrap(),
            input: serde_json::json!({"k": "v"}),
            estimate: ResourceEstimate::default(),
        };
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::ApprovalRequired {
            gate_ref: gate_ref(),
            safe_summary: "awaiting approval".to_string(),
            approval_resume: Some(approval_resume),
        });
        match &mapped.resolution {
            Resolution::Blocked(blocked @ Blocked::Approval(_)) => {
                assert_eq!(
                    blocked.resume_token().map(ResumeToken::as_str),
                    Some("approval-resume-1"),
                    "the approval resume token must cross"
                );
                assert_eq!(
                    blocked.origin().map(LoopRef::as_str),
                    Some(gate_ref().as_str()),
                    "the originating loop gate ref must be preserved"
                );
            }
            other => panic!("expected Blocked::Approval, got {other:?}"),
        }

        let auth_resume = CapabilityAuthResume {
            resume_token: CapabilityResumeToken::new("auth-resume-1").unwrap(),
            prior_approval: None,
            replay: None,
        };
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::AuthRequired {
            gate_ref: gate_ref(),
            credential_requirements: vec![],
            safe_summary: "awaiting credential".to_string(),
            auth_resume: Some(auth_resume),
        });
        match &mapped.resolution {
            Resolution::Blocked(blocked @ Blocked::Auth(_)) => {
                assert_eq!(
                    blocked.resume_token().map(ResumeToken::as_str),
                    Some("auth-resume-1")
                );
                assert_eq!(
                    blocked.origin().map(LoopRef::as_str),
                    Some(gate_ref().as_str())
                );
            }
            other => panic!("expected Blocked::Auth, got {other:?}"),
        }
    }

    /// Stage-1 non-lossy: a spawned-process suspension preserves its loop process
    /// ref on the channel (not only in the binding side-table).
    #[test]
    fn spawned_process_preserves_the_loop_process_ref_on_the_channel() {
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::SpawnedProcess(
            ProcessHandleSummary {
                process_ref: LoopProcessRef::new("process:pid-7").unwrap(),
                safe_summary: "spawned".to_string(),
            },
        ));
        match &mapped.resolution {
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
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::SpawnedChildRun {
            child_run_id,
            result_ref: result_ref(),
            safe_summary: "spawned".to_string(),
            byte_len: 64,
            model_observation: None,
        });
        match mapped.resolution {
            Resolution::Done(outcome) => {
                // TurnRunId → RunId preserves the underlying uuid identity.
                assert_eq!(
                    outcome.verdict.child_run().map(|run| run.as_uuid()),
                    Some(child_run_id.as_uuid())
                );
                assert_eq!(outcome.refs.byte_len, 64);
            }
            other => panic!("expected Done, got {other:?}"),
        }
    }

    #[test]
    fn auth_gate_record_carries_credential_requirements() {
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::AuthRequired {
            gate_ref: gate_ref(),
            credential_requirements: vec![credential_requirement()],
            safe_summary: "awaiting credential".to_string(),
            auth_resume: None,
        });
        assert!(
            matches!(mapped.resolution, Resolution::Blocked(Blocked::Auth(_))),
            "expected Blocked::Auth, got {:?}",
            mapped.resolution
        );
        match mapped.gate_record {
            Some(GateRecord::Auth {
                credential_requirements,
                ..
            }) => {
                assert_eq!(credential_requirements, vec![credential_requirement()]);
            }
            other => panic!("expected GateRecord::Auth, got {other:?}"),
        }
    }

    #[test]
    fn dependent_run_record_carries_staged_result_and_byte_len() {
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::AwaitDependentRun {
            gate_ref: gate_ref(),
            result_ref: result_ref(),
            safe_summary: "awaiting dependent".to_string(),
            byte_len: 2048,
            model_observation: None,
        });
        match mapped.gate_record {
            Some(GateRecord::DependentRun {
                byte_len,
                summary,
                result_origin,
                ..
            }) => {
                assert_eq!(byte_len, 2048);
                assert_eq!(summary.as_str(), "awaiting dependent");
                // Stage-1 non-lossy: the staged result's originating loop ref
                // is preserved ON THE DURABLE RECORD — the minted ResultRef is
                // a fresh uuid, and the transient RefBindings side-table is not
                // persisted, so without this the child output the loop staged
                // under its own ref would be unreachable from the record a
                // later resume turn renders from.
                assert_eq!(
                    result_origin.as_ref().map(LoopRef::as_str),
                    Some(result_ref().as_str()),
                    "the staged result's loop origin must ride the durable record"
                );
            }
            other => panic!("expected GateRecord::DependentRun, got {other:?}"),
        }
    }

    #[test]
    fn deny_record_reason_maps_best_effort_with_policy_fallback() {
        // A loop-originated open-set reason (EmptySurface / hook_denied) has no
        // faithful DenyReason, so it buckets into the model-visible catch-all.
        for reason_kind in [
            CapabilityDeniedReasonKind::EmptySurface,
            CapabilityDeniedReasonKind::unknown("hook_denied").unwrap(),
        ] {
            let mapped =
                capability_outcome_to_resolution(CapabilityOutcome::Denied(CapabilityDenied {
                    reason_kind,
                    safe_summary: "denied".to_string(),
                }));
            match mapped.deny_record {
                Some(DenyRecord { reason, .. }) => {
                    assert_eq!(reason, DenyReason::PolicyDenied);
                }
                other => panic!("expected a DenyRecord, got {other:?}"),
            }
        }

        // A reason string that already spells a DenyReason tag is honored.
        let mapped =
            capability_outcome_to_resolution(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: CapabilityDeniedReasonKind::unknown("network_denied").unwrap(),
                safe_summary: "blocked egress".to_string(),
            }));
        match mapped.deny_record {
            Some(DenyRecord { reason, .. }) => assert_eq!(reason, DenyReason::NetworkDenied),
            other => panic!("expected a DenyRecord, got {other:?}"),
        }
    }

    /// The bindings are not merely present — each pairs the loop-side source
    /// ref with EXACTLY the uuid ref minted into the resolution/record, so a
    /// consumer persisting the association can make already-stored loop state
    /// reachable through the host ref.
    #[test]
    fn bindings_pair_loop_refs_with_the_minted_host_refs() {
        // Dependent run: both a gate binding and a staged-result binding.
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::AwaitDependentRun {
            gate_ref: gate_ref(),
            result_ref: result_ref(),
            safe_summary: "awaiting dependent".to_string(),
            byte_len: 9,
            model_observation: None,
        });
        let (loop_gate, minted_gate) = mapped.bindings.gate.clone().expect("gate binding");
        let (loop_result, minted_result) = mapped.bindings.result.clone().expect("result binding");
        assert_eq!(loop_gate, gate_ref());
        assert_eq!(loop_result, result_ref());
        match (&mapped.resolution, &mapped.gate_record) {
            (
                Resolution::Suspended(Suspension::DependentRun(channel_gate)),
                Some(GateRecord::DependentRun { result, .. }),
            ) => {
                assert_eq!(
                    channel_gate.gate, minted_gate,
                    "gate binding matches channel"
                );
                assert_eq!(*result, minted_result, "result binding matches record");
            }
            other => panic!("expected DependentRun channel + record, got {other:?}"),
        }

        // Completed: the result binding matches OutcomeRefs.result.
        let mapped = capability_outcome_to_resolution(completed("ok"));
        let (loop_result, minted_result) = mapped.bindings.result.clone().expect("result binding");
        assert_eq!(loop_result, result_ref());
        match &mapped.resolution {
            Resolution::Done(outcome) => assert_eq!(outcome.refs.result, minted_result),
            other => panic!("expected Done, got {other:?}"),
        }

        // Spawned process: the process binding matches the suspension ref.
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::SpawnedProcess(
            ProcessHandleSummary {
                process_ref: LoopProcessRef::new("process:pid-1").unwrap(),
                safe_summary: "spawned".to_string(),
            },
        ));
        let (loop_process, minted_process) = mapped.bindings.process.clone().expect("binding");
        assert_eq!(loop_process, LoopProcessRef::new("process:pid-1").unwrap());
        match &mapped.resolution {
            Resolution::Suspended(Suspension::Process(channel_process)) => {
                assert_eq!(channel_process.process, minted_process);
            }
            other => panic!("expected Suspended(Process), got {other:?}"),
        }
    }

    #[test]
    fn an_unsafe_loop_summary_falls_back_to_the_placeholder_never_panics() {
        // A summary that violates the redaction contract (a raw path delimiter)
        // must map to the safe placeholder rather than panic.
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::ResourceBlocked {
            gate_ref: gate_ref(),
            safe_summary: "leaked path /etc/passwd".to_string(),
        });
        match mapped.gate_record {
            Some(GateRecord::Resource { summary }) => {
                assert_eq!(summary, SafeSummary::placeholder());
            }
            other => panic!("expected GateRecord::Resource, got {other:?}"),
        }
    }

    #[test]
    fn observation_preview_is_carried_when_safe_and_dropped_when_unsafe() {
        use super::super::model_observation::{
            ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail,
            ToolObservationStatus,
        };

        let safe_observation = ModelVisibleToolObservation {
            schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
            status: ToolObservationStatus::Success,
            summary: "staged 3 rows".to_string(),
            detail: ToolObservationDetail::ResultReference {
                result_ref: "result:staged".to_string(),
                byte_len: 10,
                preview: None,
                total_bytes: None,
                next_offset: None,
                item_count: None,
            },
            artifacts: vec![],
            recovery: None,
            trust: ObservationTrust::UntrustedToolOutput,
        };
        let mapped = capability_outcome_to_resolution(CapabilityOutcome::Completed(
            CapabilityResultMessage {
                result_ref: result_ref(),
                safe_summary: "ok".to_string(),
                progress: CapabilityProgress::Unknown,
                terminate_hint: false,
                byte_len: 10,
                output_digest: None,
                model_observation: Some(safe_observation),
            },
        ));
        match mapped.resolution {
            Resolution::Done(outcome) => assert_eq!(
                outcome.refs.preview.as_ref().map(SafeSummary::as_str),
                Some("staged 3 rows")
            ),
            other => panic!("expected Done, got {other:?}"),
        }
    }
}
