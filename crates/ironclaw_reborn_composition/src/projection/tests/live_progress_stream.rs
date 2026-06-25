use super::*;
use ironclaw_host_api::RuntimeKind;
use ironclaw_turns::{
    CapabilityActivityId, TurnId,
    run_profile::{
        CapabilityFailureKind, InMemoryLoopHostMilestoneSink, LoopDriverId, LoopHostMilestone,
        LoopHostMilestoneKind, LoopHostMilestoneSink,
    },
};
use std::sync::Arc;

struct LiveProjectionFixture {
    user_id: UserId,
    thread_id: ThreadId,
    scope: TurnScope,
    services: RebornProjectionServices,
    sink: Arc<dyn LoopHostMilestoneSink>,
}

fn live_projection_fixture(label: &str) -> LiveProjectionFixture {
    let tenant_id = TenantId::new(format!("{label}-tenant")).unwrap();
    let user_id = UserId::new(format!("{label}-user")).unwrap();
    let agent_id = AgentId::new(format!("{label}-agent")).unwrap();
    let thread_id = ThreadId::new(format!("{label}-thread")).unwrap();
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new(format!("{label}-reply")).unwrap(),
    );
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
    LiveProjectionFixture {
        user_id,
        thread_id,
        scope,
        services,
        sink,
    }
}

#[tokio::test]
async fn webui_event_stream_drains_live_reasoning_projection_from_update_source() {
    let fixture = live_projection_fixture("webui-thinking");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();

    let thinking_body = "Thinking Steps • Summary\n\
[] Inspect nearai/ironclaw.\n\
[] Read the thermo-loop SKILL.md fully.\n\
() Find the PR details using gh CLI.\n\
[] Run the thermonuclear code quality review.\n\
! Fix actionable findings.";

    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
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

    let events = fixture
        .services
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

// The post-run skill-learning notifier publishes a learned-skill bubble
// through a `LiveProjectionPublisher` that shares the runtime's live update
// source. This guards that such a bubble actually drains to the WebUI
// projection stream as a `SkillActivation` item (regression: the live
// `SkillActivation` envelope was silently dropped before reaching the SSE
// drain, so users never saw "learned a skill" feedback).
#[cfg(feature = "root-llm-provider")]
#[tokio::test]
async fn webui_event_stream_drains_skill_learned_projection_from_update_source() {
    let fixture = live_projection_fixture("webui-skill-learned");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();

    let publisher = fixture.services.live_projection_publisher(user_id.clone());
    publisher.publish_skill_learned(
        Some(&user_id),
        &scope,
        TurnRunId::new(),
        "file-character-count-roundtrip",
        "I picked this up from the task; it'll speed up similar work next time.",
    );

    let events = fixture
        .services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    assert!(
        events.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::ProjectionUpdate { state }
                    if state.thread_id == thread_id.to_string()
                        && state.items.iter().any(|item| matches!(
                            item,
                            ProductProjectionItem::SkillActivation { skill_names, .. }
                                if skill_names.iter().any(|name| name == "file-character-count-roundtrip")
                        ))
            )
        }),
        "post-run learned-skill bubble must drain to the WebUI projection stream"
    );
}

// Faithful reproduction of the PRODUCTION flow that broke: a run streams
// durable progress (advancing the runtime cursor) and completes; only
// AFTERWARD does the post-run skill-learning sink publish the learned-skill
// bubble. The open SSE stream resumes draining from the advanced durable
// cursor, so the bubble must still be delivered from that resume point — not
// only on a fresh `after_cursor: None` subscription. The earlier
// `*_from_update_source` test (publish-then-fresh-drain) passed while real
// users still saw nothing, because it never exercised the resume path.
#[cfg(feature = "root-llm-provider")]
#[tokio::test]
async fn skill_learned_bubble_delivers_when_sse_resumes_from_advanced_durable_cursor() {
    let tenant_id = TenantId::new("skill-resume-tenant").unwrap();
    let user_id = UserId::new("skill-resume-user").unwrap();
    let agent_id = AgentId::new("skill-resume-agent").unwrap();
    let thread_id = ThreadId::new("skill-resume-thread").unwrap();
    let invocation_id = InvocationId::new();
    let event_log = Arc::new(InMemoryDurableEventLog::new());
    event_log
        .append(RuntimeEvent::model_started(
            resource_scope(&tenant_id, &user_id, &agent_id, &thread_id, invocation_id),
            CapabilityId::new("loop.model").unwrap(),
        ))
        .await
        .unwrap();
    let event_log: Arc<dyn DurableEventLog> = event_log;
    let services = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("skill-resume-reply").unwrap(),
    );
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id.clone());
    let actor = TurnActor::new(user_id.clone());

    // 1. Initial drain consumes the durable run-status snapshot and advances
    //    the runtime cursor — exactly what the SSE handler does while the run
    //    is executing.
    let initial = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    let resume_cursor = initial
        .last()
        .expect("durable snapshot")
        .projection_cursor()
        .clone();

    // 2. A prior live reasoning update advances the live cursor on the same
    //    still-open SSE stream. This uses the production milestone-sink caller,
    //    not a projection helper.
    let sink = services.with_live_progress_milestone_sink_for_publisher(
        Arc::new(InMemoryLoopHostMilestoneSink::default()),
        services.live_projection_publisher(user_id.clone()),
    );
    sink.publish_loop_milestone(LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id: TurnRunId::from_uuid(invocation_id.as_uuid()),
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: "checking whether this task taught a reusable workflow".to_string(),
        },
    })
    .await
    .unwrap();

    // 3. The still-open SSE stream resumes from the advanced durable cursor and
    //    receives the prior live reasoning item, advancing the live cursor.
    let live_progress = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: Some(resume_cursor),
        })
        .await
        .unwrap();
    assert!(
        live_progress.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::ProjectionUpdate { state }
                    if state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::Thinking { body, .. }
                            if body.contains("checking whether this task taught a reusable workflow")
                    ))
            )
        }),
        "live reasoning must deliver when SSE resumes from an advanced durable cursor: {live_progress:#?}"
    );
    let live_resume_cursor = live_progress
        .last()
        .expect("live reasoning event")
        .projection_cursor()
        .clone();

    // 4. Post-run, ~seconds later, the skill-learning sink publishes through a
    //    fresh publisher (with its own live sequence) and must still deliver
    //    when the client resumes from the advanced live cursor.
    let publisher = services.live_projection_publisher(user_id.clone());
    publisher.publish_skill_learned(
        Some(&user_id),
        &scope,
        TurnRunId::from_uuid(invocation_id.as_uuid()),
        "file-character-count-roundtrip",
        "Learned from the run; speeds up similar work next time.",
    );

    let resumed = services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(live_resume_cursor),
        })
        .await
        .unwrap();

    assert!(
        resumed.iter().any(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::ProjectionUpdate { state }
                    if state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::SkillActivation { skill_names, .. }
                            if skill_names.iter().any(|name| name == "file-character-count-roundtrip")
                    ))
            )
        }),
        "learned-skill bubble must deliver when SSE resumes from an advanced live cursor: {resumed:#?}"
    );
}

