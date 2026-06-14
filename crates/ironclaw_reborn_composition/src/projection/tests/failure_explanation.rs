use super::*;
use ironclaw_reborn::failure_categories::{
    HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY, HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY,
    HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY, HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
    HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY, HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY,
    HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY, MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
    MODEL_CREDITS_EXHAUSTED_CATEGORY,
};
use ironclaw_turns::LoopFailureKind;

const GENERIC_FAILURE_SUMMARY: &str = "The run failed before producing a reply. Retry the run, and contact support if it keeps happening.";

#[tokio::test]
async fn webui_event_stream_projects_failed_run_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-failed-thread",
        "lease_expired",
        "The run failed because its runner lease expired. Retry the run.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_no_progress_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-no-progress-thread",
        "no_progress_detected",
        "The run stopped because it repeated work without making progress. Retry with a clearer instruction or narrower scope.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_iteration_limit_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-iteration-limit-thread",
        "iteration_limit",
        "The run stopped after reaching its iteration limit before producing a reply. Retry with a narrower request or increase the limit.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_context_build_failed_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-context-build-thread",
        "context_build_failed",
        "The run failed while building the model context. Retry the run, and contact support if it keeps happening.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_host_stage_unavailable_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-host-stage-thread",
        "host_stage_unavailable_checkpoint",
        "The run failed because the host checkpoint stage was unavailable. Retry the run, and contact support if checkpoints remain unavailable.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_unknown_failure_summary_without_echoing_code() {
    assert_failed_run_status_summary(
        "webui-events-unknown-thread",
        "unexpected_new_failure",
        GENERIC_FAILURE_SUMMARY,
    )
    .await;
}

#[test]
fn failure_summary_covers_every_loop_failure_kind_category() {
    let expected = [
        (
            LoopFailureKind::ModelError.as_str(),
            "The run failed while calling the model. Check the selected model provider and try again.",
        ),
        (
            LoopFailureKind::ContextBuildFailed.as_str(),
            "The run failed while building the model context. Retry the run, and contact support if it keeps happening.",
        ),
        (
            LoopFailureKind::CapabilityProtocolError.as_str(),
            "The run failed because a capability returned an invalid protocol response. Retry the run, and contact support if it keeps happening.",
        ),
        (
            LoopFailureKind::IterationLimit.as_str(),
            "The run stopped after reaching its iteration limit before producing a reply. Retry with a narrower request or increase the limit.",
        ),
        (
            LoopFailureKind::InvalidModelOutput.as_str(),
            "The run failed because the model returned output the runner could not use. Retry the run or choose a different model.",
        ),
        (
            LoopFailureKind::CheckpointRejected.as_str(),
            "The run failed because its checkpoint was rejected. Retry from the last available checkpoint or start a new run.",
        ),
        (
            LoopFailureKind::CheckpointUnavailable.as_str(),
            "The run failed because the checkpoint could not be loaded. Retry the run, and contact support if the checkpoint remains unavailable.",
        ),
        (
            LoopFailureKind::TranscriptWriteFailed.as_str(),
            "The run failed while saving transcript output. Retry the run, and contact support if saving still fails.",
        ),
        (
            LoopFailureKind::DriverBug.as_str(),
            "The run failed because the execution driver hit an internal bug. Retry the run, and contact support if it happens again.",
        ),
        (
            LoopFailureKind::InterruptedUnexpectedly.as_str(),
            "The run stopped unexpectedly before it could finish. Retry the run.",
        ),
        (
            LoopFailureKind::NoProgressDetected.as_str(),
            "The run stopped because it repeated work without making progress. Retry with a clearer instruction or narrower scope.",
        ),
        (
            LoopFailureKind::PolicyDenied.as_str(),
            "The run stopped because a policy denied the requested action. Change the request or permissions and try again.",
        ),
        (
            LoopFailureKind::CompactionUnavailable.as_str(),
            "The run failed because context compaction was unavailable. Retry with a shorter request or start a new thread.",
        ),
    ];

    let source_values = loop_failure_kind_as_str_values_from_source();
    let expected_values = expected
        .iter()
        .map(|(category, _)| *category)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        source_values, expected_values,
        "LoopFailureKind::as_str gained or lost a category; update the Tier-2 summary table"
    );

    for (category, expected_summary) in expected {
        let summary = crate::failure_summary::reborn_failure_summary_for_category(Some(category));
        assert_eq!(summary, expected_summary, "category {category}");
        assert_ne!(summary, GENERIC_FAILURE_SUMMARY, "category {category}");
        assert!(
            !summary.trim().eq(category),
            "summary must not be a raw failure category"
        );
    }
}

