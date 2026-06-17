use super::*;
use ironclaw_first_party_extension_ports::{
    SkillActivationMode, SkillActivationObservedEvent, SkillActivationRequest,
};
use ironclaw_product_adapters::{
    PROJECTION_SKILL_ACTIVATION_MAX_ITEMS, PROJECTION_SKILL_FEEDBACK_MAX_BYTES,
    PROJECTION_SKILL_NAME_MAX_BYTES, ProductWorkSummaryPhase,
};
use ironclaw_turns::{
    TurnId,
    run_profile::{
        CapabilityInputRef, InMemoryLoopHostMilestoneSink, InMemoryRunProfileResolver,
        LoopDriverId, LoopDriverNoteKind, LoopHostMilestone, LoopHostMilestoneKind, LoopRunContext,
        LoopSafeSummary, RunProfileResolutionRequest, RunProfileResolver,
    },
};

fn preview_input_ref(label: &str) -> CapabilityInputRef {
    CapabilityInputRef::new(format!("input:{label}")).unwrap()
}

#[tokio::test]
async fn webui_event_stream_drains_run_status_projection_from_event_stream_manager() {
    let tenant_id = TenantId::new("webui-events-tenant").unwrap();
    let user_id = UserId::new("webui-events-user").unwrap();
    let agent_id = AgentId::new("webui-events-agent").unwrap();
    let thread_id = ThreadId::new("webui-events-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            ResourceScope {
                tenant_id: tenant_id.clone(),
                user_id: user_id.clone(),
                agent_id: Some(agent_id.clone()),
                project_id: None,
                mission_id: None,
                thread_id: Some(thread_id.clone()),
                invocation_id,
            },
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-events-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    let ProductOutboundPayload::ProjectionSnapshot { state } = events[0].payload() else {
        panic!("expected projection snapshot");
    };
    assert_eq!(state.items.len(), 1);
    assert!(matches!(
        state.items[0],
        ProductProjectionItem::RunStatus { ref status, .. } if status == "running"
    ));
}

#[tokio::test]
async fn webui_event_stream_drains_capability_activity_from_projection() {
    let tenant_id = TenantId::new("webui-activity-tenant").unwrap();
    let user_id = UserId::new("webui-activity-user").unwrap();
    let agent_id = AgentId::new("webui-activity-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-thread").unwrap();
    let invocation_id = InvocationId::new();
    let capability = CapabilityId::new("script.echo").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            capability.clone(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone()),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == invocation_id
                    && activity.thread_id.as_ref() == Some(&thread_id)
                    && activity.capability_id == capability
                    && activity.status == CapabilityActivityStatusView::Started
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_enriches_activity_with_display_preview_from_store() {
    let tenant_id = TenantId::new("webui-preview-tenant").unwrap();
    let user_id = UserId::new("webui-preview-user").unwrap();
    let agent_id = AgentId::new("webui-preview-agent").unwrap();
    let thread_id = ThreadId::new("webui-preview-thread").unwrap();
    let invocation_id = InvocationId::new();
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("webui-preview-input");
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    display_previews.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "src/main.rs",
            "token": "sk-secret",
            "max_bytes": 4096
        }),
    );
    display_previews.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id,
        capability_id: &capability,
        result_ref: "result:preview-output",
        output: &serde_json::json!({"content": "fn main() {}"}),
        output_bytes: 64,
    });
    let timeline_message_id = ironclaw_threads::ThreadMessageId::new();
    let timeline_message_id_string = timeline_message_id.to_string();
    display_previews.attach_timeline_message_id(invocation_id, timeline_message_id);
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            capability.clone(),
            ExtensionId::new("builtin").unwrap(),
            RuntimeKind::FirstParty,
            64,
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-preview-reply").unwrap(),
    )
    .with_display_previews(Arc::clone(&display_previews));
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone()),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityDisplayPreview(preview)
                    if preview.invocation_id == invocation_id
                        && preview.thread_id.as_ref() == Some(&thread_id)
                        && preview.capability_id == capability
                        && preview.title == "read_file"
                        && preview.subtitle.as_deref() == Some("src/main.rs")
                        && preview.input_summary.as_deref().is_some_and(|summary| summary.contains("path: src/main.rs"))
                        && preview.output_preview.as_deref() == Some("fn main() {}")
                        && preview.timeline_message_id.as_deref() == Some(timeline_message_id_string.as_str())
                        && preview.result_ref.as_deref() == Some("result:preview-output")
                        && preview.output_bytes == Some(64)
            )
        }),
        "events: {events:#?}"
    );
    let rendered = serde_json::to_string(&events).unwrap();
    assert!(!rendered.contains("sk-secret"));
}

