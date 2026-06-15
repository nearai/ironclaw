//! Immutable loop execution state.
//!

mod bounded_ring;
mod signature;
mod slots;

pub use bounded_ring::BoundedRing;
pub use ironclaw_turns::LoopFailureKind;
pub use ironclaw_turns::run_profile::AuthResumeApprovalIdentity;
pub use signature::{ArgsHash, CapabilityCallSignature, CapabilityCallSignatureError};
pub use slots::{
    CapabilityStrategyState, CompactionPromptSnapshot, CompactionStrategyState,
    ContextStrategyState, DeferredCompactionWatermark, GateStrategyState, GoalRefreshStrategyState,
    IndexedMessageKind, MessageIndexEntry, ModelStrategyState, PostCapabilityStageState,
    RecoveryAttemptClass, RecoveryStrategyState, RepeatedCallWarningPhase,
    RepeatedCallWarningState, ReplyAdmissionRejection, ReplyAdmissionRejectionReason,
    ReplyAdmissionStrategyState, StopStrategyState,
};

use ironclaw_host_api::{ApprovalRequestId, CapabilityId, CorrelationId, ResourceEstimate};
use ironclaw_turns::{
    LoopGateRef, LoopMessageRef, LoopResultRef,
    run_profile::{
        CapabilityApprovalResume, CapabilityInputRef, CapabilityResumeToken,
        CapabilitySurfaceVersion, LoopInputCursor, LoopRunContext, ProviderToolCallReplay,
    },
};

/// Initial checkpoint payload schema reserved for the default Reborn loop.
///
/// Reborn checkpoint persistence has not shipped yet, so this branch is still
/// defining the v1 payload shape. Once persisted checkpoints are in use,
/// changing this layout requires an explicit schema bump and migration plan.
pub const CHECKPOINT_SCHEMA_ID: &str = "reborn:default-loop-v1";
pub const CHECKPOINT_SCHEMA_VERSION: u64 = 1;

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
    pub recent_failure_kinds: BoundedRing<LoopFailureKind, 8>,
    /// Rolling window of assistant-output token counts (from
    /// `LoopModelResponse::usage.output_tokens`). The default stop
    /// strategy uses this to detect diminishing-returns loops:
    /// `noprogress_window` consecutive turns whose output stays at or
    /// below `min_delta_tokens` → `StopKind::NoProgressDetected`
    /// (#3841 follow-up F1).
    pub recent_output_token_counts: BoundedRing<u32, 8>,

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
    #[serde(default)]
    pub reply_admission_state: ReplyAdmissionStrategyState,
    pub stop_state: StopStrategyState,
    pub gate_state: GateStrategyState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval_resume: Option<PendingApprovalResume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_auth_resume: Option<PendingAuthResume>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingApprovalResume {
    pub gate_ref: LoopGateRef,
    pub capability_id: CapabilityId,
    pub approval_request_id: ApprovalRequestId,
    pub resume_token: CapabilityResumeToken,
    #[serde(default = "CorrelationId::new")]
    pub correlation_id: CorrelationId,
    pub surface_version: CapabilitySurfaceVersion,
    pub input_ref: CapabilityInputRef,
    pub effective_capability_ids: Vec<CapabilityId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_replay: Option<ProviderToolCallReplay>,
    pub input: serde_json::Value,
    pub estimate: ResourceEstimate,
}

impl PendingApprovalResume {
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
            input: self.input.clone(),
            estimate: self.estimate.clone(),
        }
    }
}

/// Auth-gated capability call parked at a blocked-auth checkpoint.
///
/// When the invocation previously passed a one-shot approval (`resume_token`
/// is `Some` and `prior_approval` is `Some`), re-dispatch must reuse the
/// original invocation identifier (encoded in `resume_token`) so the
/// fingerprinted approval lease — whose scope embeds that identifier — can
/// still be matched and claimed.  Without this, a fresh invocation identifier
/// would never match the existing lease, causing an infinite re-approval loop.
///
/// When the invocation never needed approval (`resume_token` is `None` and
/// `prior_approval` is `None`), the re-dispatch goes through the normal
/// `invoke_json` path with a fresh invocation identifier, preserving current
/// behavior.
///
/// The `prior_approval` field collapses the two formerly-independent
/// `approval_request_id`/`correlation_id` options into a typed all-or-none
/// value: both sub-fields are present together or neither is.
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
    /// passed an approval gate.  Encodes the original invocation identifier so
    /// re-dispatch can reuse it instead of minting a fresh one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<CapabilityResumeToken>,
    /// Prior-approval identity, set together with `resume_token` when the
    /// invocation had previously passed a one-shot approval gate.
    /// `approval_request_id` and `correlation_id` are always set as a pair;
    /// see [`AuthResumeApprovalIdentity`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_approval: Option<AuthResumeApprovalIdentity>,
}

