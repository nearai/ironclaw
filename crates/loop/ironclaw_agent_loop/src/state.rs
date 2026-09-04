//! Immutable loop execution state.
//!

mod bounded_ring;
mod budget_ledger;
mod compaction;
mod model_recovery;
mod recovery;
mod reply_admission;
mod signature;
mod slots;
mod stop_control;
mod terminal_warning;

pub use bounded_ring::BoundedRing;
pub use budget_ledger::BudgetLedger;
pub(crate) use budget_ledger::{BudgetCharge, InvocationCharge};
pub use compaction::{
    CompactionEffectivenessBaseline, CompactionPromptSnapshot, CompactionStrategyState,
    DeferredCompactionWatermark, IndexedMessageKind, MessageIndexEntry,
};
pub use ironclaw_loop_contracts::AuthResumeApprovalIdentity;
pub use ironclaw_loop_contracts::LoopFailureKind;
pub use model_recovery::{ModelErrorRecoveryObservation, PendingModelRetryDirective};
pub use recovery::{ModelErrorObservationClass, RecoveryAttemptClass, RecoveryStrategyState};
pub use reply_admission::{
    ReplyAdmissionRejection, ReplyAdmissionRejectionReason, ReplyAdmissionStrategyState,
};
pub use signature::{
    ArgsHash, CapabilityCallSignature, CapabilityCallSignatureError, CapabilityOutputObservation,
};
pub use slots::{
    CapabilityStrategyState, ContextStrategyState, GateStrategyState, GoalRefreshStrategyState,
    ModelStrategyState, PostCapabilityStageState,
};
pub use stop_control::{RepeatedCallWarningPhase, RepeatedCallWarningState, StopStrategyState};
pub(crate) use terminal_warning::TerminalWarningObservation;
pub use terminal_warning::TerminalWarningState;

use ironclaw_host_api::ids::{ApprovalRequestId, CapabilityId, CorrelationId};
use ironclaw_host_api::turn::CapabilityActivityId;
use ironclaw_host_api::turn::{LoopGateRef, LoopMessageRef, LoopResultRef};
use ironclaw_loop_contracts::{
    CapabilityApprovalResume, CapabilityInputRef, CapabilityResumeToken, CapabilitySurfaceVersion,
    LoopInputCursor, LoopRunContext, ProviderToolCallReplay,
};

/// Checkpoint payload schema for the default Reborn loop.
///
/// Required parked-activity ids are part of the v2 payload shape. Older v1
/// checkpoints are intentionally not migrated by this refactor.
pub const CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v2";
pub const CHECKPOINT_SCHEMA_VERSION: u64 = 2;