#[tokio::test]
async fn capability_display_preview_store_redacts_unsafe_paths_and_secrets() {
    let run_id = TurnRunId::new();
    let capability = CapabilityId::new("builtin.read_file").unwrap();
    let input_ref = preview_input_ref("redacted-preview-input");
    let store = CapabilityDisplayPreviewStore::default();
    store.record_input(
        &run_id.to_string(),
        &input_ref,
        "read_file",
        &serde_json::json!({
            "path": "/Users/alice/secret.rs",
            "api_key": "sk-secret"
        }),
    );
    store.record_result(CapabilityDisplayPreviewResult {
        run_id: &run_id.to_string(),
        input_ref: &input_ref,
        invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
        capability_id: &capability,
        result_ref: "result:redacted-preview",
        output: &serde_json::json!({"content": "{\"path\":\"/etc/passwd\", unc:\"\\\\host\\\\share\", token:\"sk-secret\"}"}),
        output_bytes: 42,
    });
    let preview = store
        .preview(&CapabilityActivityProjection {
            invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
            run_id: Some(InvocationId::from_uuid(run_id.as_uuid())),
            capability_id: capability,
            thread_id: Some(ThreadId::new("webui-preview-thread").unwrap()),
            status: ironclaw_event_projections::CapabilityActivityStatus::Completed,
            provider: None,
            runtime: None,
            process_id: None,
            output_bytes: Some(42),
            error_kind: None,
            first_cursor: ironclaw_events::EventCursor::new(1),
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(preview.subtitle.is_none());
    let rendered = serde_json::to_string(&preview).unwrap();
    assert!(!rendered.contains("sk-secret"));
    assert!(!rendered.contains("/Users/alice"));
    assert!(!rendered.contains("/etc/passwd"));
    assert!(!rendered.contains("\\\\host\\\\share"));
    assert!(rendered.contains("[redacted]"));
}

#[tokio::test]
async fn webui_event_stream_replays_capability_started_before_folded_completion() {
    let tenant_id = TenantId::new("webui-activity-replay-tenant").unwrap();
    let user_id = UserId::new("webui-activity-replay-user").unwrap();
    let agent_id = AgentId::new("webui-activity-replay-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-replay-thread").unwrap();
    let run_id = InvocationId::new();
    let capability_invocation = InvocationId::new();
    let capability = CapabilityId::new("script.echo").unwrap();
    let provider = ExtensionId::new("script").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, run_id),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();

    let event_log_dyn: Arc<dyn DurableEventLog> = event_log.clone();
    let actor = TurnActor::new(user_id.clone());
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-activity-replay-reply").unwrap(),
    );
    let initial = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                capability_invocation,
            ),
            capability.clone(),
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                capability_invocation,
            ),
            capability.clone(),
            provider,
            RuntimeKind::Script,
            42,
        ))
        .await
        .unwrap();

    let replayed = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(initial[0].projection_cursor().clone()),
        })
        .await
        .unwrap();

    let statuses = replayed
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == capability_invocation
                    && activity.capability_id == capability =>
            {
                Some(activity.status)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec![
            CapabilityActivityStatusView::Started,
            CapabilityActivityStatusView::Completed,
        ]
    );
}

