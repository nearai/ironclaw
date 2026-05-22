use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_host_api::{
    AgentId, CapabilityId, ProjectId, RuntimeKind, TenantId, ThreadId, UserId,
};
use ironclaw_turns::{
    AcceptedMessageRef, AgentLoopDriver, AgentLoopDriverDescriptor, AgentLoopDriverError,
    DefaultTurnCoordinator, IdempotencyKey, InMemoryTurnStateStore, LoopBlocked, LoopBlockedKind,
    LoopCompleted, LoopCompletionKind, LoopExit, LoopExitId, LoopGateRef, LoopMessageRef,
    ReplyTargetBindingRef, RunProfileRequest, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnCoordinator,
    TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnStatus,
    events::EventCursor,
    run_profile::{
        AgentLoopDriverHost, AgentLoopHostError, AgentLoopHostErrorKind, AssistantReply,
        BatchPolicyKind, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
        CapabilityDeniedReasonKind, CapabilityDescriptorView, CapabilityInputRef,
        CapabilityInvocation, CapabilityOutcome, CapabilitySurfaceVersion, ConcurrencyHint,
        FinalizeAssistantMessage, HostManagedLoopModelPort, HostManagedLoopPromptPort,
        InMemoryInstructionMaterializationStore, InMemoryLoopHostMilestoneSink,
        InstructionBundleBuilder, InstructionBundleFingerprint, InstructionBundleRequest,
        InstructionMaterializationStore, InstructionSafetyContext, LoopCancellationPort,
        LoopCancellationSignal, LoopCapabilityPort, LoopCheckpointKind, LoopCheckpointPort,
        LoopCheckpointRequest, LoopCheckpointStateRef, LoopContextBundle, LoopContextMessage,
        LoopContextPort, LoopContextRequest, LoopContextSnippet, LoopContextSnippetMetadata,
        LoopDriverId, LoopDriverNoteKind, LoopGateKind, LoopHostMilestone,
        LoopHostMilestoneEmitter, LoopHostMilestoneKind, LoopHostMilestoneSink, LoopInputAckToken,
        LoopInputBatch, LoopInputCursor, LoopInputCursorToken, LoopInputPort,
        LoopModelBudgetAccountant, LoopModelCapabilityView, LoopModelGateway,
        LoopModelGatewayError, LoopModelGatewayRequest, LoopModelMessage, LoopModelPolicyGuard,
        LoopModelPort, LoopModelRequest, LoopModelResponse, LoopProgressEvent, LoopProgressPort,
        LoopPromptBundle, LoopPromptBundleAuthority, LoopPromptBundleRef, LoopPromptBundleRequest,
        LoopPromptPort, LoopRunContext, LoopRunInfoPort, LoopTranscriptPort, ModelCallOutcome,
        ParentLoopOutput, PromptMode, PromptSkillContextMetadata, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
    runner::{ClaimRunRequest, TurnRunTransitionPort},
};

#[tokio::test]
async fn two_fake_drivers_use_the_same_per_run_agent_loop_host_contract() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    host.push_model_response(LoopModelResponse {
        chunks: Vec::new(),
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "done".to_string(),
        }),
        effective_model_profile_id: host.context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    });
    host.push_capability_outcome(CapabilityOutcome::ApprovalRequired {
        gate_ref: LoopGateRef::new("gate:approval-needed").unwrap(),
        safe_summary: "approval required".to_string(),
    });

    let reply_exit = ReplyDriver
        .run(driver_run_request(&host), host.as_ref())
        .await
        .unwrap();
    let capability_exit = CapabilityDriver
        .run(driver_run_request(&host), host.as_ref())
        .await
        .unwrap();

    assert!(matches!(reply_exit, LoopExit::Completed(_)));
    assert!(matches!(capability_exit, LoopExit::Blocked(_)));
    assert_eq!(
        host.effects(),
        vec![
            "visible_capabilities",
            "prompt_bundle",
            "context",
            "milestone:prompt_bundle_built",
            "model",
            "milestone:model_started",
            "milestone:model_completed",
            "finalize_assistant",
            "milestone:assistant_reply_finalized",
            "progress:driver_note",
            "visible_capabilities",
            "invoke:demo.echo",
            "checkpoint:before_block",
            "progress:driver_note",
        ]
    );
    assert_eq!(host.run_context().run_id, host.context.run_id);
    assert_eq!(
        host.run_context().thread_id,
        ThreadId::new("thread-loop-host").unwrap()
    );
    assert_eq!(
        host.milestone_kind_names(),
        vec![
            "prompt_bundle_built",
            "model_started",
            "model_completed",
            "assistant_reply_finalized",
        ]
    );
    let milestones = host.milestones();
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::PromptBundleBuilt { .. }
    ));
    assert!(milestones.iter().all(|milestone| {
        milestone.scope == host.context.scope
            && milestone.turn_id == host.context.turn_id
            && milestone.run_id == host.context.run_id
    }));
    let serialized = serde_json::to_string(&milestones).unwrap();
    assert!(!serialized.contains("done"));
    assert!(!serialized.contains("RAW_AGENT_LOOP_HOST_SENTINEL"));
}

#[tokio::test]
async fn host_managed_model_port_routes_gateway_and_emits_model_milestones() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(LoopModelResponse {
        chunks: vec![ironclaw_turns::run_profile::ModelStreamChunk {
            safe_text_delta: "safe delta".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "RAW_ASSISTANT_CONTENT_SENTINEL".to_string(),
        }),
        effective_model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    }));
    let port =
        HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

    let response = port
        .stream_model(LoopModelRequest {
            messages: vec![LoopModelMessage {
                role: "user".to_string(),
                content_ref: LoopMessageRef::new("msg:user-message").unwrap(),
            }],
            surface_version: Some(CapabilitySurfaceVersion::new("surface-v1").unwrap()),
            model_preference: Some(context.resolved_run_profile.model_profile_id.clone()),
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(matches!(
        response.output,
        ParentLoopOutput::AssistantReply(_)
    ));
    let requests = gateway.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].context.run_id, context.run_id);
    assert_eq!(requests[0].context.scope, context.scope);
    assert_eq!(
        milestone_sink
            .milestones()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect::<Vec<_>>(),
        vec!["model_started", "model_completed"]
    );
    let serialized_milestones = serde_json::to_string(&milestone_sink.milestones()).unwrap();
    assert!(!serialized_milestones.contains("RAW_ASSISTANT_CONTENT_SENTINEL"));
}

#[tokio::test]
async fn host_managed_model_port_returns_response_when_model_started_milestone_fails() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(FailingOnModelStartedMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(LoopModelResponse {
        chunks: vec![ironclaw_turns::run_profile::ModelStreamChunk {
            safe_text_delta: "safe delta".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "model response survived start milestone failure".to_string(),
        }),
        effective_model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    }));
    let port =
        HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

    let response = port
        .stream_model(LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();

    let ParentLoopOutput::AssistantReply(reply) = response.output else {
        panic!("expected assistant reply");
    };
    assert_eq!(
        reply.content,
        "model response survived start milestone failure"
    );
    assert_eq!(gateway.requests().len(), 1);
    assert_eq!(milestone_sink.kind_names(), vec!["model_completed"]);
}

#[tokio::test]
async fn host_managed_model_port_returns_response_when_model_completed_milestone_fails() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(FailingOnModelCompletedMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(LoopModelResponse {
        chunks: vec![ironclaw_turns::run_profile::ModelStreamChunk {
            safe_text_delta: "safe delta".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "model response survived milestone failure".to_string(),
        }),
        effective_model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    }));
    let port =
        HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

    let response = port
        .stream_model(LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap();

    let ParentLoopOutput::AssistantReply(reply) = response.output else {
        panic!("expected assistant reply");
    };
    assert_eq!(reply.content, "model response survived milestone failure");
    assert_eq!(gateway.requests().len(), 1);
    assert_eq!(milestone_sink.kind_names(), vec!["model_started"]);
}

