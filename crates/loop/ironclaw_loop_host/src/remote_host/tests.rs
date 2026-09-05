use super::protocol::*;
use super::server::dispatch_host_call;
use ironclaw_host_api::resolution::{Resolution, ResolutionBatch};
use ironclaw_host_api::turn::RunProfileId;
use ironclaw_host_api::turn::{TurnCheckpointId, TurnScope};
use ironclaw_loop_contracts::ResolvedRunProfile;
use ironclaw_loop_contracts::*;
use ironclaw_turns::LoopMessageRef;

#[test]
fn loop_worker_wire_rejects_oversized_frames_before_transport() {
    let outcome = LoopWorkerOutcome::Failed(LoopWorkerFailure {
        kind: "oversized".to_string(),
        detail: Some("x".repeat(LOOP_WORKER_MAX_FRAME_BYTES)),
    });
    let error = encode(&WorkerFrame::Outcome(outcome)).expect_err("oversized frame must fail");
    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[test]
fn private_surface_wire_round_trips_host_assigned_runtime_kind() {
    let descriptor = CapabilityDescriptorView {
        capability_id: ironclaw_host_api::ids::CapabilityId::new("builtin.shell")
            .expect("capability id"),
        provider: None,
        runtime: ironclaw_host_api::runtime::RuntimeKind::FirstParty,
        safe_name: "builtin.shell".to_string(),
        safe_description: "Run a command".to_string(),
        description_trust: Default::default(),
        parameters_schema: serde_json::json!({"type": "object"}),
    };
    let wire = WireVisibleCapabilitySurface::from(VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        descriptors: vec![descriptor],
        callable_capability_ids: None,
    });
    let encoded = serde_json::to_vec(&wire).expect("surface serializes");
    let decoded: WireVisibleCapabilitySurface =
        serde_json::from_slice(&encoded).expect("trusted surface deserializes");
    let restored = VisibleCapabilitySurface::from(decoded);
    assert_eq!(
        restored.descriptors[0].runtime,
        ironclaw_host_api::runtime::RuntimeKind::FirstParty
    );
}

#[test]
fn private_context_wire_preserves_the_complete_empty_bundle_shape() {
    let original = LoopContextBundle::default();
    let wire = WireLoopContextBundle::from(original.clone());
    let encoded = serde_json::to_vec(&wire).expect("wire context serializes");
    let decoded: WireLoopContextBundle =
        serde_json::from_slice(&encoded).expect("wire context deserializes");
    assert_eq!(LoopContextBundle::from(decoded), original);
}