#[tokio::test]
async fn webui_event_stream_preserves_sanitized_capability_activity_error_kind() {
    let tenant_id = TenantId::new("webui-activity-redacted-tenant").unwrap();
    let user_id = UserId::new("webui-activity-redacted-user").unwrap();
    let agent_id = AgentId::new("webui-activity-redacted-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-redacted-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_failed(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
            Some(ExtensionId::new("script").unwrap()),
            Some(RuntimeKind::Script),
            "raw failure /tmp/private-host-path SECRET_SENTINEL_sk_live",
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-redacted-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == invocation_id
                    && activity.status == CapabilityActivityStatusView::Failed
                    && activity.error_kind.as_deref() == Some("Unclassified")
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_drains_live_reasoning_projection_from_update_source() {
    let tenant_id = TenantId::new("webui-thinking-tenant").unwrap();
    let user_id = UserId::new("webui-thinking-user").unwrap();
    let agent_id = AgentId::new("webui-thinking-agent").unwrap();
    let thread_id = ThreadId::new("webui-thinking-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-thinking-reply").unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );

    let thinking_body = "Thinking Steps • Summary\n\
[] Inspect nearai/ironclaw.\n\
[] Read the thermo-loop SKILL.md fully.\n\
() Find the PR details using gh CLI.\n\
[] Run the thermonuclear code quality review.\n\
! Fix actionable findings.";

    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: thinking_body.to_string(),
        },
    })
    .await
    .unwrap();

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.thread_id == thread_id.to_string()
                    && state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::Thinking { body, .. } if body == thinking_body
                    ))
        )
    }));
}

#[tokio::test]
async fn live_projection_is_keyed_to_run_actor_not_publisher_owner() {
    // A turn run by an SSO user whose id differs from the runtime owner
    // must publish live progress to THAT user's stream, not the operator's.
    // Regression for the projection-owner leak: the publisher used to key
    // every live item to its construction-time owner, so an SSO user never
    // saw their own thinking/progress while it leaked onto the operator
    // stream.
    let tenant_id = TenantId::new("webui-actor-tenant").unwrap();
    let runtime_owner = UserId::new("runtime-owner").unwrap();
    let sso_user = UserId::new("sso-user").unwrap();
    let agent_id = AgentId::new("webui-actor-agent").unwrap();
    let thread_id = ThreadId::new("webui-actor-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-actor-reply").unwrap(),
    );
    // Publisher built with the runtime owner — the fallback owner.
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(runtime_owner.clone()),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );

    // The milestone is bound to the SSO user, not the runtime owner.
    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: Some(TurnActor::new(sso_user.clone())),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: "sso user thinking".to_string(),
        },
    })
    .await
    .unwrap();

    // The SSO user (the run actor) receives their own live progress.
    let sso_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(sso_user.clone()),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    assert!(
        sso_events.iter().any(|event| matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.items.iter().any(|item| matches!(
                    item,
                    ProductProjectionItem::Thinking { body, .. } if body == "sso user thinking"
                ))
        )),
        "the run actor must receive its own live progress"
    );

    // The runtime owner (the old, wrong target) must NOT see it.
    let owner_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(runtime_owner.clone()),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();
    assert!(
        !owner_events.iter().any(|event| matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.items.iter().any(|item| matches!(
                    item,
                    ProductProjectionItem::Thinking { body, .. } if body == "sso user thinking"
                ))
        )),
        "live progress must not leak to a different user's stream"
    );
}