#[tokio::test]
async fn host_managed_model_port_sanitizes_gateway_errors() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    assert!(
        LoopModelGatewayError::new(
            AgentLoopHostErrorKind::Unavailable,
            "openai request failed: invalid api key",
        )
        .is_err()
    );
    gateway.push_response(Err(LoopModelGatewayError::new(
        AgentLoopHostErrorKind::Unavailable,
        "model unavailable",
    )
    .unwrap()));
    let port = HostManagedLoopModelPort::new(context.clone(), gateway, milestone_sink.clone());

    let error = port
        .stream_model(LoopModelRequest {
            messages: Vec::new(),
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert_eq!(error.safe_summary, "model unavailable");
    let serialized_error = serde_json::to_string(&error).unwrap();
    assert!(!serialized_error.contains("invalid api key"));
    assert_eq!(
        milestone_sink
            .milestones()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect::<Vec<_>>(),
        vec!["model_started", "model_failed"]
    );
}

#[tokio::test]
async fn instruction_bundle_builder_orders_sections_and_rebuilds_deterministically() {
    let context = claimed_run_context().await;
    let surface = VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        descriptors: vec![CapabilityDescriptorView {
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            provider: None,
            runtime: RuntimeKind::FirstParty,
            safe_name: "Echo".to_string(),
            safe_description: "Echo safe input".to_string(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: serde_json::json!({"type":"object","properties":{"input":{"type":"string"}}}),
        }],
    };
    let request = InstructionBundleRequest {
        context_bundle: LoopContextBundle {
            identity_messages: vec![LoopContextMessage {
                message_ref: Some(LoopMessageRef::new("msg:identity").unwrap()),
                role: "system".to_string(),
                safe_summary: "identity safe".to_string(),
            }],
            messages: vec![LoopContextMessage {
                message_ref: Some(LoopMessageRef::new("msg:user-message").unwrap()),
                role: "user".to_string(),
                safe_summary: "user safe".to_string(),
            }],
            instruction_snippets: vec![
                LoopContextSnippet {
                    snippet_ref: "instruction:project".to_string(),
                    safe_summary: "project rule".to_string(),
                    metadata: None,
                },
                LoopContextSnippet {
                    snippet_ref: "skill:alpha".to_string(),
                    safe_summary: "alpha skill".to_string(),
                    metadata: Some(LoopContextSnippetMetadata {
                        source_name: "alpha".to_string(),
                        trust_level: "trusted".to_string(),
                    }),
                },
                LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "system rule".to_string(),
                    metadata: None,
                },
                LoopContextSnippet {
                    snippet_ref: "instruction:user".to_string(),
                    safe_summary: "user rule".to_string(),
                    metadata: None,
                },
                LoopContextSnippet {
                    snippet_ref: "instruction:agent".to_string(),
                    safe_summary: "agent rule".to_string(),
                    metadata: None,
                },
            ],
            memory_snippets: vec![LoopContextSnippet {
                snippet_ref: "memory:project-summary".to_string(),
                safe_summary: "project memory".to_string(),
                metadata: None,
            }],
        },
        visible_surface: Some(surface),
        safety_context: Some(
            InstructionSafetyContext::new("safety:prompt-write", "prompt write safety enforced")
                .unwrap(),
        ),
    };

    let builder = InstructionBundleBuilder::new(context);
    let first = builder.build(request.clone()).unwrap();
    let second = builder.build(request).unwrap();

    assert_eq!(first.fingerprint, second.fingerprint);
    assert_eq!(first.messages, second.messages);
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.content_ref.as_str().to_string())
            .collect::<Vec<_>>(),
        vec![
            "msg:identity".to_string(),
            first.messages[1].content_ref.as_str().to_string(),
            first.messages[2].content_ref.as_str().to_string(),
            first.messages[3].content_ref.as_str().to_string(),
            first.messages[4].content_ref.as_str().to_string(),
            first.messages[5].content_ref.as_str().to_string(),
            first.messages[6].content_ref.as_str().to_string(),
            first.messages[7].content_ref.as_str().to_string(),
            first.messages[8].content_ref.as_str().to_string(),
            "msg:user-message".to_string(),
        ]
    );
    assert!(
        first.messages[1]
            .content_ref
            .as_str()
            .starts_with("msg:instruction.instruction.system.")
    );
    assert!(
        first.messages[2]
            .content_ref
            .as_str()
            .starts_with("msg:instruction.instruction.user.")
    );
    assert!(
        first.messages[3]
            .content_ref
            .as_str()
            .starts_with("msg:instruction.instruction.agent.")
    );
    assert!(
        first.messages[4]
            .content_ref
            .as_str()
            .starts_with("msg:instruction.instruction.project.")
    );
    assert!(
        first.messages[5]
            .content_ref
            .as_str()
            .starts_with("msg:snippet.skill.alpha.")
    );
    assert!(
        first.messages[6]
            .content_ref
            .as_str()
            .starts_with("msg:memory.memory.project-summary.")
    );
    assert!(
        first.messages[7]
            .content_ref
            .as_str()
            .starts_with("msg:safety.safety.prompt-write.")
    );
    assert!(
        first.messages[8]
            .content_ref
            .as_str()
            .starts_with("msg:surface.surface-v1.")
    );
    assert_eq!(first.skill_context.len(), 1);
    assert_eq!(first.skill_context[0].source_name, "alpha");
}

#[test]
fn instruction_bundle_fingerprint_deserialize_rejects_invalid_values() {
    let error = serde_json::from_value::<InstructionBundleFingerprint>(serde_json::json!(
        "not-a-sha256-fingerprint"
    ))
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("instruction bundle fingerprint must start with sha256:")
    );
}

#[tokio::test]
async fn instruction_bundle_builder_allows_safe_domain_terms_in_summaries() {
    let context = claimed_run_context().await;
    let builder = InstructionBundleBuilder::new(context);

    builder
        .build(InstructionBundleRequest {
            context_bundle: LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: vec![LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "Explain how to rotate a secret without exposing values"
                        .to_string(),
                    metadata: None,
                }],
                memory_snippets: Vec::new(),
            },
            visible_surface: None,
            safety_context: None,
        })
        .unwrap();
}

#[tokio::test]
async fn instruction_bundle_builder_allows_terms_inside_larger_words() {
    let context = claimed_run_context().await;
    let builder = InstructionBundleBuilder::new(context);

    builder
        .build(InstructionBundleRequest {
            context_bundle: LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: vec![LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "Explain preauthorization sync behavior".to_string(),
                    metadata: None,
                }],
                memory_snippets: Vec::new(),
            },
            visible_surface: None,
            safety_context: None,
        })
        .unwrap();
}

#[tokio::test]
async fn instruction_bundle_builder_rejects_secret_credential_phrases() {
    let context = claimed_run_context().await;
    let builder = InstructionBundleBuilder::new(context);

    let error = builder
        .build(InstructionBundleRequest {
            context_bundle: LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: vec![LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "client secret should not appear in prompt context".to_string(),
                    metadata: None,
                }],
                memory_snippets: Vec::new(),
            },
            visible_surface: None,
            safety_context: None,
        })
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
}

#[tokio::test]
async fn instruction_bundle_builder_allows_tool_result_reference_context_messages() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await).with_context_tail_message(
            "tool_result_reference",
            "msg:tool-result-reference",
            "tool result reference safe summary",
        ),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(bundle.messages.iter().any(|message| {
        message.role == "tool_result_reference"
            && message.content_ref.as_str() == "msg:tool-result-reference"
    }));
}

#[tokio::test]
async fn instruction_bundle_serialization_hides_materialized_content() {
    let context = claimed_run_context().await;
    let bundle = InstructionBundleBuilder::new(context)
        .build(InstructionBundleRequest {
            context_bundle: LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: vec![LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "RAW_MATERIALIZED_PROMPT_SENTINEL".to_string(),
                    metadata: None,
                }],
                memory_snippets: Vec::new(),
            },
            visible_surface: None,
            safety_context: None,
        })
        .unwrap();

    assert!(
        bundle
            .materialized_messages
            .iter()
            .any(|message| message.safe_content == "RAW_MATERIALIZED_PROMPT_SENTINEL")
    );
    let wire = serde_json::to_string(&bundle).unwrap();
    assert!(!wire.contains("RAW_MATERIALIZED_PROMPT_SENTINEL"));
    assert!(!wire.contains("materialized_messages"));
}

#[tokio::test]
async fn instruction_bundle_builder_rejects_unsafe_instruction_context() {
    let context = claimed_run_context().await;
    let builder = InstructionBundleBuilder::new(context);

    let error = builder
        .build(InstructionBundleRequest {
            context_bundle: LoopContextBundle {
                identity_messages: Vec::new(),
                messages: Vec::new(),
                instruction_snippets: vec![LoopContextSnippet {
                    snippet_ref: "instruction:system".to_string(),
                    safe_summary: "leaks /Users/alice/.ssh/id_rsa path".to_string(),
                    metadata: None,
                }],
                memory_snippets: Vec::new(),
            },
            visible_surface: None,
            safety_context: None,
        })
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
}

#[tokio::test]
async fn loop_prompt_port_builds_text_only_bundle_from_context_refs() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let surface_version = CapabilitySurfaceVersion::new("surface-v1").unwrap();
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_current_surface_version(surface_version.clone());

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface_version.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(bundle.bundle_ref.is_for_run(&host.context));
    assert_eq!(bundle.surface_version, Some(surface_version));
    assert_eq!(bundle.identity_message_count, 0);
    assert_eq!(bundle.instruction_snippet_count, 0);
    assert_eq!(
        bundle.messages,
        vec![LoopModelMessage {
            role: "user".to_string(),
            content_ref: LoopMessageRef::new("msg:user-message").unwrap(),
        }]
    );
    assert_eq!(host.effects(), vec!["context"]);
    assert_eq!(host.milestone_kind_names(), vec!["prompt_bundle_built"]);
}

#[tokio::test]
async fn loop_prompt_port_filters_visible_surface_by_capability_view() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let surface = VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        descriptors: vec![
            CapabilityDescriptorView {
                capability_id: CapabilityId::new("demo.echo").unwrap(),
                provider: None,
                runtime: RuntimeKind::Wasm,
                safe_name: "Echo".to_string(),
                safe_description: "Returns an opaque result ref".to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: serde_json::json!({"type":"object"}),
            },
            CapabilityDescriptorView {
                capability_id: CapabilityId::new("demo.hidden").unwrap(),
                provider: None,
                runtime: RuntimeKind::Wasm,
                safe_name: "Hidden".to_string(),
                safe_description: "Should not reach the prompt".to_string(),
                concurrency_hint: ConcurrencyHint::Exclusive,
                parameters_schema: serde_json::json!({"type":"object"}),
            },
        ],
    };
    let store = Arc::new(InMemoryInstructionMaterializationStore::default());
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_current_surface(surface.clone())
    .with_instruction_materialization_store(store.clone());

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: Some(LoopModelCapabilityView {
                visible_capability_ids: vec![CapabilityId::new("demo.echo").unwrap()],
            }),
        })
        .await
        .unwrap();
    let surface_message = bundle
        .messages
        .iter()
        .find_map(|message| {
            store
                .get_materialized_message(&host.context, &message.content_ref)
                .unwrap()
                .filter(|materialized| materialized.safe_content.starts_with("surface "))
        })
        .expect("surface instruction materialized");

    assert!(surface_message.safe_content.contains("demo.echo"));
    assert!(!surface_message.safe_content.contains("demo.hidden"));
}

