use super::*;
use ironclaw_host_api::runtime::RuntimeKind;
use ironclaw_turns::{
    CapabilityActivityId, LoopCompletionKind, LoopExitId, TurnId,
    run_profile::{
        InMemoryLoopHostMilestoneSink, LoopDriverId, LoopHostMilestone, LoopHostMilestoneKind,
        LoopHostMilestoneSink, LoopSafeSummary,
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

// A browser keeps its opaque SSE cursor when Chat unmounts during route
// navigation. Live cursors are process-local, however, so a deployment can
// reset the sequence and produce the same numeric cursor again. The restarted
// stream must discard only that stale live floor while preserving the durable
// cursor components; otherwise all new interim text is silently filtered until
// a full page refresh clears the browser cache.
#[tokio::test]
async fn product_event_stream_rebases_live_cursor_from_prior_process_epoch() {
    let tenant_id = TenantId::new("webui-live-restart-tenant").unwrap();
    let user_id = UserId::new("webui-live-restart-user").unwrap();
    let agent_id = AgentId::new("webui-live-restart-agent").unwrap();
    let thread_id = ThreadId::new("webui-live-restart-thread").unwrap();
    let scope = TurnScope::new(tenant_id, Some(agent_id), None, thread_id);
    let actor = TurnActor::new(user_id.clone());
    let event_log: Arc<dyn DurableEventLog> = Arc::new(InMemoryDurableEventLog::new());

    let services_before_restart = build_reborn_projection_services(
        Arc::clone(&event_log),
        ReplyTargetBindingRef::new("webui-live-restart-before").unwrap(),
    );
    let sink_before_restart = services_before_restart
        .with_live_progress_milestone_sink_for_publisher(
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            services_before_restart.live_projection_publisher(user_id.clone()),
        );
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
    sink_before_restart
        .publish_loop_milestone(reasoning("before restart"))
        .await
        .unwrap();
    let before_restart = services_before_restart
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: actor.clone(),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();
    let cached_cursor = before_restart
        .iter()
        .find(|event| {
            matches!(
                event.payload(),
                ProductOutboundPayload::ProjectionUpdate { state }
                    if state.items.iter().any(|item| matches!(
                        item,
                        ProductProjectionItem::Thinking { body, .. }
                            if body == "before restart"
                    ))
            )
        })
        .expect("first process must emit live reasoning")
        .projection_cursor()
        .clone();

    // A fresh services bundle models a new server process: it shares the
    // durable log but owns a new live-update source, sequence, and epoch.
    let services_after_restart = build_reborn_projection_services(
        event_log,
        ReplyTargetBindingRef::new("webui-live-restart-after").unwrap(),
    );
    let sink_after_restart = services_after_restart
        .with_live_progress_milestone_sink_for_publisher(
            Arc::new(InMemoryLoopHostMilestoneSink::default()),
            services_after_restart.live_projection_publisher(user_id),
        );
    sink_after_restart
        .publish_loop_milestone(reasoning("after restart"))
        .await
        .unwrap();

    let resumed = services_after_restart
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor: Some(cached_cursor),
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
                        ProductProjectionItem::Thinking { body, .. }
                            if body == "after restart"
                    ))
            )
        }),
        "a stale process-local live cursor must not suppress the restarted process's updates: {resumed:#?}"
    );
}

#[tokio::test]
async fn product_event_stream_drains_live_reasoning_projection_from_update_source() {
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
        .product_event_stream()
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
async fn fresh_product_event_stream_compacts_buffered_assistant_text_to_latest_state() {
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
        .product_event_stream()
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
        vec!["partial answer with [redacted]".to_string()],
        "a fresh route attach must hydrate the latest text state without replaying its growth history"
    );
    let wire = serde_json::to_string(&events).unwrap();
    assert!(!wire.contains(secret_like_token));
}

#[tokio::test]
async fn fresh_product_event_stream_preserves_text_phases_and_clears_terminal_run() {
    let fixture = live_projection_fixture("webui-text-phases");
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let milestone = |kind| LoopHostMilestone {
        scope: scope.clone(),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        loop_driver_id: LoopDriverId::new("test_loop").unwrap(),
        kind,
    };

    for kind in [
        LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        },
        LoopHostMilestoneKind::ModelTextDelta {
            safe_text: "I’ll research".to_string(),
        },
        LoopHostMilestoneKind::ModelTextDelta {
            safe_text: "I’ll research this first.".to_string(),
        },
        LoopHostMilestoneKind::ModelCompleted {
            effective_model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new(
                "test-model",
            )
            .unwrap(),
        },
        LoopHostMilestoneKind::ModelStarted {
            requested_model_profile_id: None,
        },
        LoopHostMilestoneKind::ModelTextDelta {
            safe_text: "Here is the final answer.".to_string(),
        },
        LoopHostMilestoneKind::Completed {
            completion_kind: LoopCompletionKind::FinalReply,
            exit_id: LoopExitId::new("exit:webui-text-phases").unwrap(),
        },
        LoopHostMilestoneKind::ModelTextDelta {
            safe_text: "Unexpected trailing text.".to_string(),
        },
    ] {
        fixture
            .sink
            .publish_loop_milestone(milestone(kind))
            .await
            .unwrap();
    }

    let events = fixture
        .services
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(fixture.user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();
    let text_items = events
        .iter()
        .flat_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => state
                .items
                .iter()
                .filter_map(|item| match item {
                    ProductProjectionItem::Text {
                        id,
                        run_id: observed_run_id,
                        body,
                    } if *observed_run_id == Some(run_id) => Some((id.clone(), body.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        text_items,
        vec![
            (
                format!("text:{run_id}:1"),
                "I’ll research this first.".to_string()
            ),
            (
                format!("text:{run_id}:2"),
                "Here is the final answer.".to_string()
            ),
            (
                format!("text:{run_id}"),
                "Unexpected trailing text.".to_string()
            ),
        ],
        "model phases must remain distinct and terminal milestones must clear phase state"
    );
}

#[tokio::test]
async fn provider_cadence_text_updates_are_not_visibly_batched() {
    let fixture = live_projection_fixture("webui-text-cadence");
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let activity_id = CapabilityActivityId::new();
    let mut subscription = fixture
        .services
        .product_event_stream()
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

    for body in ["first", "second", "third"] {
        fixture
            .sink
            .publish_loop_milestone(milestone(LoopHostMilestoneKind::ModelTextDelta {
                safe_text: body.to_string(),
            }))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    fixture
        .sink
        .publish_loop_milestone(milestone(LoopHostMilestoneKind::CapabilityInvoked {
            activity_id,
            capability_id,
        }))
        .await
        .unwrap();

    let mut text_bodies = Vec::new();
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
                    assert_eq!(text_bodies, ["first", "second", "third"]);
                    return;
                }
                _ => {}
            }
        }
    }

    panic!("tool activity did not follow provider-cadence text");
}