#[test]
fn failure_summary_covers_reborn_failure_category_constants() {
    let expected = [
        (
            MODEL_CREDITS_EXHAUSTED_CATEGORY,
            "The AI provider account is out of credits. Add credits or switch providers and try again.",
        ),
        (
            MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
            "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL, then try again.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_PROMPT_CATEGORY,
            "The run failed because the host prompt stage was unavailable. Retry the run, and contact support if it keeps happening.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_MODEL_CATEGORY,
            "The run failed because the host model stage was unavailable. Check the model provider and try again.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_CAPABILITY_CATEGORY,
            "The run failed because the host capability stage was unavailable. Retry the run, and check the tool integration if it keeps happening.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_TRANSCRIPT_CATEGORY,
            "The run failed because the host transcript stage was unavailable. Retry the run, and contact support if saving still fails.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_CHECKPOINT_CATEGORY,
            "The run failed because the host checkpoint stage was unavailable. Retry the run, and contact support if checkpoints remain unavailable.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_INPUT_CATEGORY,
            "The run failed because the host input stage was unavailable. Check the submitted message and try again.",
        ),
        (
            HOST_STAGE_UNAVAILABLE_UNKNOWN_CATEGORY,
            "The run failed because a required host stage was unavailable. Retry the run, and contact support if it keeps happening.",
        ),
    ];
    let source_values = reborn_failure_category_constant_values_from_source();
    let expected_values = expected
        .iter()
        .map(|(category, _)| *category)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        source_values, expected_values,
        "failure_categories.rs gained or lost a public category constant; update the Tier-2 summary table"
    );

    for (category, expected_summary) in expected {
        assert_eq!(
            crate::failure_summary::reborn_failure_summary_for_category(Some(category)),
            expected_summary,
            "category {category}"
        );
    }
}

#[test]
fn failure_summary_covers_host_stage_unavailable_categories() {
    let expected = [
        (
            "host_stage_unavailable_prompt",
            "The run failed because the host prompt stage was unavailable. Retry the run, and contact support if it keeps happening.",
        ),
        (
            "host_stage_unavailable_model",
            "The run failed because the host model stage was unavailable. Check the model provider and try again.",
        ),
        (
            "host_stage_unavailable_capability",
            "The run failed because the host capability stage was unavailable. Retry the run, and check the tool integration if it keeps happening.",
        ),
        (
            "host_stage_unavailable_transcript",
            "The run failed because the host transcript stage was unavailable. Retry the run, and contact support if saving still fails.",
        ),
        (
            "host_stage_unavailable_checkpoint",
            "The run failed because the host checkpoint stage was unavailable. Retry the run, and contact support if checkpoints remain unavailable.",
        ),
        (
            "host_stage_unavailable_input",
            "The run failed because the host input stage was unavailable. Check the submitted message and try again.",
        ),
        (
            "host_stage_unavailable_unknown",
            "The run failed because a required host stage was unavailable. Retry the run, and contact support if it keeps happening.",
        ),
    ];

    for (category, expected_summary) in expected {
        assert_eq!(
            crate::failure_summary::reborn_failure_summary_for_category(Some(category)),
            expected_summary,
            "category {category}"
        );
    }
}