#[tokio::test]
async fn prompt_bundle_authority_consumes_grant_after_successful_model_authorization() {
    let context = claimed_run_context().await;
    let authority = LoopPromptBundleAuthority::default();
    let messages = vec![LoopModelMessage {
        role: "user".to_string(),
        content_ref: LoopMessageRef::new("msg:user-message").unwrap(),
    }];
    let bundle = LoopPromptBundle {
        bundle_ref: LoopPromptBundleRef::for_run(&context, "bundle-once").unwrap(),
        messages: messages.clone(),
        surface_version: None,
        instruction_fingerprint: None,
        identity_message_count: 0,
        instruction_snippet_count: 0,
    };
    authority.issue_bundle(&context, &bundle).unwrap();

    let grant = authority
        .authorize_latest_model_request(&context, &messages, &None)
        .unwrap();
    assert_eq!(grant.messages, messages);

    let error = authority
        .authorize_latest_model_request(&context, &grant.messages, &None)
        .unwrap_err();
    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[tokio::test]
async fn loop_prompt_port_uses_current_surface_version_lookup_each_build() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let surface_v1 = CapabilitySurfaceVersion::new("surface:v1").unwrap();
    let surface_v2 = CapabilitySurfaceVersion::new("surface:v2").unwrap();
    let current_surface = Arc::new(Mutex::new(Some(surface_v1.clone())));
    let current_surface_for_lookup = Arc::clone(&current_surface);
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_current_surface_version_lookup(move || {
        current_surface_for_lookup
            .lock()
            .map(|current| current.clone())
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "surface version lookup is unavailable",
                )
            })
    });

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface_v1.clone()),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(bundle.surface_version, Some(surface_v1.clone()));

    *current_surface.lock().unwrap() = Some(surface_v2);

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface_v1),
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
    assert_eq!(host.effects(), vec!["context"]);
}

#[tokio::test]
async fn loop_milestone_emitter_publishes_loop_progress_kinds() {
    let host = RecordingAgentLoopHost::new(claimed_run_context().await);
    let emitter = host.milestone_emitter();

    emitter.iteration_started(3).await.unwrap();
    emitter
        .capability_batch_started(3, 2, BatchPolicyKind::Parallel)
        .await
        .unwrap();
    emitter
        .capability_batch_completed(3, 1, 0, 1, 0)
        .await
        .unwrap();
    emitter.gate_blocked(3, LoopGateKind::Auth).await.unwrap();

    let milestones = host.milestones();
    assert!(matches!(
        milestones[0].kind,
        LoopHostMilestoneKind::IterationStarted { iteration: 3 }
    ));
    assert!(matches!(
        milestones[1].kind,
        LoopHostMilestoneKind::CapabilityBatchStarted {
            iteration: 3,
            call_count: 2,
            policy: BatchPolicyKind::Parallel,
        }
    ));
    assert!(matches!(
        milestones[2].kind,
        LoopHostMilestoneKind::CapabilityBatchCompleted {
            iteration: 3,
            result_count: 1,
            denied_count: 0,
            gated_count: 1,
            failed_count: 0,
        }
    ));
    assert!(matches!(
        milestones[3].kind,
        LoopHostMilestoneKind::GateBlocked {
            iteration: 3,
            gate_kind: LoopGateKind::Auth,
        }
    ));
}

#[tokio::test]
async fn loop_prompt_port_materializes_instruction_snippets_as_system_refs() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_instruction_snippet("skill:alpha", "alpha skill context available"),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 2);
    assert_eq!(bundle.identity_message_count, 0);
    assert_eq!(bundle.instruction_snippet_count, 1);
    assert_eq!(bundle.messages[0].role, "system");
    assert_eq!(
        bundle.messages[0].content_ref,
        LoopMessageRef::new("msg:snippet.skill.alpha.0.25eba50bef20ee35").unwrap()
    );
    assert_eq!(bundle.messages[1].role, "user");
    assert_eq!(host.effects(), vec!["context"]);
    assert_eq!(host.milestone_kind_names(), vec!["prompt_bundle_built"]);
}

#[tokio::test]
async fn loop_prompt_port_preserves_mid_conversation_system_message_order() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_instruction_snippet("skill:alpha", "alpha skill context available")
            .with_context_tail_message("system", "msg:summary", "summary context available"),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 3);
    assert_eq!(bundle.messages[0].role, "system");
    assert_eq!(
        bundle.messages[0].content_ref,
        LoopMessageRef::new("msg:snippet.skill.alpha.0.25eba50bef20ee35").unwrap()
    );
    assert_eq!(bundle.messages[1].role, "user");
    assert_eq!(
        bundle.messages[1].content_ref,
        LoopMessageRef::new("msg:user-message").unwrap()
    );
    assert_eq!(bundle.messages[2].role, "system");
    assert_eq!(
        bundle.messages[2].content_ref,
        LoopMessageRef::new("msg:summary").unwrap()
    );
}

#[tokio::test]
async fn loop_prompt_port_keeps_identity_before_skill_snippets_and_records_skill_metadata() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_system_message("msg:identity", "identity context available")
            .with_context_instruction_snippet("skill:alpha", "alpha skill context available"),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_instruction_materialization_store(Arc::new(
        InMemoryInstructionMaterializationStore::default(),
    ));

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert_eq!(bundle.messages.len(), 3);
    assert_eq!(bundle.messages[0].role, "system");
    assert_eq!(
        bundle.messages[0].content_ref,
        LoopMessageRef::new("msg:identity").unwrap()
    );
    assert_eq!(bundle.messages[1].role, "system");
    assert_eq!(
        bundle.messages[1].content_ref,
        LoopMessageRef::new("msg:snippet.skill.alpha.0.25eba50bef20ee35").unwrap()
    );
    assert_eq!(bundle.messages[2].role, "user");

    let milestones = host.milestones();
    assert!(matches!(
        &milestones[0].kind,
        LoopHostMilestoneKind::PromptBundleBuilt { skill_context, .. }
            if skill_context.as_slice() == [PromptSkillContextMetadata {
                ordinal: 0,
                source_name: "alpha".to_string(),
                trust_level: "trusted".to_string(),
            }]
    ));
}

#[test]
fn prompt_bundle_built_deserializes_legacy_without_skill_context_metadata() {
    let legacy = serde_json::json!({
        "prompt_bundle_built": {
            "bundle_ref": "prompt:00000000-0000-0000-0000-000000000001:legacy",
            "mode": "text_only",
            "surface_version": null,
            "message_count": 1
        }
    });

    let kind: LoopHostMilestoneKind = serde_json::from_value(legacy).unwrap();

    assert!(matches!(
        kind,
        LoopHostMilestoneKind::PromptBundleBuilt { skill_context, .. }
            if skill_context.is_empty()
    ));
}

#[tokio::test]
async fn loop_prompt_port_rejects_unsupported_prompt_mode() {
    let host = Arc::new(RecordingAgentLoopHost::new(codeact_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::CodeAct,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert_eq!(host.effects(), Vec::<String>::new());
}

#[tokio::test]
async fn loop_prompt_port_rejects_malformed_same_run_checkpoint_ref() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: Some(
                LoopCheckpointStateRef::new(format!(
                    "checkpoint:{}:/host/path",
                    host.context.run_id
                ))
                .unwrap(),
            ),
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(host.effects(), Vec::<String>::new());
}

#[tokio::test]
async fn loop_prompt_port_rejects_cross_run_checkpoint_ref() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let other_context = LoopRunContext::new(
        host.context.scope.clone(),
        host.context.turn_id,
        TurnRunId::new(),
        host.context.resolved_run_profile.clone(),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: Some(
                LoopCheckpointStateRef::for_run(&other_context, "foreign-state").unwrap(),
            ),
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
    assert_eq!(host.effects(), Vec::<String>::new());
}

#[tokio::test]
async fn loop_prompt_port_rejects_cross_run_context_cursor() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let other_context = LoopRunContext::new(
        host.context.scope.clone(),
        host.context.turn_id,
        TurnRunId::new(),
        host.context.resolved_run_profile.clone(),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: Some(LoopInputCursor::origin_for_run(&other_context)),
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
    assert_eq!(host.effects(), Vec::<String>::new());
    assert!(host.context_requests().is_empty());
    assert!(host.milestones().is_empty());
}

#[tokio::test]
async fn loop_prompt_port_rejects_checkpoint_state_ref_until_supported() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: Some(
                LoopCheckpointStateRef::for_run(&host.context, "resume-state").unwrap(),
            ),
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(host.effects(), Vec::<String>::new());
    assert!(host.milestones().is_empty());
}

#[tokio::test]
async fn loop_prompt_port_rejects_unvalidated_surface_version() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(CapabilitySurfaceVersion::new("surface-v1").unwrap()),
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(host.effects(), Vec::<String>::new());
    assert!(host.milestones().is_empty());
}