#[tokio::test]
async fn live_text_microburst_keeps_latest_snapshot_and_precedes_tool_activity() {
    let fixture = live_projection_fixture("webui-text-burst");
    let scope = fixture.scope.clone();
    let run_id = TurnRunId::new();
    let capability_id = CapabilityId::new("builtin.http").unwrap();
    let activity_id = CapabilityActivityId::new();
    let mut subscription = fixture
        .services
        .product_event_stream()
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
        "the latest cumulative assistant text must precede tool activity"
    );
    assert!(
        text_bodies.len() <= 3,
        "the 64-update microburst should keep only paint-relevant cumulative snapshots: {text_bodies:#?}"
    );
}

// The post-run skill-learning notifier publishes a learned-skill bubble
// through a `LiveProjectionPublisher` that shares the runtime's live update
// source. This guards that such a bubble actually drains to the WebUI
// projection stream as a `SkillActivation` item (regression: the live
// `SkillActivation` envelope was silently dropped before reaching the SSE
// drain, so users never saw "learned a skill" feedback).
#[tokio::test]
async fn product_event_stream_drains_skill_learned_projection_from_update_source() {
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
        .product_event_stream()
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
        .product_event_stream()
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
        .product_event_stream()
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
        .product_event_stream()
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
    let mut subscription = fixture
        .services
        .product_event_stream()
        .subscribe(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id.clone()),
            scope: scope.clone(),
            after_cursor: None,
        })
        .await
        .unwrap();

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

    let mut thinking_cursors = Vec::new();
    for _ in 0..4 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), subscription.next())
            .await
            .expect("live projection event")
            .expect("live projection subscription remains open")
            .expect("live projection event remains valid");
        if matches!(
            event.payload(),
            ProductOutboundPayload::ProjectionUpdate { state }
                if state.items.iter().any(|item| matches!(
                    item,
                    ProductProjectionItem::Thinking { body, .. }
                        if body == "from publisher A" || body == "from publisher B"
                ))
        ) {
            thinking_cursors.push(event.projection_cursor().clone());
        }
        if thinking_cursors.len() == 2 {
            break;
        }
    }

    assert_eq!(
        thinking_cursors.len(),
        2,
        "both publishers' live reasoning items must reach the stream on their \
         own cursor"
    );
    assert_ne!(
        thinking_cursors[0], thinking_cursors[1],
        "independently created publishers must share one monotonic sequence, so \
         their live items land on distinct projection cursors"
    );
}

#[tokio::test]
async fn product_event_stream_preserves_live_reasoning_and_tool_start_order() {
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
        .product_event_stream()
        .drain(ProjectionSubscriptionRequest {
            actor: TurnActor::new(user_id),
            scope,
            after_cursor: None,
        })
        .await
        .unwrap();

    let live_items = events
        .iter()
        .flat_map(|event| match event.payload() {
            ProductOutboundPayload::ProjectionUpdate { state } => state.items.iter(),
            _ => [].iter(),
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
async fn product_event_stream_projects_live_tool_failure() {
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
                reason_kind: ironclaw_host_api::result_meta::FailureKind::InputEncode,
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
        .product_event_stream()
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
    // Unified FailureKind wire tag: the retired "invalid_input" tag is now
    // "input_encode" (from_tag still accepts the historical spelling).
    assert_eq!(activity.error_kind.as_deref(), Some("input_encode"));
    // Regression: the sanitized failure summary on the milestone must reach the
    // live activity view's `error_detail`, so the per-tool UI card shows the
    // real reason instead of only the bare kind.
    assert_eq!(
        activity.error_detail.as_deref(),
        Some("invalid JSON: expected value at line 1")
    );
}

#[tokio::test]
async fn product_event_stream_redacts_live_tool_failure_filename_detail() {
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
                reason_kind: ironclaw_host_api::result_meta::FailureKind::OperationFailed,
                safe_summary: Some(
                    LoopSafeSummary::new("failed to read AGENTS.md").expect("safe summary"),
                ),
            },
        })
        .await
        .unwrap();

    let events = fixture
        .services
        .product_event_stream()
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
async fn product_event_stream_preserves_redacted_loop_safe_failure_detail() {
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
                reason_kind: ironclaw_host_api::result_meta::FailureKind::OperationFailed,
                safe_summary: Some(LoopSafeSummary::capability_failure_summary(
                    "provider returned ghp_live_secret",
                )),
            },
        })
        .await
        .unwrap();

    let events = fixture
        .services
        .product_event_stream()
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