/// Immutable execution state threaded through the loop.
///
/// The executor rebinds its local `let mut state` each tick to the next whole
/// state. Strategies receive `&LoopExecutionState` and return outcome enums
/// that carry the new value of their own slot. The executor builds the next
/// whole state by swapping that slot.
///
/// Stop and Gate each own their own slot — there is no shared `control_state`
/// — so a family's future growth in either dimension can't accidentally mix
/// concerns through a shared struct.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoopExecutionState {
    // executor-universal
    pub iteration: u32,
    pub last_checkpoint: Option<CheckpointMarker>,
    pub assistant_refs: Vec<LoopMessageRef>,
    pub result_refs: Vec<LoopResultRef>,
    pub last_gate: Option<LoopGateRef>,
    pub input_cursor: LoopInputCursor,
    pub surface_version: Option<CapabilitySurfaceVersion>,

    // executor-observed (populated by executor; read-only to strategies)
    pub recent_call_signatures: BoundedRing<CapabilityCallSignature, 8>,
    /// (signature, output_digest) trail for completed calls whose result
    /// carried a digest. Populated by `append_completed_capability_result`
    /// (executor/capability_outcomes.rs); read by
    /// `DefaultStopConditionStrategy::should_stop_after_observed_turn`
    /// (strategies/stop.rs) to detect a call whose OUTPUT repeats, not just
    /// its signature. `#[serde(default)]` for rolling-upgrade/rollback: a
    /// legacy checkpoint with no ring decodes to empty; the guard is inert
    /// until repopulated by fresh calls.
    #[serde(default)]
    pub seen_capability_output_digests: BoundedRing<CapabilityOutputObservation, 64>,
    pub recent_failure_kinds: BoundedRing<LoopFailureKind, 8>,
    /// Provider-reported assistant-output token counts retained in checkpoint
    /// payloads for compatibility. No default stop decision reads this ring.
    pub recent_output_token_counts: BoundedRing<u32, 8>,

    /// Cumulative provider-reported token usage across this run's model calls,
    /// summed from `LoopModelResponse::usage`. Carried into the terminal
    /// `LoopExit` so the run record persists per-run usage for the
    /// OpenAI-compatible surfaces. `None` until the first call that reports
    /// usage (replay stubs and usage-less providers leave it `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cumulative_model_usage: Option<ironclaw_loop_contracts::LoopModelUsage>,

    /// Per-run budget accounting chokepoint: wall-clock start plus the
    /// model-call and capability-invocation counters. Flattened so the
    /// checkpoint wire shape is unchanged (same top-level field names,
    /// same defaults) from when these were three bare fields on this
    /// struct — see `budget_ledger.rs` and the frozen-shape test in
    /// `state/tests/checkpoint_wire.rs`.
    #[serde(flatten)]
    pub budget_ledger: BudgetLedger,

    /// Count of tools-capable completion nudges issued this run (driver-specific
    /// nudge, gated by `SteeringPolicy.allow_driver_specific_nudges`). It
    /// re-enters the loop with the full tool surface so the model can finish the
    /// task (e.g. write a required output file) before answering. Capped so the
    /// loop can't issue unbounded extra iterations. `#[serde(default)]` keeps
    /// older checkpoints decodable.
    #[serde(default)]
    pub completion_nudges_used: u32,

    /// Set when the executor decided to issue a tools-capable completion nudge
    /// on the previous turn; consumed by the next prompt build, which injects the
    /// completion-nudge control message and clears this flag. `#[serde(default)]`
    /// keeps older checkpoints decodable.
    #[serde(default)]
    pub completion_nudge_pending: bool,

    /// One-shot, typed host-authored repair context for the next model call
    /// after a model-error retry budget is exhausted. It remains checkpointed
    /// until the executor has issued the request, so restart cannot lose it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_model_error_observation: Option<ModelErrorRecoveryObservation>,

    /// Prompt-shape repair directive for the next model call. Kept separately
    /// from backoff/compaction retry mechanics because it must be reconstructed
    /// from the retry-transition checkpoint after worker restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_model_retry_directive: Option<PendingModelRetryDirective>,

    /// One-shot warning state for otherwise-terminal no-progress and iteration
    /// limit conditions. Pending prompt context and the consumed budget remain
    /// checkpointed until the executor issues the recovery model request.
    #[serde(default)]
    pub terminal_warning_state: TerminalWarningState,

    /// Whether the most recent admitted assistant reply "trailed off" without a
    /// real closing answer (empty after trim, or ends with a colon — a narrated
    /// next step with no follow-through). Populated by `AssistantReplyStage`; read
    /// by the stop handling to decide whether a graceful stop warrants a
    /// completion nudge. `#[serde(default)]` keeps older checkpoints decodable.
    #[serde(default)]
    pub last_reply_trailed_off: bool,

    /// Whether the most recent admitted assistant reply was empty after
    /// trimming. Kept separately from `last_reply_trailed_off` so unattended
    /// runs can fail empty output after their bounded nudge budget without
    /// changing the existing trailing-colon terminal behavior.
    #[serde(default)]
    pub last_reply_empty: bool,

    /// Whether the most recent admitted assistant reply's trimmed final line
    /// ended in a question mark. Scheduled runs cannot obtain an answer from a
    /// user, so this drives their origin-scoped completion recovery only.
    #[serde(default)]
    pub last_reply_ended_with_question: bool,

    // strategy slots — one per strategy that mutates state.
    pub context_state: ContextStrategyState,
    pub capability_state: CapabilityStrategyState,
    pub model_state: ModelStrategyState,
    #[serde(default)]
    pub compaction_state: CompactionStrategyState,
    #[serde(default)]
    pub compaction_prompt: CompactionPromptSnapshot,
    #[serde(default)]
    pub post_capability_state: PostCapabilityStageState,
    #[serde(default)]
    pub goal_refresh_state: GoalRefreshStrategyState,
    pub recovery_state: RecoveryStrategyState,
    /// Monotonic identity source for durable recovery evidence.
    ///
    /// The next recovery append uses this value, then advances it only after
    /// the host accepts the event. Because the counter is checkpointed with
    /// the recovery state, replay after an append/checkpoint interruption
    /// reuses the same logical event identity instead of minting a second
    /// recovery numerator.
    #[serde(default)]
    pub recovery_event_sequence: u64,
    #[serde(default)]
    pub reply_admission_state: ReplyAdmissionStrategyState,
    pub stop_state: StopStrategyState,
    pub gate_state: GateStrategyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval_resume: Option<PendingApprovalResume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_auth_resume: Option<PendingAuthResume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_external_tool_resume: Option<PendingExternalToolResume>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingApprovalResume {
    pub gate_ref: LoopGateRef,
    pub capability_id: CapabilityId,
    pub approval_request_id: ApprovalRequestId,
    pub resume_token: CapabilityResumeToken,
    /// Activity identifier for the parked invocation. Resume handling keys the
    /// parked UI row by this explicit id, not by capability id or token shape.
    pub activity_id: CapabilityActivityId,
    #[serde(default = "CorrelationId::new")]
    pub correlation_id: CorrelationId,
    pub surface_version: CapabilitySurfaceVersion,
    pub input_ref: CapabilityInputRef,
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
    /// Set when the user denied this approval gate. The loop surfaces a
    /// model-visible failure for the parked call instead of re-dispatching.
    /// See the field-name note on `PendingAuthResume::disposition`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<ironclaw_host_api::turn::GateResumeDisposition>,
}