#[tokio::test]
async fn loop_prompt_port_rejects_stale_surface_version() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_current_surface_version(CapabilitySurfaceVersion::new("surface-v2").unwrap());

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(CapabilitySurfaceVersion::new("surface-v1").unwrap()),
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::StaleSurface);
    assert_eq!(host.effects(), Vec::<String>::new());
    assert!(host.milestones().is_empty());
}

#[tokio::test]
async fn loop_prompt_port_rejects_unstored_synthetic_instruction_refs() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_instruction_snippet("instruction:system", "system instruction available"),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
}

#[tokio::test]
async fn loop_prompt_port_materializes_memory_surface_and_safety_as_host_owned_refs() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_instruction_snippet("instruction:system", "system instruction available")
            .with_context_memory_snippet("memory:project", "project memory available"),
    );
    let surface = VisibleCapabilitySurface {
        version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        descriptors: vec![CapabilityDescriptorView {
            capability_id: CapabilityId::new("demo.echo").unwrap(),
            provider: None,
            runtime: RuntimeKind::FirstParty,
            safe_name: "Echo".to_string(),
            safe_description: "Echo safe input".to_string(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: serde_json::json!({"type":"object","properties":{"input":{"type":"string"}}}),
        }],
    };
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    )
    .with_instruction_materialization_store(Arc::new(
        InMemoryInstructionMaterializationStore::default(),
    ))
    .with_current_surface(surface.clone())
    .with_safety_context(
        InstructionSafetyContext::new("safety:prompt-write", "prompt write safety enforced")
            .unwrap(),
    );

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: Some(surface.version),
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    assert!(bundle.messages.iter().any(|message| {
        message
            .content_ref
            .as_str()
            .starts_with("msg:memory.memory.project.")
    }));
    assert!(bundle.messages.iter().any(|message| {
        message
            .content_ref
            .as_str()
            .starts_with("msg:safety.safety.prompt-write.")
    }));
    assert!(bundle.messages.iter().any(|message| {
        message
            .content_ref
            .as_str()
            .starts_with("msg:surface.surface-v1.")
    }));
    assert!(bundle.instruction_fingerprint.is_some());
    assert_eq!(host.effects(), vec!["context"]);
    assert_eq!(host.milestone_kind_names(), vec!["prompt_bundle_built"]);
}

#[tokio::test]
async fn loop_prompt_port_rejects_zero_message_limit() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let error = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(0),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
    assert_eq!(host.effects(), Vec::<String>::new());
}

#[tokio::test]
async fn loop_prompt_port_clamps_default_and_requested_message_limits() {
    let zero_default_host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let zero_default_port = HostManagedLoopPromptPort::new(
        zero_default_host.context.clone(),
        zero_default_host.clone(),
        zero_default_host.milestone_sink.clone(),
    )
    .with_default_message_limit(0);

    zero_default_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(zero_default_host.context_request_limits(), vec![1]);

    let high_default_host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let high_default_port = HostManagedLoopPromptPort::new(
        high_default_host.context.clone(),
        high_default_host.clone(),
        high_default_host.milestone_sink.clone(),
    )
    .with_default_message_limit(usize::MAX);

    high_default_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(high_default_host.context_request_limits(), vec![128]);

    let high_request_host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let high_request_port = HostManagedLoopPromptPort::new(
        high_request_host.context.clone(),
        high_request_host.clone(),
        high_request_host.milestone_sink.clone(),
    );

    high_request_port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: Some(u32::MAX),
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();
    assert_eq!(high_request_host.context_request_limits(), vec![128]);
}

#[test]
fn capability_surface_versions_are_public_safe_tokens() {
    assert!(CapabilitySurfaceVersion::new("surface-v1").is_ok());
    assert!(CapabilitySurfaceVersion::new("empty:v1").is_ok());
    assert!(CapabilitySurfaceVersion::new("/host/path/surface").is_err());
    assert!(CapabilitySurfaceVersion::new("api_key:sk-test-secret").is_err());
    assert!(CapabilitySurfaceVersion::new("bearer:sk-test-secret").is_err());
    assert!(CapabilitySurfaceVersion::new("secret:v1").is_err());
    assert!(CapabilitySurfaceVersion::new("surface v1").is_err());
}

#[tokio::test]
async fn loop_prompt_bundle_public_serialization_hides_raw_content() {
    let host = Arc::new(
        RecordingAgentLoopHost::new(claimed_run_context().await)
            .with_context_message_safe_summary("safe prompt summary"),
    );
    let port = HostManagedLoopPromptPort::new(
        host.context.clone(),
        host.clone(),
        host.milestone_sink.clone(),
    );

    let bundle = port
        .build_prompt_bundle(LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            checkpoint_state_ref: None,
            max_messages: None,
            inline_messages: Vec::new(),
            capability_view: None,
        })
        .await
        .unwrap();

    let status = TurnRunState {
        scope: host.context.scope.clone(),
        actor: Some(TurnActor::new(UserId::new("user-loop-host").unwrap())),
        turn_id: host.context.turn_id,
        run_id: host.context.run_id,
        status: TurnStatus::Running,
        accepted_message_ref: AcceptedMessageRef::new("message-loop-host").unwrap(),
        source_binding_ref: SourceBindingRef::new("source-loop-host").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-loop-host").unwrap(),
        resolved_run_profile_id: host.context.resolved_run_profile.profile_id.clone(),
        resolved_run_profile_version: host.context.resolved_run_profile.profile_version,
        resolved_model_route: None,
        received_at: Utc.with_ymd_and_hms(2026, 5, 7, 12, 0, 0).unwrap(),
        checkpoint_id: None,
        gate_ref: None,
        failure: None,
        event_cursor: EventCursor(0),
    };
    let public_json = serde_json::to_string(&(bundle, host.milestones(), status)).unwrap();
    assert!(public_json.contains("prompt_bundle_built"));
    assert!(!public_json.contains("RAW_PROMPT_SENTINEL"));
    assert!(!public_json.contains("/host/path"));
    assert!(!public_json.contains("secret"));
}

#[test]
fn prompt_mode_wire_shape_uses_issue_contract_spelling() {
    assert_eq!(
        serde_json::to_string(&PromptMode::TextOnly).unwrap(),
        "\"text_only\""
    );
    assert_eq!(
        serde_json::to_string(&PromptMode::CodeAct).unwrap(),
        "\"codeact\""
    );
    assert_eq!(
        serde_json::from_str::<PromptMode>("\"codeact\"").unwrap(),
        PromptMode::CodeAct
    );
}

#[tokio::test]
async fn loop_run_context_actor_is_serde_backward_compatible() {
    let context = claimed_run_context().await;
    assert!(context.actor().is_none());

    let legacy_wire = serde_json::to_value(&context).unwrap();
    assert!(
        !legacy_wire.as_object().unwrap().contains_key("actor"),
        "actor: None must stay omitted for legacy wire compatibility"
    );
    let decoded: LoopRunContext = serde_json::from_value(legacy_wire).unwrap();
    assert!(decoded.actor().is_none());

    let actor = TurnActor::new(UserId::new("user-loop-context-serde").unwrap());
    let actor_context = context.with_actor(actor.clone());
    let decoded_with_actor: LoopRunContext =
        serde_json::from_value(serde_json::to_value(&actor_context).unwrap()).unwrap();
    assert_eq!(decoded_with_actor.actor(), Some(&actor));
}

#[tokio::test]
async fn prompt_bundle_refs_are_scoped_and_bounded() {
    let context = claimed_run_context().await;
    let bundle_ref = LoopPromptBundleRef::for_run(&context, "bundle-one").unwrap();

    assert!(bundle_ref.is_for_run(&context));
    assert!(LoopPromptBundleRef::new("prompt:missing-run-scope").is_err());
    assert!(LoopPromptBundleRef::for_run(&context, "x".repeat(200)).is_err());
}

#[tokio::test]
async fn capability_invocations_must_cite_visible_surface_before_host_dispatch() {
    let host = Arc::new(RecordingAgentLoopHost::new(claimed_run_context().await));
    let foreign = CapabilityId::new("demo.foreign").unwrap();

    let error = host
        .invoke_capability(CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
            capability_id: foreign,
            input_ref: CapabilityInputRef::new("input:opaque-agent-loop-host-sentinel").unwrap(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    assert_eq!(host.effects(), Vec::<String>::new());
    let serialized = serde_json::to_string(&error).unwrap();
    assert!(!serialized.contains("RAW_AGENT_LOOP_HOST_SENTINEL"));
}

#[test]
fn loop_host_refs_are_bounded_opaque_tokens() {
    assert!(CapabilityInputRef::new("input:opaque-tool-arguments").is_ok());
    assert!(CapabilityInputRef::new("{\"raw\":\"payload\"}").is_err());
    assert!(CapabilityInputRef::new(format!("input:{}", "x".repeat(256))).is_err());
    assert!(LoopCheckpointStateRef::new("checkpoint:state-ref").is_ok());
    assert!(LoopCheckpointStateRef::new("/host/path/checkpoint.json").is_err());
    assert!(LoopInputCursorToken::new("input-cursor:seen-1").is_ok());
    assert!(LoopInputCursorToken::new("999").is_err());
    assert!(LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "safe note").is_ok());
    assert!(LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "x".repeat(513)).is_err());
    assert!(
        LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "{\"tool_input\":\"raw\"}")
            .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(
            LoopDriverNoteKind::Planning,
            "/Users/alice/project/secret.txt"
        )
        .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "api_key=sk-test-secret")
            .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(
            LoopDriverNoteKind::Planning,
            "provider error: 401 invalid_api_key"
        )
        .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(
            LoopDriverNoteKind::Planning,
            "openai request failed: invalid api key"
        )
        .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(
            LoopDriverNoteKind::Planning,
            "access token expired for provider"
        )
        .is_err()
    );
    assert!(
        LoopProgressEvent::driver_note(LoopDriverNoteKind::Planning, "token:sk-test-secret")
            .is_err()
    );
}