#[test]
fn failure_summary_covers_agent_loop_safe_summary_categories() {
    let expected = [
        (
            "model_transient",
            "The run failed after a temporary model error. Retry the run.",
        ),
        (
            "model_context_overflow",
            "The run failed because the model context was too large. Retry with a shorter request or start a new thread.",
        ),
        (
            "model_content_filtered",
            "The run failed because the model provider filtered the response. Change the request and try again.",
        ),
        (
            "model_unavailable",
            "The run failed because the model provider was unavailable. Check the selected provider and retry the run.",
        ),
        (
            "model_internal",
            "The run failed because the model provider returned an internal error. Retry the run or choose a different provider.",
        ),
        (
            "capability_transient",
            "The run failed after a temporary tool error. Retry the run.",
        ),
        (
            "capability_permanent",
            "The run failed because a tool reported a permanent error. Change the request or tool configuration and try again.",
        ),
        (
            "capability_input_invalid",
            "The run failed because a tool rejected its input. Retry with a clearer or narrower request.",
        ),
        (
            "capability_operation_failed",
            "The run failed because a tool operation did not complete. Retry the run, and check the tool integration if it keeps happening.",
        ),
        (
            "capability_policy_denied",
            "The run failed because a tool policy denied the requested action. Change the request or permissions and try again.",
        ),
        (
            "capability_unavailable",
            "The run failed because a required tool was unavailable. Retry the run, and check the tool integration if it keeps happening.",
        ),
        (
            "capability_internal",
            "The run failed because a tool returned an internal error. Retry the run, and check the tool integration if it keeps happening.",
        ),
        (
            "compaction_invalid_cut_point",
            "The run failed because context compaction selected an invalid cut point. Retry the run, and contact support if it keeps happening.",
        ),
        (
            "compaction_unsupported_mode",
            "The run failed because the requested context compaction mode is unsupported. Retry with a shorter request or start a new thread.",
        ),
        (
            "compaction_input_too_large",
            "The run failed because context compaction input was too large. Retry with a shorter request or start a new thread.",
        ),
        (
            "compaction_security_rejected",
            "The run failed because context compaction was rejected by a safety check. Change the request and try again.",
        ),
        (
            "compaction_inference_failed",
            "The run failed because context compaction could not complete. Retry with a shorter request or start a new thread.",
        ),
        (
            "compaction_cancelled",
            "The run stopped while context compaction was being cancelled. Retry the run if you still need a response.",
        ),
        (
            "compaction_persistence_failed",
            "The run failed while saving compacted context. Retry the run, and contact support if saving still fails.",
        ),
    ];

    // Parity guard: the hardcoded table above must stay exhaustive against the
    // agent-loop source that mints these safe-summary categories. Without this,
    // a newly added category would silently fall through to
    // GENERIC_FAILURE_SUMMARY and this test would still pass.
    let source_values = agent_loop_safe_summary_category_values_from_source();
    let expected_values = expected
        .iter()
        .map(|(category, _)| *category)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        source_values, expected_values,
        "agent-loop gained or lost a safe-summary category; update the expected table and its user-facing summary"
    );

    for (category, expected_summary) in expected {
        let summary = crate::failure_summary::reborn_failure_summary_for_category(Some(category));
        assert_eq!(summary, expected_summary, "category {category}");
        assert_ne!(summary, GENERIC_FAILURE_SUMMARY, "category {category}");
    }
}

#[test]
fn failure_summary_uses_safe_generic_fallback_for_unknown_categories() {
    let summary =
        crate::failure_summary::reborn_failure_summary_for_category(Some("new_snake_case_code"));

    assert_eq!(summary, GENERIC_FAILURE_SUMMARY);
    assert_ne!(summary, "new_snake_case_code");
}

async fn assert_failed_run_status_summary(
    thread_id: &str,
    failure_category: &str,
    expected_summary: &str,
) {
    assert_failed_run_status_summary_with_explainer(
        thread_id,
        failure_category,
        expected_summary,
        None,
    )
    .await;
}

/// A failed-run lifecycle event carrying `retryable = Some(true)` must
/// surface that flag on the projected `RunStatus` item so the WebUI can
/// offer a retry affordance. Regression guard for the retry-from-failed
/// surfacing path.
#[tokio::test]
async fn webui_event_stream_projects_retryable_flag_for_failed_run() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-retryable-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("lease_expired".to_string()),
                retryable: Some(true),
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
                matches!(
                    item,
                    ProductProjectionItem::RunStatus {
                        run_id,
                        retryable: Some(true),
                        ..
                    } if *run_id == turn_run
                )
            }),
            _ => false,
        }),
        "failed-run projection must carry retryable = Some(true)"
    );
}

