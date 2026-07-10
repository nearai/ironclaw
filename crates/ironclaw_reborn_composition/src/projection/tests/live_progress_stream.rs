use super::*;
use ironclaw_host_api::RuntimeKind;
use ironclaw_turns::{
    CapabilityActivityId, TurnId,
    run_profile::{
        CapabilityFailureKind, InMemoryLoopHostMilestoneSink, LoopDriverId, LoopHostMilestone,
        LoopHostMilestoneKind, LoopHostMilestoneSink, LoopSafeSummary,
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

#[tokio::test]
async fn webui_event_stream_drains_live_assistant_text_projection_from_update_source() {
    let fixture = live_projection_fixture("webui-text");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let secret_like_token = "sk-proj-abcdefghijklmnopqrstuvwxyz123456";

    for safe_text in [
        "partial answer".to_string(),
        format!("partial answer with {secret_like_token}"),
    ] {
        fixture
            .sink
            .publish_loop_milestone(LoopHostMilestone {
                scope: scope.clone(),
                actor: None,
                turn_id: TurnId::new(),
                run_id,
                loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
                kind: LoopHostMilestoneKind::ModelTextDelta { safe_text },
            })
            .await
            .unwrap();
    }
    fixture
        .sink
        .publish_loop_milestone(LoopHostMilestone {
            scope: scope.clone(),
            actor: None,
            turn_id: TurnId::new(),
            run_id,
            loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
            kind: LoopHostMilestoneKind::CapabilityInvoked {
                activity_id: CapabilityActivityId::new(),
                capability_id: CapabilityId::new("builtin.test").unwrap(),
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
    let expected_id = format!("text:{run_id}");
    let text_bodies = events
        .iter()
        .filter_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.thread_id == thread_id.to_string() =>
            {
                state.items.iter().find_map(|item| match item {
                    ProductProjectionItem::Text {
                        id,
                        run_id: observed_run_id,
                        body,
                    } if id == &expected_id && *observed_run_id == Some(run_id) => {
                        Some(body.clone())
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        text_bodies,
        vec![
            "partial answer".to_string(),
            "partial answer with [redacted]".to_string()
        ],
        "assistant text should drain as incremental updates to one stable text item"
    );
    let wire = serde_json::to_string(&events).unwrap();
    assert!(!wire.contains(secret_like_token));
}

#[tokio::test]
async fn live_assistant_text_coalescer_flushes_latest_update_on_timer() {
    let fixture = live_projection_fixture("webui-text-timer");
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let mut subscription = fixture
        .services
        .webui_event_stream()
        .subscribe(ProjectionSubscriptionRequest {
            actor: TurnActor::new(fixture.user_id.clone()),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    let milestone = |safe_text| LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelTextDelta { safe_text },
    };

    fixture
        .sink
        .publish_loop_milestone(milestone("first".to_string()))
        .await
        .unwrap();
    fixture
        .sink
        .publish_loop_milestone(milestone("latest".to_string()))
        .await
        .unwrap();

    let mut text_bodies = Vec::new();
    for _ in 0..2 {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(1), subscription.next())
            .await
            .expect("coalesced live projection event")
            .expect("live projection subscription remains open")
            .expect("live projection event remains valid");
        let ProductOutboundPayload::ProjectionUpdate { state } = envelope.payload() else {
            continue;
        };
        text_bodies.extend(state.items.iter().filter_map(|item| match item {
            ProductProjectionItem::Text {
                run_id: observed_run_id,
                body,
                ..
            } if *observed_run_id == Some(run_id) => Some(body.clone()),
            _ => None,
        }));
    }

    assert_eq!(text_bodies, vec!["first", "latest"]);
}

#[tokio::test]
async fn live_assistant_text_burst_stays_subscribed_and_flushes_before_tool_activity() {
    let fixture = live_projection_fixture("webui-text-burst");
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let activity_id = CapabilityActivityId::new();
    let mut subscription = fixture
        .services
        .webui_event_stream()
        .subscribe(ProjectionSubscriptionRequest {
            actor: TurnActor::new(fixture.user_id.clone()),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

    let milestone = |kind| LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind,
    };

    for index in 0..64 {
        fixture
            .sink
            .publish_loop_milestone(milestone(LoopHostMilestoneKind::ModelTextDelta {
                safe_text: format!("partial answer {index}"),
            }))
            .await
            .unwrap();
    }
    fixture
        .sink
        .publish_loop_milestone(milestone(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id: capability_id.clone(),
        }))
        .await
        .unwrap();

    let mut text_bodies = Vec::new();
    let mut saw_tool = false;
    let mut latest_text_preceded_tool = false;
    for _ in 0..8 {
        let envelope = tokio::time::timeout(std::time::Duration::from_secs(1), subscription.next())
            .await
            .expect("live projection event")
            .expect("live projection subscription remains open")
            .expect("live projection event remains valid");

        let ProductOutboundPayload::ProjectionUpdate { state } = envelope.payload() else {
            continue;
        };
        for item in &state.items {
            match item {
                ProductProjectionItem::Text {
                    run_id: observed_run_id,
                    body,
                    ..
                } if *observed_run_id == Some(run_id) => text_bodies.push(body.clone()),
                ProductProjectionItem::CapabilityActivity(activity)
                    if activity.invocation_id == InvocationId::from_uuid(activity_id.as_uuid()) =>
                {
                    latest_text_preceded_tool =
                        text_bodies.last().map(String::as_str) == Some("partial answer 63");
                    saw_tool = true;
                }
                _ => {}
            }
        }
        if saw_tool {
            break;
        }
    }

    assert!(
        saw_tool,
        "the text burst must not terminate the live subscription"
    );
    assert!(
        latest_text_preceded_tool,
        "releasing the state lock must not reorder the latest text after tool activity"
    );
    assert_eq!(
        text_bodies.last().map(String::as_str),
        Some("partial answer 63"),
        "the tool boundary must flush the latest cumulative assistant text"
    );
    assert!(
        text_bodies.len() <= 3,
        "the 64-update burst should be coalesced before delivery: {text_bodies:#?}"
    );
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

// Regression: multiple `LiveProjectionPublisher` instances created from the
// same `RebornProjectionServices` over a run's lifetime (e.g. the milestone
// sink's publisher plus the post-run skill-learning publisher, created seconds
// later) must SHARE one monotonic live sequence counter. If each publisher
// owned its own counter, two live items published by different publishers would
// collide on the same projection cursor (both starting at sequence 1), and an
// SSE client resuming from the first item's cursor would silently skip the
// second. Guards the shared `Arc<AtomicU64>` wiring across
// `build_reborn_projection_services` and `live_projection_publisher` — a
// revert to a per-publisher `AtomicU64::new(0)` passes every other live-progress
// test but fails this one.
#[tokio::test]
async fn live_publishers_from_same_services_share_monotonic_sequence() {
    let fixture = live_projection_fixture("webui-shared-sequence");
    let user_id = fixture.user_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();

    let reasoning = |body: &str| LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind: LoopHostMilestoneKind::ModelReasoningDelta {
            safe_delta: body.to_string(),
        },
    };

    // Publisher A (the fixture's) emits one live reasoning item.
    fixture
        .sink
        .publish_loop_milestone(reasoning("from publisher A"))
        .await
        .unwrap();

    // A second, independently created publisher emits another. In production
    // this is a fresh publisher minted later in the run's lifetime.
    let sink_b = fixture
        .services
        .with_live_progress_milestone_sink_for_publisher(
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            fixture.services.live_projection_publisher(user_id.clone()),
        );
    sink_b
        .publish_loop_milestone(reasoning("from publisher B"))
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

    let thinking_cursors = events
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::ProjectionUpdate { state }
                    if state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::Thinking { body, .. }
                            if body == "from publisher A" || body == "from publisher B"
                    ))
            )
        })
        .map(|event| event.projection_cursor().clone())
        .collect::<Vec<_>>();

    assert_eq!(
        thinking_cursors.len(),
        2,
        "both publishers' live reasoning items must reach the stream on their \
         own cursor: {events:#?}"
    );
    assert_ne!(
        thinking_cursors[0], thinking_cursors[1],
        "independently created publishers must share one monotonic sequence, so \
         their live items land on distinct projection cursors"
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
                reason_kind: CapabilityFailureKind::InvalidInput,
                safe_summary: Some(
                    LoopSafeSummary::new("invalid JSON: expected value at line 1")
                        .expect("safe summary"),
                ),
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
    assert_eq!(activity.error_kind.as_deref(), Some("invalid_input"));
    // Regression: the sanitized failure summary on the milestone must reach the
    // live activity view's `error_detail`, so the per-tool UI card shows the
    // real reason instead of only the bare kind.
    assert_eq!(
        activity.error_detail.as_deref(),
        Some("invalid JSON: expected value at line 1")
    );
}

#[tokio::test]
async fn webui_event_stream_redacts_live_tool_failure_filename_detail() {
    let fixture = live_projection_fixture("webui-live-tool-failed-redacted");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.read_file").unwrap();
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
                reason_kind: CapabilityFailureKind::OperationFailed,
                safe_summary: Some(
                    LoopSafeSummary::new("failed to read AGENTS.md").expect("safe summary"),
                ),
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
    assert_eq!(activity.error_kind.as_deref(), Some("operation_failed"));
    assert_eq!(
        activity.error_detail.as_deref(),
        Some("can't access your workspace file")
    );
}

#[tokio::test]
async fn webui_event_stream_preserves_redacted_loop_safe_failure_detail() {
    let fixture = live_projection_fixture("webui-live-tool-failed-redacted-safe-summary");
    let user_id = fixture.user_id.clone();
    let thread_id = fixture.thread_id.clone();
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
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
                reason_kind: CapabilityFailureKind::OperationFailed,
                safe_summary: Some(LoopSafeSummary::capability_failure_summary(
                    "provider returned ghp_live_secret",
                )),
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
    assert_eq!(activity.error_kind.as_deref(), Some("operation_failed"));
    assert_eq!(
        activity.error_detail.as_deref(),
        Some("the tool failure details were redacted")
    );
}