#[test]
fn loop_host_refs_validate_when_deserialized() {
    let invalid_invocation = serde_json::json!({
        "surface_version": "surface-v1",
        "capability_id": "demo.echo",
        "input_ref": {"raw": "RAW_AGENT_LOOP_HOST_SENTINEL"}
    });
    assert!(serde_json::from_value::<CapabilityInvocation>(invalid_invocation).is_err());

    let invalid_checkpoint = serde_json::json!({
        "kind": "before_block",
        "state_ref": "raw-checkpoint-json"
    });
    assert!(serde_json::from_value::<LoopCheckpointRequest>(invalid_checkpoint).is_err());

    let invalid_surface = serde_json::json!("surface\n1");
    assert!(serde_json::from_value::<CapabilitySurfaceVersion>(invalid_surface).is_err());

    let forged_host_milestone = serde_json::json!({
        "model_started": {"requested_model_profile_id": null}
    });
    assert!(serde_json::from_value::<LoopProgressEvent>(forged_host_milestone).is_err());

    let unsafe_note = serde_json::json!({
        "driver_note": {"kind": "planning", "safe_summary": "raw\nprovider error"}
    });
    assert!(serde_json::from_value::<LoopProgressEvent>(unsafe_note).is_err());
}

#[test]
fn capability_denied_reason_kind_is_typed_and_wire_compatible() {
    let denied = CapabilityDenied {
        reason_kind: CapabilityDeniedReasonKind::EmptySurface,
        safe_summary: "no capabilities are available to this loop".to_string(),
    };

    let wire = serde_json::to_string(&denied).unwrap();
    assert!(wire.contains(r#""reason_kind":"empty_surface""#));

    let legacy = serde_json::json!({
        "reason_kind": "empty_surface",
        "safe_summary": "no capabilities are available to this loop"
    });
    let decoded = serde_json::from_value::<CapabilityDenied>(legacy).unwrap();
    assert_eq!(
        decoded.reason_kind,
        CapabilityDeniedReasonKind::EmptySurface
    );
    assert_eq!(decoded.reason_kind.as_str(), "empty_surface");
    assert_eq!(decoded.reason_kind.to_string(), "empty_surface");

    let historical_unknown = serde_json::json!({
        "reason_kind": "host_policy_denied",
        "safe_summary": "capability denied by host policy"
    });
    let decoded_unknown = serde_json::from_value::<CapabilityDenied>(historical_unknown).unwrap();
    assert_eq!(decoded_unknown.reason_kind.as_str(), "host_policy_denied");
    assert_eq!(
        decoded_unknown.reason_kind.to_string(),
        "host_policy_denied"
    );
    assert!(matches!(
        decoded_unknown.reason_kind,
        CapabilityDeniedReasonKind::Unknown(_)
    ));

    let unknown_wire = serde_json::to_string(&decoded_unknown).unwrap();
    assert!(unknown_wire.contains(r#""reason_kind":"host_policy_denied""#));

    let constructed_unknown = CapabilityDeniedReasonKind::unknown("host_policy_denied").unwrap();
    assert_eq!(constructed_unknown.as_str(), "host_policy_denied");
    assert!(CapabilityDeniedReasonKind::unknown("api_key").is_err());
    assert!(CapabilityDeniedReasonKind::unknown("secret_policy").is_err());
}

#[tokio::test]
async fn input_cursors_are_bound_to_the_claimed_run_context() {
    let context = claimed_run_context().await;
    let cursor = LoopInputCursor::from_host_token(
        &context,
        LoopInputCursorToken::new("input-cursor:seen-1").unwrap(),
    );
    assert!(cursor.is_for_run(&context));

    let other_context = LoopRunContext::new(
        context.scope.clone(),
        context.turn_id,
        TurnRunId::new(),
        context.resolved_run_profile.clone(),
    );
    assert!(!cursor.is_for_run(&other_context));
}

struct ReplyDriver;

#[async_trait]
impl AgentLoopDriver for ReplyDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        AgentLoopDriverDescriptor::new("lightweight_loop", RunProfileVersion::new(1)).unwrap()
    }

    async fn run(
        &self,
        request: ironclaw_turns::AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        assert_eq!(host.run_context().turn_id, request.turn_id);
        assert_eq!(host.run_context().run_id, request.run_id);
        let surface = host
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(driver_error)?;
        let prompt = host
            .build_prompt_bundle(LoopPromptBundleRequest {
                mode: PromptMode::TextOnly,
                context_cursor: None,
                surface_version: Some(surface.version),
                checkpoint_state_ref: None,
                max_messages: Some(8),
                inline_messages: Vec::new(),
                capability_view: None,
            })
            .await
            .map_err(driver_error)?;
        assert_eq!(prompt.messages.len(), 1);
        let response = host
            .stream_model(LoopModelRequest {
                messages: prompt.messages,
                surface_version: prompt.surface_version,
                model_preference: Some(
                    host.run_context()
                        .resolved_run_profile
                        .model_profile_id
                        .clone(),
                ),
                capability_view: None,
            })
            .await
            .map_err(driver_error)?;
        let ParentLoopOutput::AssistantReply(reply) = response.output else {
            return Err(AgentLoopDriverError::Failed {
                reason_kind: "unexpected_model_output".to_string(),
            });
        };
        let message_ref = host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(driver_error)?;
        host.emit_loop_progress(
            LoopProgressEvent::driver_note(
                LoopDriverNoteKind::Planning,
                "assistant transcript finalized",
            )
            .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?,
        )
        .await
        .map_err(driver_error)?;
        Ok(LoopExit::Completed(LoopCompleted {
            completion_kind: LoopCompletionKind::FinalReply,
            reply_message_refs: vec![message_ref],
            result_refs: Vec::new(),
            final_checkpoint_id: None,
            usage_summary_ref: None,
            exit_id: LoopExitId::new("exit:reply-driver").unwrap(),
        }))
    }

    async fn resume(
        &self,
        request: ironclaw_turns::AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run(
            ironclaw_turns::AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

struct CapabilityDriver;

#[async_trait]
impl AgentLoopDriver for CapabilityDriver {
    fn descriptor(&self) -> AgentLoopDriverDescriptor {
        AgentLoopDriverDescriptor::new("codeact_loop", RunProfileVersion::new(1)).unwrap()
    }

    async fn run(
        &self,
        _request: ironclaw_turns::AgentLoopDriverRunRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        let surface = host
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(driver_error)?;
        let outcome = host
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id: surface.descriptors[0].capability_id.clone(),
                input_ref: CapabilityInputRef::new("input:opaque-tool-arguments").unwrap(),
            })
            .await
            .map_err(driver_error)?;
        let CapabilityOutcome::ApprovalRequired { gate_ref, .. } = outcome else {
            return Err(AgentLoopDriverError::Failed {
                reason_kind: "expected_approval".to_string(),
            });
        };
        let state_ref = LoopCheckpointStateRef::new("checkpoint:approval-state").unwrap();
        let checkpoint_id = host
            .checkpoint(LoopCheckpointRequest {
                kind: LoopCheckpointKind::BeforeBlock,
                state_ref: state_ref.clone(),
            })
            .await
            .map_err(driver_error)?;
        host.emit_loop_progress(
            LoopProgressEvent::driver_note(LoopDriverNoteKind::Waiting, "blocked on approval gate")
                .map_err(|reason| AgentLoopDriverError::InvalidRequest { reason })?,
        )
        .await
        .map_err(driver_error)?;
        Ok(LoopExit::Blocked(LoopBlocked {
            kind: LoopBlockedKind::Approval,
            gate_ref,
            checkpoint_id,
            state_ref,
            exit_id: LoopExitId::new("exit:capability-driver").unwrap(),
        }))
    }

    async fn resume(
        &self,
        request: ironclaw_turns::AgentLoopDriverResumeRequest,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
    ) -> Result<LoopExit, AgentLoopDriverError> {
        self.run(
            ironclaw_turns::AgentLoopDriverRunRequest {
                turn_id: request.turn_id,
                run_id: request.run_id,
                resolved_run_profile: request.resolved_run_profile,
            },
            host,
        )
        .await
    }
}

#[derive(Default)]
struct FailingOnModelStartedMilestoneSink {
    kind_names: Mutex<Vec<&'static str>>,
}

impl FailingOnModelStartedMilestoneSink {
    fn kind_names(&self) -> Vec<&'static str> {
        self.kind_names.lock().unwrap().clone()
    }
}

#[async_trait]
impl LoopHostMilestoneSink for FailingOnModelStartedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelStarted { .. }) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "milestone sink unavailable",
            ));
        }
        self.kind_names
            .lock()
            .unwrap()
            .push(milestone.kind.kind_name());
        Ok(())
    }
}