impl PendingApprovalResume {
    pub(crate) fn activity_id_for_resume(&self) -> CapabilityActivityId {
        self.activity_id
    }

    /// Converts this pending resume into the neutral wire DTO used by the
    /// capability port.  Centralising the field-by-field mapping here removes
    /// the two manual conversion sites in the executor and ensures any new
    /// fields are propagated consistently.
    pub(crate) fn to_approval_resume(&self) -> CapabilityApprovalResume {
        CapabilityApprovalResume {
            approval_request_id: self.approval_request_id,
            resume_token: self.resume_token.clone(),
            correlation_id: self.correlation_id,
            input_ref: self.input_ref.clone(),
        }
    }
}

/// Auth-gated capability call parked at a blocked-auth checkpoint.
///
/// Auth re-dispatch reuses the original invocation identifier when a
/// `resume_token` is available, so any fingerprinted approval lease whose scope
/// embeds that identifier can still be matched and claimed. The runtime input a
/// re-dispatch needs (staged input refs may be consumed by the first dispatch or
/// scoped to a prior loop run) is no longer checkpointed here: the host persists
/// it in the host-private replay-payload store at the fresh gate raise and
/// reconstitutes it on resume, keyed by the invocation id in `resume_token`
/// (arch-simplification §5.3 Stage 2a-i).
///
/// The `prior_approval` field collapses the two formerly-independent
/// `approval_request_id`/`correlation_id` options into a typed all-or-none
/// value: both sub-fields are present together or neither is.
///
/// When `disposition` is `Some(Denied)`, the executor surfaces a model-visible
/// gate-declined failure for the parked call and SKIPS re-dispatch; in that
/// case `resume_token` is unused.
///
/// Field-name note: each pending-resume type scopes `disposition` to ONE
/// parked gate (auth or approval), so the short name is unambiguous within
/// the struct.  Turn-layer records that are gate-agnostic use the fuller
/// `resume_disposition` to distinguish the field from other disposition-like
/// values in a wider context.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingAuthResume {
    pub gate_ref: LoopGateRef,
    pub capability_id: CapabilityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
    /// Original invocation resume token, set when the invocation previously
    /// reached an auth gate.  Encodes the original invocation identifier so
    /// re-dispatch can reuse it instead of minting a fresh one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<CapabilityResumeToken>,
    /// Activity identifier for the parked invocation. Token-less auth gates
    /// carry this explicitly so a later denial finalizes the same activity
    /// instead of leaving the UI row running.
    pub activity_id: CapabilityActivityId,
    /// Prior-approval identity, set together with `resume_token` when the
    /// invocation had previously passed a one-shot approval gate.
    /// `approval_request_id` and `correlation_id` are always set as a pair;
    /// see [`AuthResumeApprovalIdentity`].
    ///
    /// Raw runtime input no longer rides the checkpoint: the host persists it in
    /// the host-private replay-payload store at the fresh gate raise and
    /// reconstitutes it on resume, keyed by the invocation id in `resume_token`
    /// (arch-simplification §5.3 Stage 2a-i). This removes the charter-violating
    /// raw-tool-args-in-state exposure (see this crate's `CLAUDE.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_approval: Option<AuthResumeApprovalIdentity>,
    /// Set when the user denied this auth gate. The loop surfaces a
    /// model-visible failure for the parked call instead of re-dispatching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<ironclaw_host_api::turn::GateResumeDisposition>,
}