#[tokio::test]
async fn webui_event_stream_drains_skill_activation_projection_from_observer() {
    let tenant_id = TenantId::new("webui-skill-activation-tenant").unwrap();
    let user_id = UserId::new("webui-skill-activation-user").unwrap();
    let agent_id = AgentId::new("webui-skill-activation-agent").unwrap();
    let thread_id = ThreadId::new("webui-skill-activation-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-skill-activation-reply").unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();
    let observer =
        services.skill_activation_observer(services.live_projection_publisher(user_id.clone()));

    observer.observe_skill_activation(SkillActivationObservedEvent {
        run_context: LoopRunContext::new(
            scope.clone(),
            TurnId::new(),
            run_id,
            InMemoryRunProfileResolver::default()
                .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
                .await
                .unwrap(),
        ),
        activations: vec![SkillActivationRequest {
            name: "code-review".to_string(),
            source: None,
            bundle_id: None,
            mode: SkillActivationMode::ExplicitMention,
        }],
        feedback: vec!["code-review: force-activated via explicit mention".to_string()],
    });

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.thread_id == thread_id.to_string()
                    && state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::SkillActivation {
                            run_id: observed_run_id,
                            skill_names,
                            feedback,
                            ..
                        } if *observed_run_id == run_id
                            && skill_names == &vec!["code-review".to_string()]
                            && feedback == &vec![
                                "code-review: force-activated via explicit mention".to_string()
                            ]
                    ))
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_bounds_skill_activation_projection_from_observer() {
    let tenant_id = TenantId::new("webui-skill-bounds-tenant").unwrap();
    let user_id = UserId::new("webui-skill-bounds-user").unwrap();
    let agent_id = AgentId::new("webui-skill-bounds-agent").unwrap();
    let thread_id = ThreadId::new("webui-skill-bounds-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-skill-bounds-reply").unwrap(),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();
    let observer =
        services.skill_activation_observer(services.live_projection_publisher(user_id.clone()));
    let mut activations = (0..=PROJECTION_SKILL_ACTIVATION_MAX_ITEMS)
        .map(|index| SkillActivationRequest {
            name: format!("skill-{index}"),
            source: None,
            bundle_id: None,
            mode: SkillActivationMode::ExplicitMention,
        })
        .collect::<Vec<_>>();
    activations[0].name = format!("skill-{}", "🚀".repeat(80));
    let mut feedback = (0..=PROJECTION_SKILL_ACTIVATION_MAX_ITEMS)
        .map(|index| format!("feedback-{index}"))
        .collect::<Vec<_>>();
    feedback[0] = format!("feedback-{}", "🚀".repeat(300));

    observer.observe_skill_activation(SkillActivationObservedEvent {
        run_context: LoopRunContext::new(
            scope.clone(),
            TurnId::new(),
            run_id,
            InMemoryRunProfileResolver::default()
                .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
                .await
                .unwrap(),
        ),
        activations,
        feedback,
    });

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    let item = events
        .iter()
        .find_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => {
                state.items.iter().find_map(|item| match item {
                    ProductProjectionItem::SkillActivation {
                        skill_names,
                        feedback,
                        ..
                    } => Some((skill_names, feedback)),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("skill activation projection update");

    assert_eq!(item.0.len(), PROJECTION_SKILL_ACTIVATION_MAX_ITEMS);
    assert_eq!(item.1.len(), PROJECTION_SKILL_ACTIVATION_MAX_ITEMS);
    assert!(item.0[0].len() <= PROJECTION_SKILL_NAME_MAX_BYTES);
    assert!(item.1[0].len() <= PROJECTION_SKILL_FEEDBACK_MAX_BYTES);
    assert!(item.0[0].is_char_boundary(item.0[0].len()));
    assert!(item.1[0].is_char_boundary(item.1[0].len()));
    assert!(!item.0.iter().any(|name| name == "skill-16"));
    assert!(!item.1.iter().any(|note| note == "feedback-16"));
}

#[tokio::test]
async fn webui_event_stream_drains_work_summary_projection_from_driver_note() {
    let tenant_id = TenantId::new("webui-work-summary-tenant").unwrap();
    let user_id = UserId::new("webui-work-summary-user").unwrap();
    let agent_id = AgentId::new("webui-work-summary-agent").unwrap();
    let thread_id = ThreadId::new("webui-work-summary-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-work-summary-reply").unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();

    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::DriverNote {
            kind: LoopDriverNoteKind::Planning,
            safe_summary: LoopSafeSummary::new("checking branch state").unwrap(),
        },
    })
    .await
    .unwrap();

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.thread_id == thread_id.to_string()
                    && state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::WorkSummary {
                            run_id: observed_run_id,
                            phase: ProductWorkSummaryPhase::Planning,
                            body,
                            ..
                        } if *observed_run_id == run_id && body == "checking branch state"
                    ))
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_live_cursor_does_not_poison_runtime_failure_resume() {
    let tenant_id = TenantId::new("webui-live-failure-tenant").unwrap();
    let user_id = UserId::new("webui-live-failure-user").unwrap();
    let agent_id = AgentId::new("webui-live-failure-agent").unwrap();
    let thread_id = ThreadId::new("webui-live-failure-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    let event_log_for_append = Arc::clone(&event_log);
    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-live-failure-reply").unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let actor = TurnActor::new(user_id.clone());
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );

    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::from_uuid(invocation_id.as_uuid()),
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::DriverNote {
            kind: LoopDriverNoteKind::Planning,
            safe_summary: LoopSafeSummary::new("checking tools").unwrap(),
        },
    })
    .await
    .unwrap();

    let live_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    assert!(live_events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.items.iter().any(|item| matches!(
                    item,
                    ProductProjectionItem::WorkSummary { body, .. } if body == "checking tools"
                ))
        )
    }));
    let live_cursor =
        parse_webui_projection_cursor(live_events.last().unwrap().projection_cursor().as_str())
            .unwrap();
    assert!(
        live_cursor.runtime.is_none(),
        "live progress must not advance the durable runtime cursor"
    );
    assert!(live_cursor.live.is_some());

    let runtime_scope = resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id);
    event_log_for_append
        .append(RuntimeEvent::model_started(
            runtime_scope.clone(),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();
    event_log_for_append
        .append(RuntimeEvent::loop_failed(
            runtime_scope,
            CapabilityId::new("loop.run").unwrap(),
            "driver_unavailable",
        ))
        .await
        .unwrap();

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(live_events.last().unwrap().projection_cursor().clone()),
        })
        .await
        .unwrap();

    assert!(
        contains_run_status(&resumed_events, invocation_id, "failed"),
        "runtime failure after live progress must still be delivered from the live cursor"
    );
}