#[derive(Default)]
struct FailingOnModelCompletedMilestoneSink {
    kind_names: Mutex<Vec<&'static str>>,
}

impl FailingOnModelCompletedMilestoneSink {
    fn kind_names(&self) -> Vec<&'static str> {
        self.kind_names.lock().unwrap().clone()
    }
}

#[async_trait]
impl LoopHostMilestoneSink for FailingOnModelCompletedMilestoneSink {
    async fn publish_loop_milestone(
        &self,
        milestone: LoopHostMilestone,
    ) -> Result<(), AgentLoopHostError> {
        if matches!(milestone.kind, LoopHostMilestoneKind::ModelCompleted { .. }) {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "milestone sink unavailable",
            ));
        }
        self.kind_names
            .lock()
            .unwrap()
            .push(milestone.kind.kind_name());
        Ok(())
    }
}

#[derive(Default)]
struct RecordingLoopModelGateway {
    requests: Mutex<Vec<LoopModelGatewayRequest>>,
    responses: Mutex<Vec<Result<LoopModelResponse, LoopModelGatewayError>>>,
}

impl RecordingLoopModelGateway {
    fn push_response(&self, response: Result<LoopModelResponse, LoopModelGatewayError>) {
        self.responses.lock().unwrap().push(response);
    }

    fn requests(&self) -> Vec<LoopModelGatewayRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl LoopModelGateway for RecordingLoopModelGateway {
    async fn stream_model(
        &self,
        request: LoopModelGatewayRequest,
    ) -> Result<LoopModelResponse, LoopModelGatewayError> {
        self.requests.lock().unwrap().push(request);
        self.responses.lock().unwrap().pop().unwrap()
    }
}

struct RecordingAgentLoopHost {
    context: LoopRunContext,
    effects: Mutex<Vec<String>>,
    context_requests: Mutex<Vec<LoopContextRequest>>,
    model_responses: Mutex<Vec<LoopModelResponse>>,
    capability_outcomes: Mutex<Vec<CapabilityOutcome>>,
    visible_surface: VisibleCapabilitySurface,
    milestone_sink: Arc<InMemoryLoopHostMilestoneSink>,
    context_message_safe_summary: String,
    context_system_messages: Vec<LoopContextMessage>,
    context_tail_messages: Vec<LoopContextMessage>,
    context_instruction_snippets: Vec<LoopContextSnippet>,
    context_memory_snippets: Vec<LoopContextSnippet>,
}

impl RecordingAgentLoopHost {
    fn new(context: LoopRunContext) -> Self {
        Self {
            context,
            effects: Mutex::new(Vec::new()),
            context_requests: Mutex::new(Vec::new()),
            model_responses: Mutex::new(Vec::new()),
            capability_outcomes: Mutex::new(Vec::new()),
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            visible_surface: VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("surface-v1").unwrap(),
                descriptors: vec![CapabilityDescriptorView {
                    capability_id: CapabilityId::new("demo.echo").unwrap(),
                    provider: None,
                    runtime: RuntimeKind::Wasm,
                    safe_name: "Echo".to_string(),
                    safe_description: "Returns an opaque result ref".to_string(),
                    concurrency_hint: ConcurrencyHint::Exclusive,
                    parameters_schema: serde_json::json!({"type":"object","properties":{"input":{"type":"string"}}}),
                }],
            },
            context_message_safe_summary: "hello".to_string(),
            context_system_messages: Vec::new(),
            context_tail_messages: Vec::new(),
            context_instruction_snippets: Vec::new(),
            context_memory_snippets: Vec::new(),
        }
    }

    fn with_context_message_safe_summary(mut self, safe_summary: impl Into<String>) -> Self {
        self.context_message_safe_summary = safe_summary.into();
        self
    }

    fn with_context_system_message(
        mut self,
        message_ref: impl Into<String>,
        safe_summary: impl Into<String>,
    ) -> Self {
        self.context_system_messages.push(LoopContextMessage {
            message_ref: Some(LoopMessageRef::new(message_ref.into()).unwrap()),
            role: "system".to_string(),
            safe_summary: safe_summary.into(),
        });
        self
    }

    fn with_context_tail_message(
        mut self,
        role: impl Into<String>,
        message_ref: impl Into<String>,
        safe_summary: impl Into<String>,
    ) -> Self {
        self.context_tail_messages.push(LoopContextMessage {
            message_ref: Some(LoopMessageRef::new(message_ref.into()).unwrap()),
            role: role.into(),
            safe_summary: safe_summary.into(),
        });
        self
    }

    fn with_context_instruction_snippet(
        mut self,
        snippet_ref: impl Into<String>,
        safe_summary: impl Into<String>,
    ) -> Self {
        let snippet_ref = snippet_ref.into();
        let metadata =
            snippet_ref
                .strip_prefix("skill:")
                .map(|source_name| LoopContextSnippetMetadata {
                    source_name: source_name.to_string(),
                    trust_level: "trusted".to_string(),
                });
        self.context_instruction_snippets.push(LoopContextSnippet {
            snippet_ref,
            safe_summary: safe_summary.into(),
            metadata,
        });
        self
    }

    fn with_context_memory_snippet(
        mut self,
        snippet_ref: impl Into<String>,
        safe_summary: impl Into<String>,
    ) -> Self {
        self.context_memory_snippets.push(LoopContextSnippet {
            snippet_ref: snippet_ref.into(),
            safe_summary: safe_summary.into(),
            metadata: None,
        });
        self
    }

    fn push_model_response(&self, response: LoopModelResponse) {
        self.model_responses.lock().unwrap().push(response);
    }

    fn push_capability_outcome(&self, outcome: CapabilityOutcome) {
        self.capability_outcomes.lock().unwrap().push(outcome);
    }

    fn effects(&self) -> Vec<String> {
        self.effects.lock().unwrap().clone()
    }

    fn context_requests(&self) -> Vec<LoopContextRequest> {
        self.context_requests.lock().unwrap().clone()
    }

    fn context_request_limits(&self) -> Vec<usize> {
        self.context_requests()
            .into_iter()
            .map(|request| request.limit)
            .collect()
    }

    fn milestones(&self) -> Vec<ironclaw_turns::run_profile::LoopHostMilestone> {
        self.milestone_sink.milestones()
    }

    fn milestone_kind_names(&self) -> Vec<&'static str> {
        self.milestones()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect()
    }

    fn milestone_emitter(&self) -> LoopHostMilestoneEmitter<InMemoryLoopHostMilestoneSink> {
        LoopHostMilestoneEmitter::new(self.context.clone(), self.milestone_sink.clone())
    }

    fn record(&self, effect: impl Into<String>) {
        self.effects.lock().unwrap().push(effect.into());
    }
}

impl LoopRunInfoPort for RecordingAgentLoopHost {
    fn run_context(&self) -> &LoopRunContext {
        &self.context
    }
}

#[async_trait]
impl LoopContextPort for RecordingAgentLoopHost {
    async fn load_loop_context(
        &self,
        request: LoopContextRequest,
    ) -> Result<LoopContextBundle, AgentLoopHostError> {
        self.context_requests.lock().unwrap().push(request);
        self.record("context");
        let mut messages = vec![LoopContextMessage {
            message_ref: Some(LoopMessageRef::new("msg:user-message").unwrap()),
            role: "user".to_string(),
            safe_summary: self.context_message_safe_summary.clone(),
        }];
        messages.extend(self.context_tail_messages.clone());
        Ok(LoopContextBundle {
            identity_messages: self.context_system_messages.clone(),
            messages,
            instruction_snippets: self.context_instruction_snippets.clone(),
            memory_snippets: self.context_memory_snippets.clone(),
        })
    }
}

#[async_trait]
impl LoopInputPort for RecordingAgentLoopHost {
    async fn poll_inputs(
        &self,
        _after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: LoopInputCursor::from_host_token(
                &self.context,
                LoopInputCursorToken::new("input-cursor:0").unwrap(),
            ),
        })
    }

    async fn ack_inputs(&self, _tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        Ok(())
    }
}

#[async_trait]
impl LoopPromptPort for RecordingAgentLoopHost {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<LoopPromptBundle, AgentLoopHostError> {
        self.record("prompt_bundle");
        let context = self
            .load_loop_context(LoopContextRequest {
                after: request.context_cursor,
                limit: request.max_messages.unwrap_or(8) as usize,
                mode: request.mode,
            })
            .await?;
        let bundle = LoopPromptBundle {
            bundle_ref: LoopPromptBundleRef::for_run(&self.context, "recording-host").map_err(
                |reason| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason),
            )?,
            messages: context
                .messages
                .into_iter()
                .filter_map(|message| {
                    message.message_ref.map(|content_ref| LoopModelMessage {
                        role: message.role,
                        content_ref,
                    })
                })
                .collect(),
            surface_version: request.surface_version,
            instruction_fingerprint: None,
            identity_message_count: 0,
            instruction_snippet_count: 0,
        };
        self.milestone_emitter()
            .prompt_bundle_built(
                bundle.bundle_ref.clone(),
                request.mode,
                bundle.surface_version.clone(),
                bundle.messages.len(),
                Vec::new(),
            )
            .await?;
        self.record("milestone:prompt_bundle_built");
        Ok(bundle)
    }
}