impl PendingAuthResume {
    pub(crate) fn activity_id_for_resume(&self) -> CapabilityActivityId {
        self.activity_id
    }
}

/// Client-supplied ("external") tool call parked at a `BlockedExternalTool`
/// checkpoint. Unlike auth/approval, external-tool resume carries no resume
/// token: the run is re-dispatched as a plain invocation and the host's
/// external-tool decorator completes it from the run-scoped catalog (which holds
/// the client-submitted output keyed by provider call id). The `provider_replay`
/// is re-registered on resume so the decorator re-binds `input_ref -> call_id`
/// and the model's tool arguments are re-staged.
///
/// When `disposition` is `Some(Denied)`, the executor surfaces a model-visible
/// failure for the parked call and SKIPS re-dispatch (so a cancelled external
/// tool cannot re-block forever).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingExternalToolResume {
    pub gate_ref: LoopGateRef,
    pub capability_id: CapabilityId,
    pub activity_id: CapabilityActivityId,
    pub surface_version: CapabilitySurfaceVersion,
    pub input_ref: CapabilityInputRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<ironclaw_host_api::turn::GateResumeDisposition>,
}

impl PendingExternalToolResume {
    pub(crate) fn activity_id_for_resume(&self) -> CapabilityActivityId {
        self.activity_id
    }
}

impl LoopExecutionState {
    /// Accumulate one model call's reported usage into the run's cumulative
    /// total. No-op when the call reported no usage (replay stubs, usage-less
    /// providers), leaving any prior total intact.
    pub(crate) fn accumulate_model_usage(
        &mut self,
        usage: Option<ironclaw_loop_contracts::LoopModelUsage>,
    ) {
        if let Some(usage) = usage {
            self.cumulative_model_usage
                .get_or_insert_with(Default::default)
                .add_assign(&usage);
        }
    }