#[tokio::test]
async fn webui_event_stream_delivers_prior_completed_activity_before_pending_approval_preview() {
    let tenant_id = TenantId::new("webui-live-pending-preview-tenant").unwrap();
    let user_id = UserId::new("webui-live-pending-preview-user").unwrap();
    let agent_id = AgentId::new("webui-live-pending-preview-agent").unwrap();
    let thread_id = ThreadId::new("webui-live-pending-preview-thread").unwrap();
    let first_extension_invocation = InvocationId::new();
    let second_extension_invocation = InvocationId::new();
    let approval_invocation = InvocationId::new();
    let extension_search = CapabilityId::new("builtin.extension_search").unwrap();
    let web_access_search = CapabilityId::new("web-access.search").unwrap();
    let provider = ExtensionId::new("builtin").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                first_extension_invocation,
            ),
            extension_search.clone(),
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                second_extension_invocation,
            ),
            extension_search.clone(),
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                second_extension_invocation,
            ),
            extension_search.clone(),
            provider.clone(),
            RuntimeKind::FirstParty,
            48,
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                first_extension_invocation,
            ),
            extension_search.clone(),
            provider,
            RuntimeKind::FirstParty,
            32,
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                approval_invocation,
            ),
            web_access_search.clone(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let display_previews = Arc::new(CapabilityDisplayPreviewStore::default());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-live-pending-preview-reply").unwrap(),
    )
    .with_display_previews(display_previews);

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    let activities = events
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::CapabilityActivity(activity) => Some((
                activity.invocation_id,
                activity.capability_id.clone(),
                activity.status,
                activity.activity_order,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        activities,
        vec![
            (
                first_extension_invocation,
                extension_search.clone(),
                CapabilityActivityStatusView::Completed,
                Some(1),
            ),
            (
                second_extension_invocation,
                extension_search,
                CapabilityActivityStatusView::Completed,
                Some(2),
            ),
            (
                approval_invocation,
                web_access_search,
                CapabilityActivityStatusView::Started,
                Some(5),
            ),
        ],
        "a pending approval preview must not hide already completed tool activity"
    );
    let cursor = parse_webui_projection_cursor(
        events
            .last()
            .expect("activity payloads should be delivered")
            .projection_cursor()
            .as_str(),
    )
    .unwrap();
    assert!(cursor.runtime.is_none());
    assert!(cursor.runtime_item.is_some());
    assert_eq!(cursor.runtime_payloads_delivered, 4);
}