async fn assert_failed_run_status_summary_with_explainer(
    thread_id: &str,
    failure_category: &str,
    expected_summary: &str,
    failure_explainer: Option<Arc<dyn FailureExplanationProvider>>,
) {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new(thread_id).unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some(failure_category.to_string()),
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    );
    let services = if let Some(failure_explainer) = failure_explainer {
        services.with_failure_explainer(failure_explainer)
    } else {
        services
    };

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && status == "failed"
                    && category.category() == failure_category
                    && summary == expected_summary
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_projects_model_credit_exhaustion_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-credit-failed-thread",
        MODEL_CREDITS_EXHAUSTED_CATEGORY,
        "The AI provider account is out of credits. Add credits or switch providers and try again.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_projects_model_credentials_failure_summary() {
    assert_failed_run_status_summary(
        "webui-events-model-credentials-thread",
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
        "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL, then try again.",
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_pins_model_credentials_summary_before_explainer() {
    assert_failed_run_status_summary_with_explainer(
        "webui-events-pinned-model-credentials-thread",
        MODEL_CREDENTIALS_UNAVAILABLE_CATEGORY,
        "The run failed because model credentials or provider configuration are invalid. Check the selected provider's API key and base URL, then try again.",
        Some(Arc::new(FakeFailureExplainer {
            explanation: "SENTINEL explainer output should not be used".to_string(),
        })),
    )
    .await;
}

#[tokio::test]
async fn webui_event_stream_uses_model_failure_explanation_when_available() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-model-failed-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_invalid_request".to_string()),
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_failure_explainer(Arc::new(FakeFailureExplainer {
        explanation:
            "The run asked the driver for an invalid operation, so it stopped before replying."
                .to_string(),
    }));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && status == "failed"
                    && category.category() == "driver_invalid_request"
                    && summary
                        == "The run asked the driver for an invalid operation, so it stopped before replying."
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn webui_event_stream_caches_model_failure_explanation_across_replay() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-cache-failed-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_invalid_request".to_string()),
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1)),
        }),
    )
    .with_failure_explainer(Arc::new(CountingFailureExplainer {
        explanation: "The driver rejected this request, so the run stopped.".to_string(),
        calls: Arc::clone(&calls),
    }));

    for _ in 0..2 {
        let events = services
            .webui_event_stream()
            .drain(ProjectionSubscriptionRequest {
                actor: actor.clone(),
                scope: scope.clone(),
                after_cursor: None,
            })
            .await
            .unwrap();

        assert!(events.iter().any(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => {
                state.items.iter().any(|item| {
                    matches!(
                        item,
                        ProductProjectionItem::RunStatus {
                            run_id,
                            failure_summary: Some(summary),
                            ..
                        } if *run_id == turn_run
                            && summary == "The driver rejected this request, so the run stopped."
                    )
                })
            }
            _ => false,
        }));
    }

    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn webui_event_stream_projects_recovery_required_failure_summary() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-recovery-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::RecoveryRequired,
                kind: TurnEventKind::RecoveryRequired,
                blocked_gate: None,
                sanitized_reason: Some("driver_failed".to_string()),
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::RecoveryRequired,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    );

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    status,
                    failure_category: Some(category),
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && status == "recovery_required"
                    && category.category() == "driver_failed"
                    && summary == "The run failed because the execution driver reported an error. Retry the run, and contact support if it happens again."
            )
        }),
        _ => false,
    }));
}

#[tokio::test]
async fn failure_details_returns_fallback_when_model_gateway_times_out() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-timeout-fallback-thread").unwrap();
    let turn_run = TurnRunId::new();
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let event_log_dyn: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let actor = TurnActor::new(user_id.clone());
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    )
    .with_turn_events(
        Arc::new(FakeTurnEventSource {
            events: vec![TurnLifecycleEvent {
                cursor: TurnEventCursor(1),
                scope: scope.clone(),
                occurred_at: Some(chrono::Utc::now()),
                owner_user_id: Some(user_id.clone()),
                run_id: turn_run,
                status: TurnStatus::Failed,
                kind: TurnEventKind::Failed,
                blocked_gate: None,
                sanitized_reason: Some("driver_panic".to_string()),
                retryable: None,
            }],
        }),
        Arc::new(FakeTurnCoordinator {
            state: TurnRunState {
                status: TurnStatus::Failed,
                ..turn_run_state(&scope, &user_id, turn_run, TurnEventCursor(1))
            },
        }),
    )
    .with_failure_explainer(Arc::new(ModelFailureExplanationProvider::new(Arc::new(
        SlowSystemInference,
    ))));

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| match event.payload() {
        ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter().any(|item| {
            matches!(
                item,
                ProductProjectionItem::RunStatus {
                    run_id,
                    failure_summary: Some(summary),
                    ..
                } if *run_id == turn_run
                    && summary == "The run failed because the execution driver stopped unexpectedly. Retry the run, and contact support if it happens again."
            )
        }),
        _ => false,
    }));
}

#[test]
fn bounded_failure_explanation_truncates_at_utf8_boundary() {
    let input = "é".repeat(300);
    let output = bounded_failure_explanation(&input).expect("bounded explanation");

    assert!(output.len() <= 512);
    assert!(output.is_char_boundary(output.len()));
    assert!(output.chars().all(|character| character == 'é'));
}

#[test]
fn bounded_failure_explanation_returns_none_for_empty_or_whitespace_input() {
    assert_eq!(bounded_failure_explanation(""), None);
    assert_eq!(bounded_failure_explanation("   \n\t"), None);
}