    /// Builds the initial state at the start of a fresh run.
    ///
    /// The `input_cursor` field is populated via
    /// [`LoopInputCursor::origin_for_run`], which binds the cursor to the
    /// active run's `(scope, run_id)`. Callers must therefore hold a valid
    /// [`LoopRunContext`] at the start of every run — there is no
    /// `Default`-shaped constructor because every cursor must name a run.
    pub fn initial_for_run(context: &LoopRunContext) -> Self {
        Self {
            iteration: 0,
            last_checkpoint: None,
            assistant_refs: Vec::new(),
            result_refs: Vec::new(),
            last_gate: None,
            input_cursor: LoopInputCursor::origin_for_run(context),
            surface_version: None,
            recent_call_signatures: BoundedRing::new(),
            seen_capability_output_digests: BoundedRing::new(),
            recent_failure_kinds: BoundedRing::new(),
            recent_output_token_counts: BoundedRing::new(),
            cumulative_model_usage: None,
            budget_ledger: BudgetLedger::fresh_for_run(),
            completion_nudges_used: 0,
            completion_nudge_pending: false,
            pending_model_error_observation: None,
            pending_model_retry_directive: None,
            terminal_warning_state: TerminalWarningState::default(),
            last_reply_trailed_off: false,
            last_reply_empty: false,
            last_reply_ended_with_question: false,
            context_state: ContextStrategyState::default(),
            capability_state: CapabilityStrategyState::default(),
            model_state: ModelStrategyState::default(),
            compaction_state: CompactionStrategyState::default(),
            compaction_prompt: CompactionPromptSnapshot::default(),
            post_capability_state: PostCapabilityStageState::default(),
            goal_refresh_state: GoalRefreshStrategyState::default(),
            recovery_state: RecoveryStrategyState::default(),
            recovery_event_sequence: 0,
            reply_admission_state: ReplyAdmissionStrategyState::default(),
            stop_state: StopStrategyState::default(),
            gate_state: GateStrategyState::default(),
            pending_approval_resume: None,
            pending_auth_resume: None,
            pending_external_tool_resume: None,
        }
    }

    /// Rehydrates state from a checkpoint payload's bytes.
    ///
    /// The bytes are the raw JSON-serialized `LoopExecutionState` — i.e. what
    /// the executor produced via `serde_json::to_vec(&state)` before passing
    /// the bytes to `LoopCheckpointPort::stage_checkpoint_payload`. The payload
    /// contains **no outer envelope**: schema-id and kind live in journal
    /// metadata, validated by the process-backed checkpoint projection
    /// before the bytes ever reach this function. The `kind` argument is
    /// accepted for API symmetry (the call site can document what boundary the
    /// checkpoint belongs to) but is not used to authenticate the bytes.
    pub fn from_checkpoint_payload(
        payload: &[u8],
        _kind: CheckpointKind,
    ) -> Result<Self, CheckpointPayloadError> {
        serde_json::from_slice(payload).map_err(|error| CheckpointPayloadError::InvalidField {
            field: "payload",
            reason: error.to_string(),
        })
    }