#[tokio::test]
async fn webui_event_stream_maps_subscription_terminated_work_summary_to_context() {
    let tenant_id = TenantId::new("webui-terminated-summary-tenant").unwrap();
    let user_id = UserId::new("webui-terminated-summary-user").unwrap();
    let agent_id = AgentId::new("webui-terminated-summary-agent").unwrap();
    let thread_id = ThreadId::new("webui-terminated-summary-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-terminated-summary-reply").unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let run_id = TurnRunId::new();

    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::DriverNote {
            kind: LoopDriverNoteKind::EventSubscriptionTerminated,
            safe_summary: LoopSafeSummary::new("event subscription terminated").unwrap(),
        },
    })
    .await
    .unwrap();

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.thread_id == thread_id.to_string()
                    && state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::WorkSummary {
                            run_id: observed_run_id,
                            phase: ProductWorkSummaryPhase::Context,
                            body,
                            ..
                        } if *observed_run_id == run_id && body == "event subscription terminated"
                    ))
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_skips_empty_work_summary_body() {
    let tenant_id = TenantId::new("webui-empty-summary-tenant").unwrap();
    let user_id = UserId::new("webui-empty-summary-user").unwrap();
    let agent_id = AgentId::new("webui-empty-summary-agent").unwrap();
    let thread_id = ThreadId::new("webui-empty-summary-thread").unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-empty-summary-reply").unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );

    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::DriverNote {
            kind: LoopDriverNoteKind::Planning,
            safe_summary: LoopSafeSummary::new("   ").unwrap(),
        },
    })
    .await
    .unwrap();

    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().all(|event| {
        !matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.items.iter().any(|item| matches!(
                    item,
                    ProductProjectionItem::WorkSummary { .. }
                ))
        )
    }));
}

#[tokio::test]
async fn webui_event_stream_resumes_inside_multi_payload_runtime_projection_item() {
    let tenant_id = TenantId::new("webui-activity-resume-tenant").unwrap();
    let user_id = UserId::new("webui-activity-resume-user").unwrap();
    let agent_id = AgentId::new("webui-activity-resume-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-resume-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-resume-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(initial_events.len(), 2);
    assert!(matches!(
        initial_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(matches!(
        initial_events[1].payload(),
        ProductOutboundPayload::CapabilityActivity(_)
    ));
    let partial_cursor =
        parse_webui_projection_cursor(initial_events[0].projection_cursor().as_str()).unwrap();
    assert!(partial_cursor.runtime.is_none());
    assert!(partial_cursor.runtime_item.is_some());
    assert_eq!(partial_cursor.runtime_payloads_delivered, 1);

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(initial_events[0].projection_cursor().clone()),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 1);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == invocation_id
    ));
    let resumed_cursor =
        parse_webui_projection_cursor(resumed_events[0].projection_cursor().as_str()).unwrap();
    assert!(resumed_cursor.runtime.is_some());
    assert_eq!(resumed_cursor.runtime_payloads_delivered, 0);
}

#[tokio::test]
async fn webui_event_stream_accepts_legacy_partial_origin_cursor() {
    let tenant_id = TenantId::new("webui-activity-legacy-tenant").unwrap();
    let user_id = UserId::new("webui-activity-legacy-user").unwrap();
    let agent_id = AgentId::new("webui-activity-legacy-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-legacy-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("script.echo").unwrap(),
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let legacy_cursor = product_cursor_from_webui_cursor(&WebuiProjectionCursor {
        runtime: None,
        live: None,
        runtime_item: None,
        turn: None,
        runtime_payloads_delivered: 1,
    })
    .unwrap();
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-legacy-reply").unwrap(),
    );

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(legacy_cursor),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 1);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::CapabilityActivity(activity)
            if activity.invocation_id == invocation_id
    ));
}