#[tokio::test]
async fn webui_event_stream_preserves_live_reasoning_and_tool_start_order() {
    let fixture = live_projection_fixture("webui-live-order");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let activity_id = CapabilityActivityId::new();
    let milestone_base = || LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: String::new(),
        },
    };

    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
            kind: LoopHostMilestoneKind::ModelReasoningDelta {
                safe_delta: "before tool".to_string(),
            },
            ..milestone_base()
        })
        .await
        .unwrap();
    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
            kind: LoopHostMilestoneKind::CapabilityInvoked {
                activity_id,
                capability_id: capability_id.clone(),
            },
            ..milestone_base()
        })
        .await
        .unwrap();
    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
            kind: LoopHostMilestoneKind::ModelReasoningDelta {
                safe_delta: "after tool".to_string(),
            },
            ..milestone_base()
        })
        .await
        .unwrap();

    let events = fixture
        .services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    let live_items = events
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => state.items.first(),
            _ => None,
        })
        .map(|item| match item {
            ProductProjectionItem::Thinking { body, .. } => format!("thinking:{body}"),
            ProductProjectionItem::CapabilityActivity(activity) => {
                assert_eq!(
                    activity.invocation_id,
                    InvocationId::from_uuid(activity_id.as_uuid())
                );
                assert_eq!(activity.thread_id.as_ref(), Some(&thread_id));
                assert_eq!(&activity.capability_id, &capability_id);
                assert_eq!(activity.status, CapabilityActivityStatusView::Started);
                "tool:builtin.http".to_string()
            }
            other => panic!("unexpected live item: {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        live_items,
        vec![
            "thinking:before tool".to_string(),
            "tool:builtin.http".to_string(),
            "thinking:after tool".to_string(),
        ]
    );
}

#[tokio::test]
async fn webui_event_stream_projects_live_tool_failure() {
    let fixture = live_projection_fixture("webui-live-tool-failed");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("nearai.web_search").unwrap();
    let activity_id = CapabilityActivityId::new();

    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
            scope: scope.clone(),
            actor: None,
            turn_id: TurnId::new(),
            run_id,
            loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
            kind: LoopHostMilestoneKind::CapabilityFailed {
                activity_id,
                capability_id: capability_id.clone(),
                provider: None,
                runtime: Some(RuntimeKind::FirstParty),
                reason_kind: CapabilityFailureKind::Authorization,
            },
        })
        .await
        .unwrap();

    let events = fixture
        .services
        .webui_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    let activity = events
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => Some(state.items.iter()),
            _ => None,
        })
        .flatten()
        .find_map(|item| match item {
            ProductProjectionItem::CapabilityActivity(activity) => Some(activity),
            _ => None,
        })
        .expect("live failed activity");

    assert_eq!(
        activity.invocation_id,
        InvocationId::from_uuid(activity_id.as_uuid())
    );
    assert_eq!(activity.thread_id.as_ref(), Some(&thread_id));
    assert_eq!(&activity.capability_id, &capability_id);
    assert_eq!(activity.status, CapabilityActivityStatusView::Failed);
    assert_eq!(activity.runtime.as_ref(), Some(&RuntimeKind::FirstParty));
    assert_eq!(activity.error_kind.as_deref(), Some("authorization"));
}