#[async_trait]
impl LoopModelPort for RecordingAgentLoopHost {
    async fn stream_model(
        &self,
        request: LoopModelRequest,
    ) -> Result<LoopModelResponse, AgentLoopHostError> {
        self.record("model");
        let emitter = self.milestone_emitter();
        emitter
            .model_started(request.model_preference.clone())
            .await?;
        self.record("milestone:model_started");
        let response = self.model_responses.lock().unwrap().pop().ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "model response unavailable",
            )
        })?;
        emitter
            .model_completed(response.effective_model_profile_id.clone())
            .await?;
        self.record("milestone:model_completed");
        Ok(response)
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingAgentLoopHost {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.record("visible_capabilities");
        Ok(self.visible_surface.clone())
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        if request.surface_version != self.visible_surface.version
            || !self
                .visible_surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == request.capability_id)
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "capability was not present in the cited visible surface",
            ));
        }
        self.record(format!("invoke:{}", request.capability_id));
        self.capability_outcomes
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "capability outcome unavailable",
                )
            })
    }

    async fn invoke_capability_batch(
        &self,
        _request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        Ok(CapabilityBatchOutcome {
            outcomes: Vec::new(),
            stopped_on_suspension: false,
        })
    }
}

#[async_trait]
impl LoopTranscriptPort for RecordingAgentLoopHost {
    async fn finalize_assistant_message(
        &self,
        request: FinalizeAssistantMessage,
    ) -> Result<LoopMessageRef, AgentLoopHostError> {
        assert_eq!(request.reply.content, "done");
        self.record("finalize_assistant");
        let message_ref = LoopMessageRef::new("msg:assistant-final")
            .map_err(|reason| AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, reason))?;
        self.milestone_emitter()
            .assistant_reply_finalized(message_ref.clone())
            .await?;
        self.record("milestone:assistant_reply_finalized");
        Ok(message_ref)
    }
}

#[async_trait]
impl LoopCheckpointPort for RecordingAgentLoopHost {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        self.record(format!("checkpoint:{}", request.kind.as_str()));
        Ok(TurnCheckpointId::new())
    }
}

#[async_trait]
impl LoopProgressPort for RecordingAgentLoopHost {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        self.record(format!("progress:{}", event.kind_name()));
        Ok(())
    }
}

impl LoopCancellationPort for RecordingAgentLoopHost {
    fn observe_cancellation(&self) -> Option<LoopCancellationSignal> {
        None
    }
}

async fn codeact_run_context() -> LoopRunContext {
    let mut context = claimed_run_context().await;
    let codeact_descriptor =
        AgentLoopDriverDescriptor::new("codeact_loop", RunProfileVersion::new(1)).unwrap();
    context.loop_driver_id = LoopDriverId::new("codeact_loop").unwrap();
    context.loop_driver_version = codeact_descriptor.version;
    context.resolved_run_profile.loop_driver = codeact_descriptor;
    context
}

async fn claimed_run_context() -> LoopRunContext {
    let scope = ironclaw_turns::TurnScope::new(
        TenantId::new("tenant-loop").unwrap(),
        Some(AgentId::new("agent-loop").unwrap()),
        Some(ProjectId::new("project-loop").unwrap()),
        ThreadId::new("thread-loop-host").unwrap(),
    );
    let store = Arc::new(InMemoryTurnStateStore::default());
    let coordinator = DefaultTurnCoordinator::new(store.clone());
    let response = coordinator
        .submit_turn(SubmitTurnRequest {
            scope: scope.clone(),
            actor: TurnActor::new(UserId::new("user-loop").unwrap()),
            accepted_message_ref: AcceptedMessageRef::new("message-loop-host").unwrap(),
            source_binding_ref: SourceBindingRef::new("source-loop-host").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply-loop-host").unwrap(),
            requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
            idempotency_key: IdempotencyKey::new("idem-loop-host").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 7, 12, 0, 0).unwrap(),
        })
        .await
        .unwrap();
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    let claimed = store
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.state.run_id, run_id);
    LoopRunContext::new(
        claimed.state.scope,
        claimed.state.turn_id,
        claimed.state.run_id,
        claimed.resolved_run_profile,
    )
}

fn driver_run_request(host: &RecordingAgentLoopHost) -> ironclaw_turns::AgentLoopDriverRunRequest {
    ironclaw_turns::AgentLoopDriverRunRequest {
        turn_id: host.context.turn_id,
        run_id: host.context.run_id,
        resolved_run_profile: host.context.resolved_run_profile.clone(),
    }
}

fn driver_error(error: AgentLoopHostError) -> AgentLoopDriverError {
    AgentLoopDriverError::Failed {
        reason_kind: error.kind.as_str().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Test helpers for budget accountant and policy guard
// ---------------------------------------------------------------------------

/// A policy guard that always denies model calls.
struct DenyAllPolicyGuard;

#[async_trait]
impl LoopModelPolicyGuard for DenyAllPolicyGuard {
    async fn check_model_policy(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        Err(LoopModelGatewayError::new(
            AgentLoopHostErrorKind::PolicyDenied,
            "model denied by policy",
        )
        .expect("safe summary is valid"))
    }
}

/// A recording budget accountant that tracks pre/post calls and can be
/// configured to reject pre-call.
struct RecordingBudgetAccountant {
    pre_called: AtomicBool,
    post_called: AtomicBool,
    reject_pre: AtomicBool,
    reject_post: AtomicBool,
    post_saw_failure: AtomicBool,
}

impl RecordingBudgetAccountant {
    fn new() -> Self {
        Self {
            pre_called: AtomicBool::new(false),
            post_called: AtomicBool::new(false),
            reject_pre: AtomicBool::new(false),
            reject_post: AtomicBool::new(false),
            post_saw_failure: AtomicBool::new(false),
        }
    }

    fn rejecting() -> Self {
        let accountant = Self::new();
        accountant.reject_pre.store(true, Ordering::SeqCst);
        accountant
    }

    fn rejecting_post() -> Self {
        let accountant = Self::new();
        accountant.reject_post.store(true, Ordering::SeqCst);
        accountant
    }

    fn was_pre_called(&self) -> bool {
        self.pre_called.load(Ordering::SeqCst)
    }

    fn was_post_called(&self) -> bool {
        self.post_called.load(Ordering::SeqCst)
    }

    fn post_saw_failure(&self) -> bool {
        self.post_saw_failure.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LoopModelBudgetAccountant for RecordingBudgetAccountant {
    async fn pre_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
    ) -> Result<(), LoopModelGatewayError> {
        self.pre_called.store(true, Ordering::SeqCst);
        if self.reject_pre.load(Ordering::SeqCst) {
            return Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "model call budget exceeded",
            )
            .expect("safe summary is valid"));
        }
        Ok(())
    }

    async fn post_model_call(
        &self,
        _context: &LoopRunContext,
        _request: &LoopModelRequest,
        outcome: ModelCallOutcome<'_>,
    ) -> Result<(), LoopModelGatewayError> {
        self.post_called.store(true, Ordering::SeqCst);
        if matches!(outcome, ModelCallOutcome::Failure(_)) {
            self.post_saw_failure.store(true, Ordering::SeqCst);
        }
        if self.reject_post.load(Ordering::SeqCst) {
            return Err(LoopModelGatewayError::new(
                AgentLoopHostErrorKind::BudgetExceeded,
                "model call accounting failed",
            )
            .expect("safe summary is valid"));
        }
        Ok(())
    }
}

fn simple_model_request(context: &LoopRunContext) -> LoopModelRequest {
    LoopModelRequest {
        messages: vec![LoopModelMessage {
            role: "user".to_string(),
            content_ref: LoopMessageRef::new("msg:user-message").unwrap(),
        }],
        surface_version: None,
        model_preference: Some(context.resolved_run_profile.model_profile_id.clone()),
        capability_view: None,
    }
}

fn success_response(context: &LoopRunContext) -> LoopModelResponse {
    LoopModelResponse {
        chunks: vec![ironclaw_turns::run_profile::ModelStreamChunk {
            safe_text_delta: "safe delta".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "hello".to_string(),
        }),
        effective_model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    }
}

// ---------------------------------------------------------------------------
// KB-006 acceptance criteria tests
// ---------------------------------------------------------------------------

/// Denied policy rejects without calling the downstream provider gateway.
#[tokio::test]
async fn denied_policy_rejects_without_calling_gateway() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(success_response(&context)));

    let port = HostManagedLoopModelPort::with_guards(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        Arc::new(RecordingBudgetAccountant::new()),
        Arc::new(DenyAllPolicyGuard),
    );

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::PolicyDenied);
    assert!(error.safe_summary.contains("model denied by policy"));
    // Gateway was never called.
    assert_eq!(gateway.requests().len(), 0);
    // No milestones emitted.
    assert!(milestone_sink.milestones().is_empty());
}

/// Missing/unavailable credential returns a stable sanitized error with the
/// CredentialUnavailable kind, containing no raw credential strings.
#[tokio::test]
async fn missing_credential_returns_sanitized_credential_unavailable_error() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Err(LoopModelGatewayError::new(
        AgentLoopHostErrorKind::CredentialUnavailable,
        "credential not available for requested model",
    )
    .unwrap()));

    let port =
        HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::CredentialUnavailable);
    let serialized = serde_json::to_string(&error).unwrap();
    let debug_output = format!("{:?}", error);
    // No raw credential strings leak.
    for sentinel in ["sk-test", "OPENAI_API_KEY", "Bearer ", "authorization:"] {
        assert!(
            !serialized.contains(sentinel),
            "serialized error must not contain `{sentinel}`"
        );
        assert!(
            !debug_output.contains(sentinel),
            "debug output must not contain `{sentinel}`"
        );
    }
}