#[tokio::test]
async fn webui_projection_snapshot_bounds_activity_fanout_before_payload_mapping() {
    let tenant_id = TenantId::new("webui-activity-cap-tenant").unwrap();
    let user_id = UserId::new("webui-activity-cap-user").unwrap();
    let agent_id = AgentId::new("webui-activity-cap-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-cap-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
    let projection_scope = runtime_projection_scope(&actor, &scope);
    let cursor =
        EventProjectionCursor::for_scope(projection_scope, ironclaw_events::EventCursor::new(1));
    let snapshot = ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: Vec::new(),
        },
        runs: vec![RunStatusProjection {
            invocation_id: InvocationId::new(),
            capability_id: capability.clone(),
            thread_id: Some(thread_id.clone()),
            status: RunProjectionStatus::Running,
            provider: None,
            runtime: None,
            process_id: None,
            error_kind: None,
            last_cursor: ironclaw_events::EventCursor::new(1),
            updated_at: chrono::Utc::now(),
        }],
        capability_activities: (0..(WEBUI_PROJECTION_PAGE_LIMIT + 10))
            .map(|index| CapabilityActivityProjection {
                invocation_id: InvocationId::new(),
                run_id: None,
                capability_id: capability.clone(),
                thread_id: Some(thread_id.clone()),
                status: ironclaw_event_projections::CapabilityActivityStatus::Running,
                provider: None,
                runtime: None,
                process_id: None,
                output_bytes: None,
                error_kind: None,
                first_cursor: ironclaw_events::EventCursor::new(index as u64 + 1),
                last_cursor: ironclaw_events::EventCursor::new(index as u64 + 1),
                updated_at: chrono::Utc::now(),
            })
            .collect(),
        next_cursor: cursor.clone(),
        truncated: false,
    };

    let display_previews = NoopCapabilityDisplayPreviewSource;
    let item = runtime_payloads_for_item(
        &scope,
        &display_previews,
        RuntimePayloadItemInput {
            runs: snapshot.runs,
            capability_activities: snapshot.capability_activities,
            cursor: cursor.clone(),
            state_kind: StatePayloadKind::Snapshot,
        },
        None,
        0,
        WEBUI_PROJECTION_PAGE_LIMIT + 11,
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(item.total, WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert_eq!(item.payloads.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert!(matches!(
        &item.payloads[0].payload,
        ProductOutboundPayload::ProjectionSnapshot { state } if state.items.len() == 1
    ));
    assert_eq!(
        item.payloads
            .iter()
            .filter(|payload| matches!(
                payload.payload,
                ProductOutboundPayload::CapabilityActivity(_)
            ))
            .count(),
        WEBUI_PROJECTION_PAGE_LIMIT
    );
}

#[tokio::test]
async fn webui_event_stream_bounds_large_activity_history_before_dto_construction() {
    let tenant_id = TenantId::new("webui-activity-overflow-tenant").unwrap();
    let user_id = UserId::new("webui-activity-overflow-user").unwrap();
    let agent_id = AgentId::new("webui-activity-overflow-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-overflow-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let activity_count = WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 3;
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    for _ in 0..activity_count {
        event_log
            .append(RuntimeEvent::dispatch_requested(
                resource_scope(
                    &tenant_id,
                    &user_id,
                    &agent_id,
                    &thread_id,
                    InvocationId::new(),
                ),
                capability.clone(),
            ))
            .await
            .unwrap();
    }

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-overflow-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(initial_events.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    let initial_cursor = parse_webui_projection_cursor(
        initial_events
            .last()
            .expect("initial event")
            .projection_cursor()
            .as_str(),
    )
    .unwrap();
    assert!(initial_cursor.runtime.is_some());
    assert!(initial_cursor.runtime_item.is_none());
    assert_eq!(initial_cursor.runtime_payloads_delivered, 0);
    assert!(matches!(
        initial_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(
                initial_events
                    .last()
                    .expect("initial event")
                    .projection_cursor()
                    .clone(),
            ),
        })
        .await
        .unwrap();

    assert!(resumed_events.is_empty());
    let emitted_activity_count = initial_events
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityActivity(_)
            )
        })
        .count();
    assert_eq!(emitted_activity_count, WEBUI_PROJECTION_PAGE_LIMIT);

    let final_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(UserId::new("webui-activity-overflow-user").unwrap()),
            scope: TurnScope::new(
                TenantId::new("webui-activity-overflow-tenant").unwrap(),
                Some(AgentId::new("webui-activity-overflow-agent").unwrap()),
                None,
                ThreadId::new("webui-activity-overflow-thread").unwrap(),
            ),
            after_cursor: Some(
                initial_events
                    .last()
                    .expect("initial event")
                    .projection_cursor()
                    .clone(),
            ),
        })
        .await
        .unwrap();
    assert!(final_events.is_empty());
}

#[tokio::test]
async fn webui_event_stream_mints_resumable_cursors_for_long_valid_scope_ids() {
    let tenant_id = TenantId::new(long_test_id("tenant", 't')).unwrap();
    let user_id = UserId::new(long_test_id("user", 'u')).unwrap();
    let agent_id = AgentId::new(long_test_id("agent", 'a')).unwrap();
    let thread_id = ThreadId::new(long_test_id("thread", 'h')).unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    for _ in 0..(WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 1) {
        event_log
            .append(RuntimeEvent::dispatch_requested(
                resource_scope(
                    &tenant_id,
                    &user_id,
                    &agent_id,
                    &thread_id,
                    InvocationId::new(),
                ),
                capability.clone(),
            ))
            .await
            .unwrap();
    }

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-long-scope-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert_eq!(events.len(), WEBUI_RUNTIME_ITEM_MAX_PAYLOADS);
    assert!(
        events
            .iter()
            .all(|event| event.projection_cursor().as_str().len() <= 1024)
    );
}