    /// Rebinds run-owned host state after loading a checkpoint into a new retry
    /// run.
    ///
    /// Retryable failed runs intentionally reuse the source run's checkpoint
    /// payload. The input cursor inside that payload is scoped to the source
    /// `(scope, run_id)`, so it cannot be submitted to the retry host. Durable
    /// transcript/result refs in the payload are also owned by the source run;
    /// carrying them into the retry would make the retry's terminal exit claim
    /// foreign-run evidence. Reset these run-owned fields and let the retry host
    /// produce its own refs. The repeat-call and output-digest observations
    /// plus terminal warning/control state are likewise run-owned and are
    /// reset below so the retry cannot inherit a no-progress strike.
    ///
    /// Gate-bound resume state (`last_gate`, `pending_approval_resume`,
    /// `pending_auth_resume`) is deliberately NOT cleared here: this same path
    /// (`PlannedDriver::resume` -> `from_checkpoint_payload().rebase_for_run()`)
    /// is what resumes a run after an approval/auth gate is resolved, and the
    /// pending-resume record is exactly the evidence that tells the loop to
    /// re-dispatch the gated capability. Clearing it drops the resumed
    /// invocation (regression: only the pre-gate call runs). The resume host
    /// re-validates the gate before honoring the record, so this is not a
    /// trust-boundary leak.
    pub fn rebase_for_run(mut self, context: &LoopRunContext) -> Self {
        if self.input_cursor.is_for_run(context) {
            return self;
        }
        self.input_cursor = LoopInputCursor::origin_for_run(context);
        self.assistant_refs.clear();
        self.result_refs.clear();
        // A retry rebases onto a different run id; the failed run's token total
        // belongs to that run and must not be re-reported under the new one.
        // (Same-run gate resumes return early above, preserving the total.)
        self.cumulative_model_usage = None;
        // `budget_ledger` holds per-run budget accounting (see its doc
        // comments and `ResourceBudgetPolicy` in the budget stage). It
        // belongs to the source run, not the retry: carrying an exhausted
        // counter or a stale wall-clock start across to a fresh `TurnRunId`
        // would make the retry fail its budget stage immediately, before it
        // does any work. Reset it here so the retry starts its own budget
        // window. (Same-run gate resumes return early above, preserving it.)
        self.budget_ledger = BudgetLedger::fresh_for_run();
        // Repeat-call and no-progress observations plus terminal warnings are
        // also scoped to the source run. Carrying any of them across a retry
        // would let the new run inherit a prior strike and terminate without
        // earning it.
        self.recent_call_signatures = BoundedRing::new();
        self.seen_capability_output_digests = BoundedRing::new();
        self.terminal_warning_state = TerminalWarningState::default();
        self.stop_state.repeated_call_warning = None;
        self.stop_state.trailing_no_progress_results = 0;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CheckpointMarker {
    pub kind: CheckpointKind,
    pub iteration_at_checkpoint: u32,
}

/// Mirrors the four checkpoint boundaries from the executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    BeforeModel,
    BeforeSideEffect,
    BeforeBlock,
    Final,
}

impl CheckpointKind {
    /// Whether resuming from a checkpoint of this kind would re-execute an
    /// external side effect.
    ///
    /// Mirrors `ironclaw_processes::ProcessCheckpointKind::replays_side_effect`,
    /// which is what the scheduler's lease-expiry recovery reads off the
    /// process row: a run whose newest checkpoint replays a side effect is
    /// FAILED rather than requeued, because no durable tool-idempotency table
    /// exists to prove the effect did not land. Deliberately duplicated rather
    /// than imported — `ironclaw_agent_loop` does not depend on the process
    /// kernel, and the two enums are separate contracts that happen to agree.
    /// Fail-closed: only the kinds proven safe answer `false`.
    pub fn replays_side_effect(self) -> bool {
        match self {
            Self::BeforeModel | Self::BeforeBlock => false,
            // `Final` is terminal evidence, never a resume point. Treating it
            // as side-effecting keeps this predicate fail-closed if a future
            // caller reaches it.
            Self::BeforeSideEffect | Self::Final => true,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckpointPayloadError {
    #[error("checkpoint payload schema id mismatch: expected `{expected}`, got `{actual}`")]
    SchemaMismatch { expected: String, actual: String },
    #[error("checkpoint payload kind mismatch: expected `{expected:?}`, got `{actual:?}`")]
    KindMismatch {
        expected: CheckpointKind,
        actual: CheckpointKind,
    },
    #[error("checkpoint payload missing required field `{field}`")]
    MissingField { field: &'static str },
    #[error("checkpoint payload field `{field}` failed validation: {reason}")]
    InvalidField { field: &'static str, reason: String },
}

#[cfg(test)]
mod tests;