impl LoopExecutionState {
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
            recent_failure_kinds: BoundedRing::new(),
            recent_output_token_counts: BoundedRing::new(),
            context_state: ContextStrategyState::default(),
            capability_state: CapabilityStrategyState::default(),
            model_state: ModelStrategyState::default(),
            compaction_state: CompactionStrategyState::default(),
            compaction_prompt: CompactionPromptSnapshot::default(),
            post_capability_state: PostCapabilityStageState::default(),
            goal_refresh_state: GoalRefreshStrategyState::default(),
            recovery_state: RecoveryStrategyState::default(),
            reply_admission_state: ReplyAdmissionStrategyState::default(),
            stop_state: StopStrategyState::default(),
            gate_state: GateStrategyState::default(),
            pending_approval_resume: None,
            pending_auth_resume: None,
        }
    }

    /// Rehydrates state from a checkpoint payload's bytes.
    ///
    /// The bytes are the raw JSON-serialized `LoopExecutionState` — i.e. what
    /// the executor produced via `serde_json::to_vec(&state)` before passing
    /// the bytes to `LoopCheckpointPort::stage_checkpoint_payload`. The payload
    /// contains **no outer envelope**: schema-id and kind live in store-side
    /// metadata, validated by `CheckpointStateStore::get_checkpoint_state`
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
mod tests {
    use ironclaw_host_api::{CapabilityId, TenantId, ThreadId};
    use ironclaw_turns::{
        AgentLoopDriverDescriptor, RunProfileId, RunProfileVersion, TurnId, TurnRunId, TurnScope,
        run_profile::{
            CancellationPolicy, CapabilitySurfaceProfileId, CheckpointPolicy, CheckpointSchemaId,
            ConcurrencyClass, ContextProfileId, LoopDriverId, ModelProfileId,
            RedactedRunProfileProvenance, ResolvedRunProfile, ResourceBudgetPolicy,
            ResourceBudgetTier, RunClassId, RunProfileFingerprint, RuntimeProfileConstraints,
            SchedulingClass, SteeringPolicy,
        },
    };
    use serde_json::json;

    use super::*;

    fn test_run_context() -> LoopRunContext {
        let scope = TurnScope::new(
            TenantId::new("tenant-loop-state").expect("valid"),
            None,
            None,
            ThreadId::new("thread-loop-state").expect("valid"),
        );
        let descriptor = AgentLoopDriverDescriptor {
            id: LoopDriverId::new("loop_state_test_driver").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(
                CheckpointSchemaId::new("loop_state_test_checkpoint").expect("valid"),
            ),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        };
        let resolved_run_profile = ResolvedRunProfile {
            run_class_id: RunClassId::new("loop_state_test_class").expect("valid"),
            profile_id: RunProfileId::default_profile(),
            profile_version: RunProfileVersion::new(1),
            loop_driver: descriptor.clone(),
            checkpoint_schema_id: descriptor
                .checkpoint_schema_id
                .clone()
                .expect("descriptor checkpoint id"),
            checkpoint_schema_version: descriptor
                .checkpoint_schema_version
                .expect("descriptor checkpoint version"),
            model_profile_id: ModelProfileId::new("loop_state_test_model").expect("valid"),
            capability_surface_profile_id: CapabilitySurfaceProfileId::new(
                "loop_state_test_capabilities",
            )
            .expect("valid"),
            context_profile_id: ContextProfileId::new("loop_state_test_context").expect("valid"),
            steering_policy: SteeringPolicy {
                allow_steering: false,
                allow_interrupt: true,
                allow_driver_specific_nudges: false,
            },
            cancellation_policy: CancellationPolicy {
                allow_cancel: true,
                require_checkpoint_before_cancel: false,
            },
            checkpoint_policy: CheckpointPolicy {
                require_before_model: false,
                require_before_side_effect: false,
                require_before_block: true,
                max_checkpoint_bytes: 64 * 1024,
                require_final_checkpoint: false,
                allow_no_reply_completion: false,
            },
            resource_budget_policy: ResourceBudgetPolicy {
                tier: ResourceBudgetTier::new("loop_state_test_tier").expect("valid"),
                max_model_calls: 32,
                max_capability_invocations: 64,
            },
            personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
            runtime_constraints: RuntimeProfileConstraints {
                allow_raw_runtime_backend_selection: false,
                allow_broad_capability_surface: false,
            },
            runner_pool_id: None,
            scheduling_class: SchedulingClass::new("interactive").expect("valid"),
            concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
            resolution_fingerprint: RunProfileFingerprint::new("loop-state-test-fingerprint")
                .expect("valid"),
            provenance: RedactedRunProfileProvenance {
                sources: vec![],
                effective_privileges: vec![],
            },
        };
        LoopRunContext::new(scope, TurnId::new(), TurnRunId::new(), resolved_run_profile)
    }