#[tokio::test]
async fn webui_event_stream_rebases_stale_partial_activity_cursor() {
    let tenant_id = TenantId::new("webui-activity-stale-tenant").unwrap();
    let user_id = UserId::new("webui-activity-stale-user").unwrap();
    let agent_id = AgentId::new("webui-activity-stale-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-stale-thread").unwrap();
    let capability = CapabilityId::new("script.echo").unwrap();
    let initial_invocation = InvocationId::new();
    let newer_invocation = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                initial_invocation,
            ),
            capability.clone(),
        ))
        .await
        .unwrap();

    let event_log_dyn: Arc<dyn DurableEventLog> = event_log.clone();
    let actor = TurnActor::new(user_id.clone());
    let scope = TurnScope::new(
        tenant_id.clone(),
        Some(agent_id.clone()),
        None,
        thread_id.clone(),
    );
    let services = build_reborn_projection_services(
        event_log_dyn,
        ReplyTargetBindingRef::new("webui-activity-stale-reply").unwrap(),
    );
    let initial_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    assert_eq!(initial_events.len(), 2);
    let stale_cursor = initial_events[0].projection_cursor().clone();

    event_log
        .append(RuntimeEvent::dispatch_requested(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                newer_invocation,
            ),
            capability,
        ))
        .await
        .unwrap();

    let resumed_events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(stale_cursor),
        })
        .await
        .unwrap();

    assert_eq!(resumed_events.len(), 3);
    assert!(matches!(
        resumed_events[0].payload(),
        ProductOutboundPayload::ProjectionSnapshot { .. }
    ));
    assert!(resumed_events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == initial_invocation
        )
    }));
    assert!(resumed_events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == newer_invocation
        )
    }));
    let resumed_cursor = parse_webui_projection_cursor(
        resumed_events
            .last()
            .expect("resumed event")
            .projection_cursor()
            .as_str(),
    )
    .unwrap();
    assert!(resumed_cursor.runtime.is_some());
    assert!(resumed_cursor.runtime_item.is_none());
    assert_eq!(resumed_cursor.runtime_payloads_delivered, 0);
}

#[tokio::test]
async fn webui_event_stream_drains_completed_and_failed_capability_activity_metadata() {
    let tenant_id = TenantId::new("webui-activity-terminal-tenant").unwrap();
    let user_id = UserId::new("webui-activity-terminal-user").unwrap();
    let agent_id = AgentId::new("webui-activity-terminal-agent").unwrap();
    let thread_id = ThreadId::new("webui-activity-terminal-thread").unwrap();
    let completed_invocation = InvocationId::new();
    let failed_invocation = InvocationId::new();
    let capability = CapabilityId::new("script.echo").unwrap();
    let provider = ExtensionId::new("script").unwrap();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::dispatch_succeeded(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                completed_invocation,
            ),
            capability.clone(),
            provider.clone(),
            RuntimeKind::Script,
            64,
        ))
        .await
        .unwrap();
    event_log
        .append(RuntimeEvent::dispatch_failed(
            resource_scope(
                &tenant_id,
                &user_id,
                &agent_id,
                &thread_id,
                failed_invocation,
            ),
            capability.clone(),
            Some(provider),
            Some(RuntimeKind::Script),
            "policy_denied",
        ))
        .await
        .unwrap();

    let event_log: Arc<dyn DurableEventLog> = event_log;
    let actor = TurnActor::new(user_id);
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-activity-terminal-reply").unwrap(),
    );
    let events = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope: TurnScope::new(tenant_id, Some(agent_id), None, thread_id),
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == completed_invocation
                    && activity.status == CapabilityActivityStatusView::Completed
                    && activity.output_bytes == Some(64)
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.payload(),
            ProductOutboundPayload::CapabilityActivity(activity)
                if activity.invocation_id == failed_invocation
                    && activity.status == CapabilityActivityStatusView::Failed
                    && activity.error_kind.as_deref() == Some("policy_denied")
        )
    }));
}
