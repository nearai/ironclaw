//! Transitional `CapabilityOutcome` → `Resolution` adapter (arch-simplification
//! §5.3 Stage 2b — being deleted).
//!
//! The non-lossy redaction now lives in the producer-facing constructors in
//! [`super::resolution`]; this module is a thin delegator kept ONLY so producers
//! not yet migrated to the constructors keep compiling through the collapse. It
//! mints no refs and carries no side-table: [`RefBindings`] is always empty (the
//! flip routes loop-ref recovery via the channel's preserved `origin`), and the
//! sibling records are re-collected from the constructor results. Once every
//! producer emits a `Resolution` directly, this file and `MappedResolution`/
//! `RefBindings` are deleted.

use ironclaw_host_api::{DenyRecord, GateRecord, Resolution};

use super::host::CapabilityOutcome;
use super::resolution;

/// A [`Resolution`] plus the side records its opaque refs render from (§5.2.9).
///
/// Retained transitionally for callers of [`capability_outcome_to_resolution`].
/// `bindings` is always empty; loop-ref recovery rides the channel `origin`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedResolution {
    pub resolution: Resolution,
    pub gate_record: Option<GateRecord>,
    pub deny_record: Option<DenyRecord>,
    pub bindings: RefBindings,
}

/// Retained transitionally; always empty (the flip preserves loop refs on the
/// channel `origin`, not this side-table). Deleted with `resolution_mapping`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefBindings;

/// Map one loop-facing [`CapabilityOutcome`] onto its host_api [`Resolution`]
/// channel plus any side record, delegating to the producer-facing constructors
/// in [`super::resolution`].
pub fn capability_outcome_to_resolution(outcome: CapabilityOutcome) -> MappedResolution {
    let bare = |resolution: Resolution| MappedResolution {
        resolution,
        gate_record: None,
        deny_record: None,
        bindings: RefBindings,
    };
    match outcome {
        CapabilityOutcome::Completed(message) => bare(resolution::completed(
            message.result_ref,
            message.safe_summary,
            message.progress,
            message.terminate_hint,
            message.byte_len,
            message.output_digest,
            message.model_observation,
        )),
        CapabilityOutcome::Failed(failure) => bare(resolution::failed(
            failure.error_kind,
            failure.safe_summary,
            failure.detail,
        )),
        CapabilityOutcome::SpawnedProcess(process) => {
            bare(resolution::spawned_process(process.process_ref))
        }
        CapabilityOutcome::SpawnedChildRun {
            child_run_id,
            result_ref,
            safe_summary,
            byte_len,
            model_observation,
        } => bare(resolution::spawned_child_run(
            child_run_id,
            result_ref,
            safe_summary,
            byte_len,
            model_observation,
        )),
        CapabilityOutcome::Denied(denied) => {
            let denied = resolution::denied(denied.reason_kind, denied.safe_summary);
            MappedResolution {
                resolution: denied.resolution,
                gate_record: None,
                deny_record: Some(denied.deny_record),
                bindings: RefBindings,
            }
        }
        CapabilityOutcome::ApprovalRequired {
            gate_ref,
            safe_summary,
            approval_resume,
        } => gated(resolution::approval_required(
            gate_ref,
            safe_summary,
            approval_resume,
        )),
        CapabilityOutcome::AuthRequired {
            gate_ref,
            credential_requirements,
            safe_summary,
            auth_resume,
        } => gated(resolution::auth_required(
            gate_ref,
            credential_requirements,
            safe_summary,
            auth_resume,
        )),
        CapabilityOutcome::ResourceBlocked {
            gate_ref,
            safe_summary,
        } => gated(resolution::resource_blocked(gate_ref, safe_summary)),
        CapabilityOutcome::AwaitDependentRun {
            gate_ref,
            result_ref,
            safe_summary,
            byte_len,
            model_observation,
        } => gated(resolution::await_dependent_run(
            gate_ref,
            result_ref,
            safe_summary,
            byte_len,
            model_observation,
        )),
        CapabilityOutcome::ExternalToolPending {
            gate_ref,
            safe_summary,
        } => gated(resolution::external_tool_pending(gate_ref, safe_summary)),
    }
}

fn gated(gated: resolution::GatedResolution) -> MappedResolution {
    MappedResolution {
        resolution: gated.resolution,
        gate_record: gated.gate_record,
        deny_record: None,
        bindings: RefBindings,
    }
}