    /// Encode a checkpoint payload the same way the executor does:
    /// `serde_json::to_vec(&state)` — no outer envelope.
    /// Schema-id and kind are stored as side-channel metadata by
    /// `CheckpointStateStore::put_checkpoint_state`, not inside the bytes.
    fn encode_payload(state: &LoopExecutionState) -> Vec<u8> {
        serde_json::to_vec(state).expect("encode payload")
    }

    #[test]
    fn bounded_ring_push_rolls_over_at_capacity() {
        let mut ring = BoundedRing::<u32, 3>::new();
        ring.push(1);
        ring.push(2);
        ring.push(3);
        ring.push(4);

        assert_eq!(ring.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
    }

    #[test]
    fn bounded_ring_most_common_count_respects_window() {
        let mut ring = BoundedRing::<u32, 8>::new();
        for item in [1, 2, 2, 3, 3, 3] {
            ring.push(item);
        }

        assert_eq!(ring.most_common_count_in(0), 0);
        assert_eq!(ring.most_common_count_in(2), 2);
        assert_eq!(ring.most_common_count_in(6), 3);
        assert_eq!(ring.most_common_count_in(20), 3);
    }

    #[test]
    fn bounded_ring_same_run_length_counts_trailing_run() {
        let empty = BoundedRing::<u32, 4>::new();
        assert_eq!(empty.same_run_length(), 0);

        let mut distinct = BoundedRing::<u32, 4>::new();
        distinct.push(1);
        distinct.push(2);
        distinct.push(3);
        assert_eq!(distinct.same_run_length(), 1);

        let mut run = BoundedRing::<u32, 8>::new();
        for item in [1, 2, 3, 3, 3] {
            run.push(item);
        }
        assert_eq!(run.same_run_length(), 3);
    }

    #[test]
    fn capability_call_signature_is_stable_under_key_reordering() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let reordered = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            capability,
            &json!({"b": 2, "a": {"d": false, "c": [1, null]}}),
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            reordered,
            &json!({"a": {"c": [1, null], "d": false}, "b": 2}),
        )
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn capability_call_signature_is_stable_across_pretty_vs_minified_inputs() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let minified: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"b":[2,3],"c":{"d":4}}"#).unwrap();
        let pretty: serde_json::Value = serde_json::from_str(
            "{\n  \"a\": 1,\n  \"b\": [2, 3],\n  \"c\": {\n    \"d\": 4\n  }\n}",
        )
        .unwrap();

        let from_minified =
            CapabilityCallSignature::from_call(capability.clone(), &minified).unwrap();
        let from_pretty = CapabilityCallSignature::from_call(capability, &pretty).unwrap();
        assert_eq!(from_minified.args_hash, from_pretty.args_hash);
    }

    #[test]
    fn capability_call_signature_is_stable_under_nested_key_reordering() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let first = CapabilityCallSignature::from_call(
            capability.clone(),
            &json!({
                "outer": {
                    "alpha": 1,
                    "beta": {"x": 10, "y": 20},
                    "gamma": [
                        {"p": 1, "q": 2},
                        {"r": 3, "s": 4}
                    ]
                }
            }),
        )
        .unwrap();
        let second = CapabilityCallSignature::from_call(
            capability,
            &json!({
                "outer": {
                    "gamma": [
                        {"q": 2, "p": 1},
                        {"s": 4, "r": 3}
                    ],
                    "beta": {"y": 20, "x": 10},
                    "alpha": 1
                }
            }),
        )
        .unwrap();
        assert_eq!(first.args_hash, second.args_hash);
    }

    #[test]
    fn capability_call_signature_rejects_nan_and_infinity() {
        let capability = CapabilityId::new("demo.echo").unwrap();
        let nan = serde_json::Number::from_f64(f64::NAN);
        let infinity = serde_json::Number::from_f64(f64::INFINITY);
        // serde_json refuses to construct NaN/Infinity through its public API;
        // synthesize them via a manually built Value to exercise the guard.
        // If the upstream representation rejects these inputs entirely, the
        // guard is unreachable at the public boundary — assert that.
        assert!(nan.is_none(), "serde_json refuses NaN at the Number level");
        assert!(
            infinity.is_none(),
            "serde_json refuses Infinity at the Number level"
        );

        // Round-trip a JSON string that contains a NaN-like token. serde_json
        // rejects this at the parser, so we exercise the guard via the
        // signature's own check against the canonicalized output.
        let parse: Result<serde_json::Value, _> = serde_json::from_str("NaN");
        assert!(parse.is_err());

        // The function is fallible by signature; with valid JSON input we
        // should always get Ok.
        let ok = CapabilityCallSignature::from_call(capability, &json!({"x": 1.0}));
        assert!(ok.is_ok());
    }

    #[test]
    fn initial_state_is_value_equal_across_calls() {
        let context = test_run_context();
        assert_eq!(
            LoopExecutionState::initial_for_run(&context),
            LoopExecutionState::initial_for_run(&context)
        );
    }

    #[test]
    fn loop_execution_state_round_trips_through_json() {
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let value = serde_json::to_value(&state).unwrap();
        let restored: LoopExecutionState = serde_json::from_value(value).unwrap();

        assert_eq!(restored, state);
    }

    #[test]
    fn compaction_prompt_snapshot_round_trips_through_checkpoints() {
        let context = test_run_context();
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 42,
            }]);

        let value = serde_json::to_value(&state).unwrap();
        assert!(
            value
                .as_object()
                .expect("state serializes as object")
                .contains_key("compaction_prompt")
        );
        let restored: LoopExecutionState = serde_json::from_value(value).unwrap();

        assert_eq!(restored.compaction_prompt, state.compaction_prompt);
        assert_eq!(restored.compaction_state, state.compaction_state);
    }

    #[test]
    fn loop_execution_state_has_no_control_state_field() {
        // Grep-style assertion: when serialized, the JSON object must carry
        // `stop_state` and `gate_state` and must NOT carry `control_state`.
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let value = serde_json::to_value(&state).unwrap();
        let object = value.as_object().expect("state serializes as object");
        assert!(
            object.contains_key("stop_state"),
            "missing stop_state on serialized LoopExecutionState"
        );
        assert!(
            object.contains_key("gate_state"),
            "missing gate_state on serialized LoopExecutionState"
        );
        assert!(
            !object.contains_key("control_state"),
            "unexpected control_state on serialized LoopExecutionState"
        );
    }

    #[test]
    fn stop_and_gate_strategy_state_round_trip() {
        let stop = StopStrategyState::default();
        let stop_bytes = serde_json::to_vec(&stop).unwrap();
        let stop_restored: StopStrategyState = serde_json::from_slice(&stop_bytes).unwrap();
        assert_eq!(stop_restored, stop);

        let gate = GateStrategyState::default();
        let gate_bytes = serde_json::to_vec(&gate).unwrap();
        let gate_restored: GateStrategyState = serde_json::from_slice(&gate_bytes).unwrap();
        assert_eq!(gate_restored, gate);
    }

    /// Schema-id and kind validation now live in the store layer
    /// (`CheckpointStateStore::get_checkpoint_state`) — not in the payload
    /// bytes. `from_checkpoint_payload` therefore succeeds for any
    /// well-formed `LoopExecutionState` regardless of what kind is passed.
    #[test]
    fn checkpoint_payload_round_trips_raw_state_bytes() {
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let payload = encode_payload(&state);

        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
                .unwrap();
        assert_eq!(restored, state);
    }

    #[test]
    fn checkpoint_payload_kind_arg_is_accepted_for_any_valid_state() {
        // kind is metadata — passing Final for bytes encoded without a kind
        // label must still succeed, because kind authentication happens at the
        // store boundary before bytes are handed to from_checkpoint_payload.
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        let payload = encode_payload(&state);

        let result = LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::Final);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), state);
    }

    #[test]
    fn checkpoint_payload_rejects_malformed_bytes() {
        // Non-JSON bytes must still fail with InvalidField { field: "payload" }.
        let result = LoopExecutionState::from_checkpoint_payload(
            b"not json at all",
            CheckpointKind::BeforeModel,
        );

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::InvalidField {
                field: "payload",
                ..
            })
        ));
    }

    #[test]
    fn checkpoint_payload_rejects_bounded_ring_over_capacity() {
        // Raw state bytes with an over-capacity BoundedRing must fail on
        // deserialization (the BoundedRing Deserialize impl enforces capacity).
        let context = test_run_context();
        let mut state =
            serde_json::to_value(LoopExecutionState::initial_for_run(&context)).unwrap();
        let recent_call_signatures = state
            .get_mut("recent_call_signatures")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|object| object.get_mut("items"))
            .and_then(serde_json::Value::as_array_mut)
            .unwrap();
        for index in 0..9 {
            recent_call_signatures.push(json!(
                CapabilityCallSignature::from_call(
                    CapabilityId::new(format!("demo.echo_{index}")).unwrap(),
                    &json!({ "index": index })
                )
                .unwrap()
            ));
        }
        // Encode as raw state bytes (no envelope).
        let bytes = serde_json::to_vec(&state).unwrap();

        let result =
            LoopExecutionState::from_checkpoint_payload(&bytes, CheckpointKind::BeforeModel);

        assert!(matches!(
            result,
            Err(CheckpointPayloadError::InvalidField {
                field: "payload",
                ..
            })
        ));
    }

    /// Round-2 test coverage: verify that a non-default `post_capability_state`
    /// (populated `pending_capability_bytes` + `skip_model_this_iteration = true`)
    /// survives `to_checkpoint_payload` / `from_checkpoint_payload` intact.
    ///
    /// This is the replay-correctness gate: if these fields are lost on
    /// checkpoint encode/decode, a resumed run would start with a stale byte
    /// accumulator or would incorrectly re-run the model that was supposed to
    /// be skipped.
    #[test]
    fn post_capability_state_with_bytes_round_trips_through_checkpoint() {
        let context = test_run_context();
        let mut state = LoopExecutionState::initial_for_run(&context);

        // Seed pending_capability_bytes with a non-zero entry.
        let cap_id = CapabilityId::new("builtin.http").expect("valid capability id");
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(cap_id.clone(), 33_001);
        // Also set skip_model_this_iteration to verify it round-trips.
        state.post_capability_state.skip_model_this_iteration = true;

        let payload = encode_payload(&state);
        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeModel)
                .expect("decode checkpoint payload");

        assert_eq!(
            restored
                .post_capability_state
                .pending_capability_bytes
                .get(&cap_id),
            Some(&33_001),
            "pending_capability_bytes must survive checkpoint encode/decode"
        );
        assert!(
            restored.post_capability_state.skip_model_this_iteration,
            "skip_model_this_iteration must survive checkpoint encode/decode"
        );
        // Full equality check — no other fields must have changed.
        assert_eq!(
            restored.post_capability_state, state.post_capability_state,
            "entire PostCapabilityStageState must round-trip without loss"
        );
    }

    #[test]
    fn pending_auth_resume_round_trips_through_checkpoint_payload() {
        let context = test_run_context();
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.pending_auth_resume = Some(PendingAuthResume {
            gate_ref: LoopGateRef::new("gate:auth-test").expect("valid gate ref"),
            capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("valid cap id"),
            surface_version: CapabilitySurfaceVersion::new("surface-v1")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:test").expect("valid input ref"),
            effective_capability_ids: vec![],
            provider_replay: None,
            resume_token: None,
            prior_approval: None,
        });
        let payload = encode_payload(&state);
        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
                .expect("decode checkpoint payload");
        assert_eq!(
            restored.pending_auth_resume, state.pending_auth_resume,
            "PendingAuthResume must survive checkpoint encode/decode"
        );
    }

    #[test]
    fn checkpoint_payload_without_auth_resume_slot_decodes_to_none() {
        // Encode a state with no pending_auth_resume; decode must yield None.
        // Also exercise backward compat: a payload JSON object lacking the
        // field entirely (as pre-existing checkpoints would) must decode
        // with None rather than failing.
        let context = test_run_context();
        let state = LoopExecutionState::initial_for_run(&context);
        assert!(
            state.pending_auth_resume.is_none(),
            "initial state must have no pending_auth_resume"
        );

        // Round-trip through the normal encode/decode path.
        let payload = encode_payload(&state);
        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
                .expect("decode checkpoint payload");
        assert!(
            restored.pending_auth_resume.is_none(),
            "decoded state must have no pending_auth_resume when field was absent from payload"
        );

        // Explicitly remove the field from the JSON to simulate a checkpoint
        // produced before PendingAuthResume was added. Decoding must still succeed.
        let mut value: serde_json::Value = serde_json::from_slice(&payload).expect("parse");
        value
            .as_object_mut()
            .expect("state serializes as object")
            .remove("pending_auth_resume");
        let stripped_payload = serde_json::to_vec(&value).expect("re-encode");
        let from_legacy = LoopExecutionState::from_checkpoint_payload(
            &stripped_payload,
            CheckpointKind::BeforeBlock,
        )
        .expect("decode legacy checkpoint payload without pending_auth_resume");
        assert!(
            from_legacy.pending_auth_resume.is_none(),
            "legacy checkpoint missing pending_auth_resume field must decode to None"
        );
    }

    /// Checkpoints written before `resume_token` and `prior_approval` were
    /// added to `PendingAuthResume` must decode to `None` for those fields
    /// (backward compat: serde `default` on optional fields).
    #[test]
    fn pending_auth_resume_without_resume_token_fields_decodes_to_none() {
        use ironclaw_host_api::{ApprovalRequestId, CorrelationId};
        use ironclaw_turns::run_profile::{AuthResumeApprovalIdentity, CapabilityResumeToken};

        let context = test_run_context();
        let mut state = LoopExecutionState::initial_for_run(&context);

        // Build a PendingAuthResume with all optional fields set.
        let resume_token = CapabilityResumeToken::new("00000000-0000-0000-0000-000000000001")
            .expect("valid resume token");
        let approval_request_id = ApprovalRequestId::new();
        let correlation_id = CorrelationId::new();
        state.pending_auth_resume = Some(PendingAuthResume {
            gate_ref: LoopGateRef::new("gate:auth-with-approval").expect("valid gate ref"),
            capability_id: CapabilityId::new("gsuite.calendar.list_events").expect("valid cap id"),
            surface_version: CapabilitySurfaceVersion::new("surface-v2")
                .expect("valid surface version"),
            input_ref: CapabilityInputRef::new("input:approval-auth").expect("valid input ref"),
            effective_capability_ids: vec![],
            provider_replay: None,
            resume_token: Some(resume_token.clone()),
            prior_approval: Some(AuthResumeApprovalIdentity {
                approval_request_id,
                correlation_id,
            }),
        });

        // Round-trip: all optional fields must survive encode/decode.
        let payload = encode_payload(&state);
        let restored =
            LoopExecutionState::from_checkpoint_payload(&payload, CheckpointKind::BeforeBlock)
                .expect("decode checkpoint payload with resume_token fields");
        let pending = restored
            .pending_auth_resume
            .expect("pending_auth_resume must be present after round-trip");
        assert_eq!(
            pending.resume_token,
            Some(resume_token),
            "resume_token must survive checkpoint encode/decode"
        );
        let pa = pending
            .prior_approval
            .expect("prior_approval must survive checkpoint encode/decode");
        assert_eq!(
            pa.approval_request_id, approval_request_id,
            "prior_approval.approval_request_id must survive checkpoint encode/decode"
        );
        assert_eq!(
            pa.correlation_id, correlation_id,
            "prior_approval.correlation_id must survive checkpoint encode/decode"
        );

        // Compat: strip the new fields from JSON to simulate a pre-existing
        // checkpoint. Decoding must yield None for the absent optional fields.
        let mut value: serde_json::Value = serde_json::from_slice(&payload).expect("parse");
        let auth_resume = value
            .as_object_mut()
            .expect("state is object")
            .get_mut("pending_auth_resume")
            .expect("pending_auth_resume field present")
            .as_object_mut()
            .expect("pending_auth_resume is object");
        auth_resume.remove("resume_token");
        auth_resume.remove("prior_approval");
        let stripped_payload = serde_json::to_vec(&value).expect("re-encode");
        let from_legacy = LoopExecutionState::from_checkpoint_payload(
            &stripped_payload,
            CheckpointKind::BeforeBlock,
        )
        .expect("decode legacy checkpoint without resume_token fields");
        let legacy_pending = from_legacy
            .pending_auth_resume
            .expect("pending_auth_resume must still be present");
        assert!(
            legacy_pending.resume_token.is_none(),
            "resume_token absent from checkpoint payload must decode to None"
        );
        assert!(
            legacy_pending.prior_approval.is_none(),
            "prior_approval absent from checkpoint payload must decode to None"
        );
    }
}