/// Redaction sentinel test: push a response/error containing known sentinels
/// and verify they don't appear in serialized errors, debug output, deltas,
/// milestone metadata, or safe summaries.
#[tokio::test]
async fn redaction_sentinels_never_leak_through_serialized_surfaces() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());

    // Success path: response content has a sentinel, but milestones must not expose it.
    gateway.push_response(Ok(LoopModelResponse {
        chunks: vec![ironclaw_turns::run_profile::ModelStreamChunk {
            safe_text_delta: "RAW_CREDENTIAL_SENTINEL visible in chunk".to_string(),
        }],
        output: ParentLoopOutput::AssistantReply(AssistantReply {
            content: "sk-test-key-12345 leaked content".to_string(),
        }),
        effective_model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
        usage: None,
    }));

    let port =
        HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

    let response = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap();

    // Milestones must not contain the sentinels.
    let serialized_milestones = serde_json::to_string(&milestone_sink.milestones()).unwrap();
    for sentinel in ["RAW_CREDENTIAL_SENTINEL", "sk-test-key-12345"] {
        assert!(
            !serialized_milestones.contains(sentinel),
            "milestone serialization must not contain `{sentinel}`"
        );
    }

    let serialized_response = serde_json::to_string(&response).unwrap();
    let debug_response = format!("{:?}", response);
    for sentinel in ["RAW_CREDENTIAL_SENTINEL", "sk-test-key-12345"] {
        assert!(
            response
                .chunks
                .iter()
                .all(|chunk| !chunk.safe_text_delta.contains(sentinel)),
            "model chunks must not contain `{sentinel}`"
        );
        assert!(
            !serialized_response.contains(sentinel),
            "serialized response must not contain `{sentinel}`"
        );
        assert!(
            !debug_response.contains(sentinel),
            "debug response must not contain `{sentinel}`"
        );
    }

    let debug_milestones = format!("{:?}", milestone_sink.milestones());
    for sentinel in ["RAW_CREDENTIAL_SENTINEL", "sk-test-key-12345"] {
        assert!(
            !debug_milestones.contains(sentinel),
            "milestone debug output must not contain `{sentinel}`"
        );
    }

    // Error path: push a gateway error and verify it is clean.
    let gateway2 = Arc::new(RecordingLoopModelGateway::default());
    gateway2.push_response(Err(LoopModelGatewayError::new(
        AgentLoopHostErrorKind::Unavailable,
        "model unavailable",
    )
    .unwrap()));
    let milestone_sink2 = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let port2 =
        HostManagedLoopModelPort::new(context.clone(), gateway2.clone(), milestone_sink2.clone());

    let error = port2
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    let serialized_error = serde_json::to_string(&error).unwrap();
    let debug_error = format!("{:?}", error);
    for sentinel in [
        "RAW_CREDENTIAL_SENTINEL",
        "sk-test-key-12345",
        "api_key",
        "secret",
    ] {
        assert!(
            !serialized_error.contains(sentinel),
            "serialized error must not contain `{sentinel}`"
        );
        assert!(
            !debug_error.contains(sentinel),
            "debug error must not contain `{sentinel}`"
        );
    }
}

/// LoopSafeSummary rejects API-key-like tokens and known credential patterns.
#[test]
fn safe_summary_validation_rejects_credential_patterns() {
    // sk-prefixed tokens
    assert!(
        LoopModelGatewayError::new(
            AgentLoopHostErrorKind::Unavailable,
            "token:sk-test-secret-key"
        )
        .is_err()
    );
    // "api key" family
    assert!(
        LoopModelGatewayError::new(
            AgentLoopHostErrorKind::Unauthorized,
            "openai request failed: invalid api key"
        )
        .is_err()
    );
    // "bearer " prefix
    assert!(
        LoopModelGatewayError::new(
            AgentLoopHostErrorKind::Unauthorized,
            "authorization: bearer xyz"
        )
        .is_err()
    );
    // But clean summaries pass
    assert!(
        LoopModelGatewayError::new(AgentLoopHostErrorKind::Unavailable, "model unavailable")
            .is_ok()
    );
}

/// Budget accounting on success: both pre and post hooks fire with correct context.
#[tokio::test]
async fn budget_accounting_on_success_invokes_pre_and_post() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(success_response(&context)));
    let accountant = Arc::new(RecordingBudgetAccountant::new());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let _response = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap();

    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(!accountant.post_saw_failure());
    // Gateway was called.
    assert_eq!(gateway.requests().len(), 1);
}

/// Post-call accounting failure after provider success fails closed instead of
/// returning an unaccounted successful response.
#[tokio::test]
async fn post_accounting_failure_after_success_fails_closed() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(success_response(&context)));
    let accountant = Arc::new(RecordingBudgetAccountant::rejecting_post());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
    assert_eq!(error.safe_summary, "model call accounting failed");
    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(!accountant.post_saw_failure());
    assert_eq!(gateway.requests().len(), 1);
}

/// Model-started milestone projection failures are non-fatal; accounting still
/// wraps the provider call and records its actual outcome.
#[tokio::test]
async fn model_started_failure_still_accounts_provider_outcome() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(FailingOnModelStartedMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(success_response(&context)));
    let accountant = Arc::new(RecordingBudgetAccountant::new());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let response = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap();

    assert_eq!(
        response.effective_model_profile_id,
        context.resolved_run_profile.model_profile_id
    );
    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(!accountant.post_saw_failure());
    assert_eq!(gateway.requests().len(), 1);
    assert_eq!(milestone_sink.kind_names(), vec!["model_completed"]);
}

/// Budget accounting on failure: post hook still fires.
#[tokio::test]
async fn budget_accounting_on_failure_still_fires_post() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Err(LoopModelGatewayError::new(
        AgentLoopHostErrorKind::Unavailable,
        "model unavailable",
    )
    .unwrap()));
    let accountant = Arc::new(RecordingBudgetAccountant::new());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(accountant.post_saw_failure());
}

/// Post-call accounting failure after provider failure must fail closed so
/// reservation cleanup/reconciliation loss cannot hide behind the model error.
#[tokio::test]
async fn post_accounting_failure_after_gateway_failure_fails_closed() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Err(LoopModelGatewayError::new(
        AgentLoopHostErrorKind::Unavailable,
        "model unavailable",
    )
    .unwrap()));
    let accountant = Arc::new(RecordingBudgetAccountant::rejecting_post());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
    assert_eq!(error.safe_summary, "model call accounting failed");
    assert!(accountant.was_pre_called());
    assert!(accountant.was_post_called());
    assert!(accountant.post_saw_failure());
    assert_eq!(gateway.requests().len(), 1);
    assert_eq!(
        milestone_sink
            .milestones()
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect::<Vec<_>>(),
        vec!["model_started", "model_failed"]
    );
}

/// Budget-exceeded pre-call rejection prevents gateway call.
#[tokio::test]
async fn budget_exceeded_pre_call_rejects_without_calling_gateway() {
    let context = claimed_run_context().await;
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let gateway = Arc::new(RecordingLoopModelGateway::default());
    gateway.push_response(Ok(success_response(&context)));
    let accountant = Arc::new(RecordingBudgetAccountant::rejecting());

    let port = HostManagedLoopModelPort::with_accountant(
        context.clone(),
        gateway.clone(),
        milestone_sink.clone(),
        accountant.clone(),
    );

    let error = port
        .stream_model(simple_model_request(&context))
        .await
        .unwrap_err();

    assert_eq!(error.kind, AgentLoopHostErrorKind::BudgetExceeded);
    assert!(error.safe_summary.contains("budget exceeded"));
    // Gateway was never called.
    assert_eq!(gateway.requests().len(), 0);
    // No milestones emitted.
    assert!(milestone_sink.milestones().is_empty());
    // Pre was called, post was not (pre rejected before gateway).
    assert!(accountant.was_pre_called());
    assert!(!accountant.was_post_called());
}

/// Error kind mapping: verify all gateway error kinds map correctly through
/// HostManagedLoopModelPort to the corresponding AgentLoopHostError kinds.
#[tokio::test]
async fn error_kind_mapping_through_host_managed_port() {
    let context = claimed_run_context().await;

    let kinds_and_summaries = [
        (AgentLoopHostErrorKind::Unavailable, "model unavailable"),
        (
            AgentLoopHostErrorKind::Unauthorized,
            "model call unauthorized",
        ),
        (
            AgentLoopHostErrorKind::BudgetExceeded,
            "model call budget exceeded",
        ),
        (
            AgentLoopHostErrorKind::PolicyDenied,
            "model denied by policy",
        ),
        (
            AgentLoopHostErrorKind::CredentialUnavailable,
            "credential not available for requested model",
        ),
    ];

    for (expected_kind, summary) in kinds_and_summaries {
        let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
        let gateway = Arc::new(RecordingLoopModelGateway::default());
        gateway.push_response(Err(
            LoopModelGatewayError::new(expected_kind, summary).unwrap()
        ));

        let port =
            HostManagedLoopModelPort::new(context.clone(), gateway.clone(), milestone_sink.clone());

        let error = port
            .stream_model(simple_model_request(&context))
            .await
            .unwrap_err();

        assert_eq!(
            error.kind, expected_kind,
            "expected {expected_kind:?} but got {:?}",
            error.kind
        );
        assert_eq!(error.safe_summary, summary);
    }
}