#[tokio::test]
async fn model_failure_explainer_returns_bounded_assistant_reply() {
    let gateway = Arc::new(RecordingFailureGateway {
        response: Mutex::new(Ok(SystemInferenceResponse {
            task_id: SystemInferenceTaskId::new(),
            output_text: "The request used an unsupported driver operation, so the run stopped."
                .to_string(),
            elapsed_ms: 1,
        })),
        requests: Mutex::new(Vec::new()),
    });
    let explainer = ModelFailureExplanationProvider::new(gateway.clone());

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
            failure_category: "driver_invalid_request".to_string(),
            fallback_summary: "The run failed because the execution driver rejected the request."
                .to_string(),
        })
        .await;

    assert_eq!(
        explanation.as_deref(),
        Some("The request used an unsupported driver operation, so the run stopped.")
    );
    let requests = gateway.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].input_text.contains("failure_category"));
    assert_eq!(
        requests[0].identity.task_kind,
        SystemTaskKind::FailureExplanation
    );
}

#[tokio::test]
async fn model_failure_explainer_returns_none_when_gateway_fails() {
    let gateway = Arc::new(RecordingFailureGateway {
        response: Mutex::new(Err(SystemInferenceError::Failed {
            safe_summary: LoopSafeSummary::new("model unavailable").unwrap(),
        })),
        requests: Mutex::new(Vec::new()),
    });
    let explainer = ModelFailureExplanationProvider::new(gateway);

    let explanation = explainer
        .explain_failure(FailureExplanationInput {
            failure_category: "driver_unavailable".to_string(),
            fallback_summary: "The run failed because the execution driver was unavailable."
                .to_string(),
        })
        .await;

    assert_eq!(explanation, None);
}

fn loop_failure_kind_as_str_values_from_source() -> std::collections::BTreeSet<&'static str> {
    const SOURCE: &str = include_str!("../../../../ironclaw_turns/src/loop_exit.rs");
    source_match_string_values(SOURCE, "impl LoopFailureKind")
}

fn reborn_failure_category_constant_values_from_source() -> std::collections::BTreeSet<&'static str>
{
    const SOURCE: &str = include_str!("../../../../ironclaw_reborn/src/failure_categories.rs");
    SOURCE
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with("pub const ") || !trimmed.contains("_CATEGORY") {
                return None;
            }
            quoted_value(trimmed)
        })
        .collect()
}

fn source_match_string_values(
    source: &'static str,
    impl_header: &str,
) -> std::collections::BTreeSet<&'static str> {
    let mut in_impl = false;
    let mut values = std::collections::BTreeSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(impl_header) {
            in_impl = true;
            continue;
        }
        if in_impl && trimmed.starts_with("fn to_sanitized_failure") {
            break;
        }
        if in_impl
            && trimmed.starts_with("Self::")
            && trimmed.contains("=>")
            && let Some(value) = quoted_value(trimmed)
        {
            values.insert(value);
        }
    }
    values
}

fn quoted_value(line: &'static str) -> Option<&'static str> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(&line[start..end])
}

/// Collects the safe-summary category strings the agent loop mints, scanning the
/// three category-producing functions in the agent-loop source. Mirrors the
/// source-parity approach used for the Tier-2 and `LoopFailureKind` tables.
fn agent_loop_safe_summary_category_values_from_source() -> std::collections::BTreeSet<&'static str>
{
    const MAPPING: &str = include_str!("../../../../ironclaw_agent_loop/src/executor/mapping.rs");
    const PROMPT: &str = include_str!("../../../../ironclaw_agent_loop/src/executor/prompt.rs");
    let mut values = std::collections::BTreeSet::new();
    values.extend(fn_match_arm_string_values(
        MAPPING,
        "capability_error_failure_category",
    ));
    values.extend(fn_match_arm_string_values(
        MAPPING,
        "model_error_failure_category",
    ));
    values.extend(fn_match_arm_string_values(
        PROMPT,
        "compaction_failure_category",
    ));
    values
}

/// Returns the quoted values of `Variant => "..."` match arms inside the named
/// free function, stopping at the next function definition.
fn fn_match_arm_string_values(
    source: &'static str,
    fn_marker: &str,
) -> std::collections::BTreeSet<&'static str> {
    let mut values = std::collections::BTreeSet::new();
    let mut in_fn = false;
    for line in source.lines() {
        let trimmed = line.trim();
        let is_fn_header = trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub(super) fn ")
            || trimmed.starts_with("pub(crate) fn ");
        if !in_fn {
            if is_fn_header && trimmed.contains(fn_marker) {
                in_fn = true;
            }
            continue;
        }
        if is_fn_header {
            break;
        }
        if trimmed.contains("=> \"")
            && let Some(value) = quoted_value(line)
        {
            values.insert(value);
        }
    }
    values
}
