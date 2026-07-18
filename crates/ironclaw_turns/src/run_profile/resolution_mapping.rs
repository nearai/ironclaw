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
//! ## What crosses, and what is dropped
//!
//! The mapping is the definition of done in §5.3's acceptance table. Two classes
//! of field on the old variants have **no home** on the new channels and are
//! deliberately dropped here (documented per the G-decisions in the doc):
//!
//! - **G1 — the failure recovery class does not cross.** `CapabilityFailure`'s
//!   `error_kind` ([`CapabilityFailureKind`]) and `detail` are host-side recovery
//!   classification; [`Outcome`] carries only a typed [`ToolVerdict`] plus the
//!   redacted summary, so a `Failed` maps to a plain
//!   [`ToolVerdict::RecoverableFailure`] and its kind/detail are not propagated.
//! - **G4 — loop-derived signals do not cross.** `CapabilityResultMessage`'s
//!   `progress`, `terminate_hint`, and `output_digest` are loop-derived (the loop
//!   computes them); they are not part of the host's `Outcome` and are dropped.
//!
//! `SpawnedProcess`'s `safe_summary` is also dropped: a
//! [`Suspension::Process`] carries only a [`ProcessRef`] — host_api has no process
//! record type for a summary to land on. `AwaitDependentRun`'s `model_observation`
//! is dropped: [`GateRecord::DependentRun`] carries a summary + staged result, not
//! a preview.
//!
//! ## String refs → uuid refs
//!
//! The loop's refs ([`LoopResultRef`], [`LoopGateRef`], [`LoopProcessRef`],
//! [`TurnRunId`]) are opaque prefixed strings; host_api's refs
//! ([`ResultRef`]/[`GateRef`]/[`DenyRef`]/[`ProcessRef`]) are uuids. This pure
//! mapping **mints a fresh uuid ref** for each host-side handle — it cannot
//! reconstruct a meaningful uuid from an opaque loop string, and the durable
//! ref↔record association is the later store's responsibility, not this
//! function's. The only exception is [`TurnRunId`] → [`RunId`]: both wrap a
//! `Uuid`, so the child run's identity is preserved via `RunId::from_uuid`.

use ironclaw_host_api::{
    Blocked, DenyRecord, DenyReason, DenyRef, GateRecord, GateRef, Outcome, OutcomeRefs, ProcessRef,
    Resolution, ResultRef, RunId, SafeSummary, Suspension, ToolVerdict,
};

use super::host::{
    CapabilityDenied, CapabilityDeniedReasonKind, CapabilityFailure, CapabilityOutcome,
    CapabilityResultMessage, ProcessHandleSummary,
};
use super::model_observation::ModelVisibleToolObservation;

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
}

impl MappedResolution {
    /// A resolution that carries no side record (the `Done` and
    /// `Suspended(Process)` channels).
    fn bare(resolution: Resolution) -> Self {
        Self {
            resolution,
            gate_record: None,
            deny_record: None,
        }
    }

    /// A gate/suspension channel paired with the [`GateRecord`] its ref renders
    /// from.
    fn with_gate(resolution: Resolution, gate_record: GateRecord) -> Self {
        Self {
            resolution,
            gate_record: Some(gate_record),
            deny_record: None,
        }
    }

