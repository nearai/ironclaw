use ironclaw_host_api::turn::{LoopGateRef, LoopResultRef};
use ironclaw_host_api::{
    resolution::{DependentRunResult, Outcome},
    result_meta::{LoopRef, ResultProgress},
};
use ironclaw_loop_contracts::{
    CapabilityProgress, CapabilityResultMessage, ContentDigest, LoopProcessRef,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation, ObservationTrust,
    ToolObservationDetail, ToolObservationStatus,
};

use super::AgentLoopExecutorError;

// ---------------------------------------------------------------------------
// host_api::Resolution -> loop vocabulary reconstruction (§5.3 Stage 2 flip).
//
// The loop-facing result IS `Resolution` now; these total helpers reconstruct
// the loop-side values the existing downstream stages consume, from the
// channel's preserved `origin` refs and PR-B model-visible content. The
// producer always populates `origin` (the mapping preserves it), so a missing
// one is an internal contract violation, not a recoverable model error.
// ---------------------------------------------------------------------------

fn loop_result_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopResultRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopResultRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop result origin",
        })
}

pub(super) fn loop_gate_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopGateRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopGateRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop gate origin",
        })
}

pub(super) fn loop_process_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopProcessRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopProcessRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop process origin",
        })
}

fn capability_progress_from(progress: ResultProgress) -> CapabilityProgress {
    match progress {
        ResultProgress::Unknown => CapabilityProgress::Unknown,
        ResultProgress::MadeProgress => CapabilityProgress::MadeProgress,
        ResultProgress::NoChange => CapabilityProgress::NoChange,
        ResultProgress::Blocked => CapabilityProgress::Blocked,
    }
}

pub(super) fn capability_result_from_outcome(
    outcome: &Outcome,
) -> Result<CapabilityResultMessage, AgentLoopExecutorError> {
    Ok(CapabilityResultMessage {
        result_ref: loop_result_ref_from_origin(outcome.refs.origin.as_ref())?,
        safe_summary: outcome.summary.as_str().to_string(),
        progress: capability_progress_from(outcome.progress),
        terminate_hint: outcome.terminate_hint.should_terminate(),
        byte_len: outcome.refs.byte_len,
        model_observation: result_reference_observation_from_outcome(outcome),
        output_digest: outcome
            .refs
            .output_digest
            .map(|digest| ContentDigest(digest.value())),
    })
}

pub(super) struct ChildResultAppendInput {
    pub(super) result_ref: LoopResultRef,
    pub(super) safe_summary: String,
    pub(super) byte_len: u64,
    pub(super) model_observation: Option<ModelVisibleToolObservation>,
}

pub(super) fn child_result_from_outcome(
    outcome: &Outcome,
) -> Result<ChildResultAppendInput, AgentLoopExecutorError> {
    Ok(ChildResultAppendInput {
        result_ref: loop_result_ref_from_origin(outcome.refs.origin.as_ref())?,
        safe_summary: outcome.summary.as_str().to_string(),
        byte_len: outcome.refs.byte_len,
        model_observation: result_reference_observation_from_outcome(outcome),
    })
}

/// Rebuild the `ResultReference` model observation from a completed [`Outcome`],
/// carrying the #5838 first-look inline preview content the model reads without a
/// follow-up `result_read`. Reconstructed from the channel's real
/// [`ModelResultPreview`] (`refs.preview`) and its independent continuation
/// metadata. Metadata-only observations are reconstructed when preview safety
/// suppresses the text; `None` is reserved for outcomes with neither preview nor
/// continuation metadata, where `append_capability_result_ref` synthesizes a bare
/// success observation.
fn result_reference_observation_from_outcome(
    outcome: &Outcome,
) -> Option<ModelVisibleToolObservation> {
    let preview = outcome.refs.preview.as_ref();
    let meta = &outcome.refs.preview_meta;
    if preview.is_none() && meta.is_empty() {
        return None;
    }
    // The observation references the preview's OWN result: `preview_meta`'s
    // referenced ref when it differs (a `result_read` presenting another result),
    // else the outcome's own preserved origin.
    let result_ref = meta
        .referenced_result_ref
        .as_ref()
        .or(outcome.refs.origin.as_ref())?
        .as_str()
        .to_string();
    Some(ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        // The observation's OWN producer-authored summary (carried through the
        // collapse in `preview_meta`), NOT the generic outcome caption: it holds
        // the truncation/continuation hint ("preview truncated, use result_read …")
        // that a completed result message's `safe_summary` ("capability completed")
        // does not. Falls back to the outcome caption when the producer authored no
        // observation summary (or it failed the caption contract).
        summary: meta
            .summary
            .as_ref()
            .map(|summary| summary.as_str().to_string())
            .unwrap_or_else(|| outcome.summary.as_str().to_string()),
        detail: ToolObservationDetail::ResultReference {
            result_ref,
            byte_len: outcome.refs.byte_len,
            preview: preview.map(|preview| preview.as_str().to_string()),
            structured_json_view: meta.structured_json_view,
            // Continuation metadata for a truncated first-look preview; falls back
            // to the full inline size for a complete preview.
            total_bytes: meta.total_bytes.or(Some(outcome.refs.byte_len)),
            next_offset: meta.next_offset,
            item_count: meta.item_count,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    })
}

/// Rebuild the staged dependent-child result the parent observes on resume from
/// the inline [`DependentRunResult`] (Stage 1b) — no host-storage read.
pub(super) fn dependent_run_result_message(
    result: &DependentRunResult,
) -> Result<CapabilityResultMessage, AgentLoopExecutorError> {
    let result_ref = loop_result_ref_from_origin(result.origin.as_ref())?;
    // Forward the child's staged observation caption (#6287 IronLoop). The
    // mapping preserves a bounded `SafeSummary` caption on
    // `DependentRunResult.observation` — "model_observation now rides the inline
    // observation preview (was dropped entirely)". Hardcoding `None` here re-drops
    // it, so `append_capability_result_ref` falls back to a bare synthesized
    // success observation and the resumed parent loses both the caption and the
    // staged result reference. Surface it as a `ResultReference` observation
    // pointing at the staged child result. The full inline first-look preview
    // content stays host-owned and is the completed-`Outcome` path, not this
    // suspension channel.
    let model_observation =
        result
            .observation
            .as_ref()
            .map(|caption| ModelVisibleToolObservation {
                schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
                status: ToolObservationStatus::Success,
                summary: caption.as_str().to_string(),
                detail: ToolObservationDetail::ResultReference {
                    result_ref: result_ref.as_str().to_string(),
                    byte_len: result.byte_len,
                    preview: None,
                    structured_json_view: false,
                    total_bytes: None,
                    next_offset: None,
                    item_count: None,
                },
                artifacts: Vec::new(),
                recovery: None,
                trust: ObservationTrust::UntrustedToolOutput,
            });
    Ok(CapabilityResultMessage {
        result_ref,
        safe_summary: result.summary.as_str().to_string(),
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: result.byte_len,
        output_digest: None,
        model_observation,
    })
}