#[test]
fn private_checkpoint_wire_revalidates_redacted_payload_bytes() {
    let original = LoadedCheckpointPayload {
        kind: LoopCheckpointKind::BeforeModel,
        schema_id: CheckpointSchemaId::new("canonical-loop-state").expect("schema id"),
        schema_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
        payload: RedactedCheckpointPayload::new(br#"{"iteration":1}"#.to_vec())
            .expect("bounded payload"),
    };
    let wire = WireLoadedCheckpointPayload::from(original.clone());
    let encoded = serde_json::to_vec(&wire).expect("wire checkpoint serializes");
    let decoded: WireLoadedCheckpointPayload =
        serde_json::from_slice(&encoded).expect("wire checkpoint deserializes");
    let restored = LoadedCheckpointPayload::try_from(decoded).expect("payload revalidates");
    assert_eq!(restored, original);
}

#[derive(Debug, Default)]
struct RecordingContentPort {
    requested_refs: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl LoopMessageContentPort for RecordingContentPort {
    async fn resolve_message_content(
        &self,
        messages: Vec<LoopModelMessage>,
    ) -> Result<Vec<ResolvedModelMessage>, AgentLoopHostError> {
        if let Ok(mut refs) = self.requested_refs.lock() {
            refs.extend(
                messages
                    .iter()
                    .map(|message| message.content_ref.as_str().to_string()),
            );
        }
        Ok(messages
            .into_iter()
            .map(|message| ResolvedModelMessage {
                role: message.role,
                content_ref: message.content_ref,
                content: "resolved transcript text".to_string(),
                tool_result: Some(ResolvedToolResult {
                    provider_call_id: Some("call_1".to_string()),
                    content: "tool result text".to_string(),
                }),
            })
            .collect())
    }
}

/// Minimal driver host: never called by the ResolveMessages dispatch path.
#[derive(Default)]
struct NoopDriverHost;

impl LoopRunInfoPort for NoopDriverHost {
    fn run_context(&self) -> &LoopRunContext {
        unreachable!("run_context is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopContextPort for NoopDriverHost {
    async fn load_loop_context(
        &self,
        _request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        unreachable!("load_loop_context is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopPromptPort for NoopDriverHost {
    async fn build_prompt_bundle(
        &self,
        _request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        Ok(LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::new(
                "prompt:01950000-0000-7000-8000-000000000001:test",
            )
            .expect("prompt ref"),
            messages: vec![
                loop_message("one", "user"),
                loop_message("two", "assistant"),
                loop_message("budget", "user"),
            ],
            surface_version: None,
            compaction_message_index: Vec::new(),
            recent_window_truncation: None,
            instruction_fingerprint: None,
            identity_message_count: 0,
            instruction_snippet_count: 0,
        })
    }
}

#[async_trait::async_trait]
impl LoopInputPort for NoopDriverHost {
    async fn poll_inputs(
        &self,
        _after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        unreachable!("poll_inputs is never called by ResolveMessages dispatch")
    }

    async fn ack_inputs(&self, _tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        unreachable!("ack_inputs is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopModelPort for NoopDriverHost {
    async fn stream_model(
        &self,
        _request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        unreachable!("stream_model is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopCapabilityPort for NoopDriverHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        unreachable!("visible_capabilities is never called by ResolveMessages dispatch")
    }

    async fn invoke_capability(
        &self,
        _request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        unreachable!("invoke_capability is never called by ResolveMessages dispatch")
    }

    async fn invoke_capability_batch(
        &self,
        _request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        unreachable!("invoke_capability_batch is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopTranscriptPort for NoopDriverHost {
    async fn finalize_assistant_message(
        &self,
        _request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        unreachable!("finalize_assistant_message is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopCheckpointPort for NoopDriverHost {
    async fn checkpoint(
        &self,
        _request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        unreachable!("checkpoint is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopProgressPort for NoopDriverHost {
    async fn emit_loop_progress(
        &self,
        _event: LoopProgressEvent,
    ) -> Result<(), AgentLoopHostError> {
        unreachable!("emit_loop_progress is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopCompactionPort for NoopDriverHost {
    async fn compact_loop_context(
        &self,
        _request: LoopCompactionRequest,
    ) -> Result<LoopCompactionOutcome, LoopCompactionError> {
        unreachable!("compact_loop_context is never called by ResolveMessages dispatch")
    }
}

#[async_trait::async_trait]
impl LoopCancellationPort for NoopDriverHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }

    async fn cancellation_requested(&self) -> LoopCancellationSignal {
        std::future::pending().await
    }
}

fn test_noop_host() -> NoopDriverHost {
    NoopDriverHost
}

fn test_rpc_state() -> super::server::HostRpcState {
    super::server::tests::rpc_state(4, 4)
}

fn loop_message(ref_suffix: &str, role: &str) -> LoopModelMessage {
    LoopModelMessage {
        role: role.to_string(),
        content_ref: ironclaw_turns::LoopMessageRef::new(format!("msg:{ref_suffix}"))
            .expect("message ref"),
    }
}

fn wire_error_kind(error: WireError) -> AgentLoopHostErrorKind {
    match error {
        WireError::Host(error) => error.kind,
        other => panic!("expected a host error, got {other:?}"),
    }
}

async fn issue_prompt(state: &mut super::server::HostRpcState) {
    dispatch_host_call(
        &test_noop_host(),
        None,
        WorkerContentVisibility::Resolved,
        HostCall::BuildPrompt(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            capability_view: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
        }),
        state,
    )
    .await
    .expect("issue prompt through the host");
}

#[tokio::test]
async fn blind_bootstrap_denies_resolve_messages_without_calling_the_port() {
    let port = RecordingContentPort {
        requested_refs: std::sync::Mutex::new(Vec::new()),
    };
    let mut state = test_rpc_state();
    let call = HostCall::ResolveMessages(ResolveMessagesRequest {
        messages: vec![loop_message("denied", "user")],
    });

    let error = dispatch_host_call(
        &test_noop_host(),
        Some(&port),
        WorkerContentVisibility::Blind,
        call,
        &mut state,
    )
    .await
    .expect_err("blind worker must be denied");

    assert_eq!(wire_error_kind(error), AgentLoopHostErrorKind::PolicyDenied);
    assert!(port.requested_refs.lock().expect("refs").is_empty());
}

#[tokio::test]
async fn resolved_bootstrap_returns_content_for_exactly_the_requested_refs() {
    let port = RecordingContentPort {
        requested_refs: std::sync::Mutex::new(Vec::new()),
    };
    let mut state = test_rpc_state();
    issue_prompt(&mut state).await;
    let call = HostCall::ResolveMessages(ResolveMessagesRequest {
        messages: vec![
            loop_message("one", "user"),
            loop_message("two", "assistant"),
        ],
    });

    let value = dispatch_host_call(
        &test_noop_host(),
        Some(&port),
        WorkerContentVisibility::Resolved,
        call,
        &mut state,
    )
    .await
    .expect("resolved worker may resolve content");

    let resolved: Vec<WireResolvedModelMessage> =
        serde_json::from_value(value).expect("wire shape");
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0].content, "resolved transcript text");
    assert_eq!(
        resolved[0]
            .tool_result
            .as_ref()
            .expect("tool result")
            .content,
        "tool result text"
    );
    assert_eq!(
        resolved[0]
            .tool_result
            .as_ref()
            .expect("tool result")
            .provider_call_id
            .as_deref(),
        Some("call_1")
    );
    assert_eq!(
        port.requested_refs.lock().expect("refs").as_slice(),
        ["msg:one", "msg:two"]
    );
}

#[test]
fn v1_shaped_bootstrap_without_content_visibility_decodes_blind() {
    let bootstrap = LoopWorkerBootstrap {
        wire_version: LOOP_WORKER_WIRE_VERSION,
        run_context: LoopRunContext::new(
            TurnScope::new(
                ironclaw_host_api::ids::TenantId::new("tenant").expect("tenant"),
                None,
                None,
                ironclaw_host_api::ids::ThreadId::new("thread").expect("thread"),
            ),
            ironclaw_host_api::turn::TurnId::new(),
            ironclaw_host_api::turn::TurnRunId::new(),
            ResolvedRunProfile::legacy_compatibility(
                RunProfileId::interactive_default(),
                ironclaw_host_api::turn::RunProfileVersion::new(1),
                true,
            ),
        ),
        invocation: LoopWorkerInvocation::Run(AgentLoopDriverRunRequest {
            turn_id: ironclaw_host_api::turn::TurnId::new(),
            run_id: ironclaw_host_api::turn::TurnRunId::new(),
            resolved_run_profile: ResolvedRunProfile::legacy_compatibility(
                RunProfileId::interactive_default(),
                ironclaw_host_api::turn::RunProfileVersion::new(1),
                true,
            ),
        }),
        settings: LoopWorkerSettings::default(),
        tool_definitions: Vec::new(),
        current_visible_capabilities: None,
        content_visibility: WorkerContentVisibility::Resolved,
    };
    let encoded = serde_json::to_vec(&bootstrap).expect("bootstrap serializes");
    let mut value: serde_json::Value = serde_json::from_slice(&encoded).expect("json");
    assert_eq!(value["wire_version"], 2);
    // Simulate a v1 worker encoding: no content_visibility field at all.
    let removed = value
        .as_object_mut()
        .expect("object")
        .remove("content_visibility");
    assert!(removed.is_some(), "v2 bootstrap must carry the field");

    let decoded: LoopWorkerBootstrap = serde_json::from_value(value).expect("v1 shape decodes");
    assert_eq!(decoded.wire_version, 2);
    assert_eq!(decoded.content_visibility, WorkerContentVisibility::Blind);
}

#[tokio::test]
async fn resolve_messages_counts_against_the_rpc_budget() {
    let port = RecordingContentPort {
        requested_refs: std::sync::Mutex::new(Vec::new()),
    };
    let mut state = super::server::tests::rpc_state(1, 1);
    issue_prompt(&mut state).await;
    let make_call = || {
        HostCall::ResolveMessages(ResolveMessagesRequest {
            messages: vec![loop_message("budget", "user")],
        })
    };

    dispatch_host_call(
        &test_noop_host(),
        Some(&port),
        WorkerContentVisibility::Resolved,
        make_call(),
        &mut state,
    )
    .await
    .expect("first resolve is admitted");
    dispatch_host_call(
        &test_noop_host(),
        Some(&port),
        WorkerContentVisibility::Resolved,
        make_call(),
        &mut state,
    )
    .await
    .expect("second resolve is admitted");

    let error = wire_error_kind(
        dispatch_host_call(
            &test_noop_host(),
            Some(&port),
            WorkerContentVisibility::Resolved,
            make_call(),
            &mut state,
        )
        .await
        .expect_err("third resolve exceeds the budget"),
    );
    assert_eq!(error, AgentLoopHostErrorKind::BudgetExceeded);
}

#[tokio::test]
async fn resolved_worker_cannot_expand_issued_reference_authority() {
    let port = RecordingContentPort {
        requested_refs: std::sync::Mutex::new(Vec::new()),
    };
    let mut state = test_rpc_state();
    issue_prompt(&mut state).await;
    for messages in [
        Vec::new(),
        vec![loop_message("guessed-same-thread", "user")],
        vec![loop_message("foreign-thread", "user")],
        vec![loop_message("one", "system")],
    ] {
        let error = dispatch_host_call(
            &test_noop_host(),
            Some(&port),
            WorkerContentVisibility::Resolved,
            HostCall::ResolveMessages(ResolveMessagesRequest { messages }),
            &mut state,
        )
        .await
        .expect_err("unissued references must not reach content storage");
        assert!(matches!(
            wire_error_kind(error),
            AgentLoopHostErrorKind::PolicyDenied | AgentLoopHostErrorKind::InvalidInvocation
        ));
    }
    assert!(port.requested_refs.lock().expect("refs").is_empty());
}