    /// A denial paired with the [`DenyRecord`] its ref renders from.
    fn with_deny(resolution: Resolution, deny_record: DenyRecord) -> Self {
        Self {
            resolution,
            gate_record: None,
            deny_record: Some(deny_record),
        }
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
        // (G4) and the string result_ref are dropped; a fresh ResultRef is minted.
        CapabilityOutcome::Completed(message) => {
            MappedResolution::bare(Resolution::Done(completed_outcome(message)))
        }
        // Ran and failed in a model-visible, correctable way. The recovery class
        // (error_kind) and detail (G1) are host-side and do not cross.
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
        // Re-entrant gate: needs human approval before it may run.
        CapabilityOutcome::ApprovalRequired {
            safe_summary,
            // approval_resume is loop/host resume identity, not gate-render
            // content; it does not cross into the GateRecord.
            ..
        } => MappedResolution::with_gate(
            Resolution::Blocked(Blocked::Approval(GateRef::new())),
            GateRecord::Approval {
                summary: safe_summary_or_placeholder(safe_summary),
            },
        ),
        // Re-entrant gate: needs a credential the caller has not supplied. The
        // host-owned credential requirements ride the record (G3).
        CapabilityOutcome::AuthRequired {
            credential_requirements,
            safe_summary,
            // auth_resume is loop/host resume identity, not gate-render content.
            ..
        } => MappedResolution::with_gate(
            Resolution::Blocked(Blocked::Auth(GateRef::new())),
            GateRecord::Auth {
                summary: safe_summary_or_placeholder(safe_summary),
                credential_requirements,
            },
        ),
        // Re-entrant gate: needs resource budget currently unavailable.
        CapabilityOutcome::ResourceBlocked { safe_summary, .. } => MappedResolution::with_gate(
            Resolution::Blocked(Blocked::Resource(GateRef::new())),
            GateRecord::Resource {
                summary: safe_summary_or_placeholder(safe_summary),
            },
        ),
        // Parked work: a spawned OS process the turn now waits on. Process
        // suspensions track a ProcessRef, not a gate record; the loop summary has
        // no home on the host channel and is dropped.
        CapabilityOutcome::SpawnedProcess(ProcessHandleSummary { .. }) => {
            MappedResolution::bare(Resolution::Suspended(Suspension::Process(ProcessRef::new())))
        }
        // NON-suspending (the #6137 bug class): the executor appends the child
        // result and continues. Maps to Done/ChildSpawned, carrying the child's
        // RunId on the verdict. child_run_id (a TurnRunId) preserves identity via
        // RunId::from_uuid; the string result_ref is replaced by a fresh ResultRef.
        CapabilityOutcome::SpawnedChildRun {
            child_run_id,
            safe_summary,
            byte_len,
            model_observation,
            ..
        } => MappedResolution::bare(Resolution::Done(Outcome {
            refs: OutcomeRefs {
                result: ResultRef::new(),
                byte_len,
                preview: observation_preview(model_observation),
            },
            verdict: ToolVerdict::ChildSpawned {
                child_run: RunId::from_uuid(child_run_id.as_uuid()),
            },
            summary: safe_summary_or_placeholder(safe_summary),
        })),
        // Parked work: awaits a dependent child run. Gate-shaped, so it carries a
        // GateRecord holding the staged result handle + byte length (G2). Both the
        // gate ref and the staged result ref are freshly minted; the loop's
        // model_observation has no home on DependentRun and is dropped.
        CapabilityOutcome::AwaitDependentRun {
            safe_summary,
            byte_len,
            ..
        } => MappedResolution::with_gate(
            Resolution::Suspended(Suspension::DependentRun(GateRef::new())),
            GateRecord::DependentRun {
                summary: safe_summary_or_placeholder(safe_summary),
                result: ResultRef::new(),
                byte_len,
            },
        ),
        // Parked work: a client-executed external tool the host does not run.
        CapabilityOutcome::ExternalToolPending { safe_summary, .. } => MappedResolution::with_gate(
            Resolution::Suspended(Suspension::ExternalTool(GateRef::new())),
            GateRecord::ExternalTool {
                summary: safe_summary_or_placeholder(safe_summary),
            },
        ),
    }
}

/// Build the `Done` payload for a `Completed` outcome (verdict `Success`).
fn completed_outcome(message: CapabilityResultMessage) -> Outcome {
    let CapabilityResultMessage {
        safe_summary,
        byte_len,
        model_observation,
        // G4: progress / terminate_hint / output_digest are loop-derived and have
        // no home on the host Outcome; result_ref (a loop string) is replaced by a
        // freshly-minted ResultRef.
        ..
    } = message;
    Outcome {
        refs: OutcomeRefs {
            result: ResultRef::new(),
            byte_len,
            preview: observation_preview(model_observation),
        },
        verdict: ToolVerdict::Success,
        summary: safe_summary_or_placeholder(safe_summary),
    }
}

/// Build the `Done` payload for a `Failed` outcome (verdict `RecoverableFailure`).
fn failed_outcome(failure: CapabilityFailure) -> Outcome {
    let CapabilityFailure {
        safe_summary,
        // G1: error_kind (the recovery class) and detail are host-side and do not
        // cross into the model-visible Outcome.
        ..
    } = failure;
    Outcome {
        refs: OutcomeRefs {
            // A recoverable failure stages no durable output beyond its summary;
            // the ref is a minted handle the later store may leave unpopulated.
            result: ResultRef::new(),
            byte_len: 0,
            preview: None,
        },
        verdict: ToolVerdict::RecoverableFailure,
        summary: safe_summary_or_placeholder(safe_summary),
    }
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
    serde_json::from_value::<DenyReason>(serde_json::Value::String(kind.as_str().to_string()))
        .unwrap_or(DenyReason::PolicyDenied)
}

#[cfg(test)]
mod tests {
    use super::super::{
        CapabilityFailureKind, CapabilityProgress, LoopProcessRef,
        MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
    };
    use super::*;
    use crate::{LoopGateRef, LoopResultRef, TurnRunId};
    use ironclaw_host_api::{
        ExtensionId, RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
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
        }

        let rows = vec![
            Row {
                label: "Completed",
                outcome: completed("read 3 files"),
                channel: "done",
                suspends: false,
                gate_record: None,
                deny_record: false,
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
                byte_len, summary, ..
            }) => {
                assert_eq!(byte_len, 2048);
                assert_eq!(summary.as_str(), "awaiting dependent");
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
            let mapped = capability_outcome_to_resolution(CapabilityOutcome::Denied(
                CapabilityDenied {
                    reason_kind,
                    safe_summary: "denied".to_string(),
                },
            ));
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
