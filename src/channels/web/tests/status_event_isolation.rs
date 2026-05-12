//! Regression tests for cross-tenant SSE/WS status event isolation.
//!
//! Reproduces the report that users in a multi-tenant deployment could
//! see another user's status events (Thinking / ToolStarted / ToolResult /
//! ...). The leak vector was the unscoped global broadcast fallback in
//! `GatewayChannel::send_status` — see `mod.rs::dispatch_status_event`.
//!
//! These tests assert two invariants:
//!
//! 1. **Multi-tenant**: a status event without `metadata.user_id` is
//!    DROPPED — no SSE subscriber receives it. Producers that lose
//!    `user_id` along the way silently fail-closed; the warning surfaces
//!    in logs so the producer can be fixed without exposing tenant data.
//! 2. **Single-tenant**: the same dropped-in-multi-tenant case
//!    falls through to a global broadcast — there is one tenant, one
//!    subscriber population, and the unscoped fan-out is by design.
//!
//! Plus the cross-cutting case: if `metadata.user_id` IS present, the
//! event is delivered ONLY to that user's stream, regardless of mode.

use ironclaw_common::AppEvent;
use tokio_stream::StreamExt;

use crate::channels::web::dispatch_status_event;
use crate::channels::web::sse::SseManager;

/// Sentinel event used to flush past the dispatch under test. SSE
/// streams have no peek/timeout APIs in this test surface, so we always
/// broadcast a Heartbeat after the call and read until we see it. If
/// the event-under-test arrived, it shows up before the heartbeat;
/// if it was dropped, the heartbeat is the first thing we see.
fn flush_sentinel(manager: &SseManager) {
    manager.broadcast(AppEvent::Heartbeat);
}

#[tokio::test]
async fn unscoped_status_event_is_dropped_in_multi_tenant() {
    let manager = SseManager::new();
    let mut alice = Box::pin(
        manager
            .subscribe_raw(Some("alice".to_string()), false)
            .expect("alice subscribe"),
    );
    let mut bob = Box::pin(
        manager
            .subscribe_raw(Some("bob".to_string()), false)
            .expect("bob subscribe"),
    );

    // Producer forgot to include user_id in metadata — this is the bug
    // shape that previously leaked across tenants.
    dispatch_status_event(
        &manager,
        true, // multi_tenant_mode = ON
        None, // no user_id in metadata
        AppEvent::Thinking {
            message: "secret tool reasoning".to_string(),
            thread_id: Some("alice-thread".to_string()),
        },
    );
    flush_sentinel(&manager);

    // Both subscribers receive the heartbeat first because the unscoped
    // Thinking was dropped. If the leak comes back, alice (or bob) will
    // see Thinking before Heartbeat and the assertion below fails.
    let alice_first = alice.next().await.expect("alice receives sentinel");
    assert!(
        matches!(alice_first, AppEvent::Heartbeat),
        "multi-tenant leak: alice received an unscoped event before the heartbeat sentinel: {alice_first:?}"
    );
    let bob_first = bob.next().await.expect("bob receives sentinel");
    assert!(
        matches!(bob_first, AppEvent::Heartbeat),
        "multi-tenant leak: bob received an unscoped event before the heartbeat sentinel: {bob_first:?}"
    );
}

#[tokio::test]
async fn unscoped_status_event_passes_in_single_tenant() {
    let manager = SseManager::new();
    let mut sole = Box::pin(
        manager
            .subscribe_raw(Some("only-user".to_string()), false)
            .expect("subscribe"),
    );

    // In single-tenant mode there is one user and one subscriber
    // population. Unscoped events MUST still reach them or background
    // producers (heartbeat, routines) lose their UI.
    dispatch_status_event(
        &manager,
        false, // multi_tenant_mode = OFF
        None,
        AppEvent::Thinking {
            message: "single-tenant background work".to_string(),
            thread_id: None,
        },
    );

    let event = sole
        .next()
        .await
        .expect("single-tenant subscriber receives unscoped Thinking");
    match event {
        AppEvent::Thinking { message, .. } => {
            assert_eq!(message, "single-tenant background work")
        }
        other => panic!("expected Thinking, got {other:?}"),
    }
}

#[tokio::test]
async fn scoped_status_event_only_reaches_owning_user_in_multi_tenant() {
    let manager = SseManager::new();
    let mut alice = Box::pin(
        manager
            .subscribe_raw(Some("alice".to_string()), false)
            .expect("alice subscribe"),
    );
    let mut bob = Box::pin(
        manager
            .subscribe_raw(Some("bob".to_string()), false)
            .expect("bob subscribe"),
    );

    dispatch_status_event(
        &manager,
        true,
        Some("alice"),
        AppEvent::ToolStarted {
            name: "telegram_send".to_string(),
            detail: Some("alice's tool call".to_string()),
            call_id: Some("call-1".to_string()),
            thread_id: Some("alice-thread".to_string()),
        },
    );
    flush_sentinel(&manager);

    // Alice sees her own scoped event, then the heartbeat.
    let first = alice.next().await.expect("alice receives ToolStarted");
    assert!(
        matches!(&first, AppEvent::ToolStarted { name, .. } if name == "telegram_send"),
        "alice should receive her own ToolStarted, got {first:?}"
    );
    let second = alice.next().await.expect("alice receives heartbeat");
    assert!(matches!(second, AppEvent::Heartbeat));

    // Bob sees only the heartbeat — alice's scoped event was filtered.
    let bob_event = bob.next().await.expect("bob receives heartbeat");
    assert!(
        matches!(bob_event, AppEvent::Heartbeat),
        "cross-tenant leak: bob received alice's ToolStarted: {bob_event:?}"
    );
}

#[tokio::test]
async fn scoped_status_event_routes_in_single_tenant_too() {
    // In single-tenant mode the scoped path still works — it is the
    // happy path for chat send. This guards against a regression that
    // would route every status event through the unscoped fallback.
    let manager = SseManager::new();
    let mut sole = Box::pin(
        manager
            .subscribe_raw(Some("alice".to_string()), false)
            .expect("subscribe"),
    );

    dispatch_status_event(
        &manager,
        false,
        Some("alice"),
        AppEvent::ToolCompleted {
            name: "memory_write".to_string(),
            success: true,
            error: None,
            parameters: None,
            call_id: Some("call-1".to_string()),
            duration_ms: Some(12),
            thread_id: Some("t".to_string()),
        },
    );

    let event = sole
        .next()
        .await
        .expect("subscriber receives ToolCompleted");
    assert!(
        matches!(event, AppEvent::ToolCompleted { name, .. } if name == "memory_write"),
        "single-tenant scoped delivery broke"
    );
}

/// Walk a representative selection of `AppEvent` variants and assert
/// each one is dropped when emitted unscoped in multi-tenant mode.
/// Adding a new status variant should require updating this list — if
/// that is missed, the leak surface grows silently and this is the test
/// that catches it.
///
/// The `_compile_time_appevent_variant_check` helper below pairs with
/// the runtime `leak_candidates` list as a build-time reminder: it
/// exhaustively matches every `AppEvent` variant, so adding a new
/// variant fails the test compilation until the helper is updated.
/// When you update the helper, also extend `leak_candidates` with the
/// new variant so the runtime drop assertion actually exercises it.
/// This is two-step (compile fails → developer adds to both places)
/// rather than fully automatic, but it converts a silent regression
/// into a loud build break.
#[tokio::test]
async fn unscoped_drop_holds_for_every_status_variant_in_multi_tenant() {
    let manager = SseManager::new();
    let mut alice = Box::pin(
        manager
            .subscribe_raw(Some("alice".to_string()), false)
            .expect("subscribe"),
    );

    let leak_candidates = [
        AppEvent::Thinking {
            message: "x".into(),
            thread_id: None,
        },
        AppEvent::ToolStarted {
            name: "x".into(),
            detail: None,
            call_id: None,
            thread_id: None,
        },
        AppEvent::ToolCompleted {
            name: "x".into(),
            success: true,
            error: None,
            parameters: None,
            call_id: None,
            duration_ms: None,
            thread_id: None,
        },
        AppEvent::Status {
            message: "x".into(),
            thread_id: None,
        },
        AppEvent::Response {
            content: "x".into(),
            thread_id: "t".into(),
        },
    ];

    for event in leak_candidates {
        dispatch_status_event(&manager, true, None, event);
    }
    flush_sentinel(&manager);

    let first = alice.next().await.expect("subscriber receives sentinel");
    assert!(
        matches!(first, AppEvent::Heartbeat),
        "multi-tenant leak: at least one unscoped status variant reached the subscriber \
         before the heartbeat sentinel — got {first:?}. If a new AppEvent variant was \
         added to the dispatch path, add it to `leak_candidates` and re-run."
    );
}

/// Compile-time reminder for `unscoped_drop_holds_for_every_status_variant_in_multi_tenant`.
///
/// This function is never called. It exists only so the exhaustive match
/// below fails to compile when a new `AppEvent` variant is added — that
/// failure is the prompt to update both this helper AND the runtime
/// `leak_candidates` list above. Without this, a new variant that bypasses
/// the dispatcher's drop-in-multi-tenant rule could silently ship.
///
/// The match must list every variant explicitly; do not add a `_` arm.
/// `event_type()` in `crates/ironclaw_common/src/event.rs` follows the
/// same pattern for the same reason.
#[allow(dead_code)]
fn _compile_time_appevent_variant_check(e: AppEvent) {
    match e {
        AppEvent::Response { .. }
        | AppEvent::Thinking { .. }
        | AppEvent::ToolStarted { .. }
        | AppEvent::ToolCompleted { .. }
        | AppEvent::ToolResult { .. }
        | AppEvent::StreamChunk { .. }
        | AppEvent::Status { .. }
        | AppEvent::JobStarted { .. }
        | AppEvent::ApprovalNeeded { .. }
        | AppEvent::OnboardingState { .. }
        | AppEvent::GateRequired { .. }
        | AppEvent::GateResolved { .. }
        | AppEvent::Error { .. }
        | AppEvent::Heartbeat
        | AppEvent::JobMessage { .. }
        | AppEvent::JobToolUse { .. }
        | AppEvent::JobToolResult { .. }
        | AppEvent::JobStatus { .. }
        | AppEvent::JobResult { .. }
        | AppEvent::ImageGenerated { .. }
        | AppEvent::Suggestions { .. }
        | AppEvent::TurnCost { .. }
        | AppEvent::SkillActivated { .. }
        | AppEvent::ExtensionStatus { .. }
        | AppEvent::ReasoningUpdate { .. }
        | AppEvent::JobReasoning { .. }
        | AppEvent::ToolResultFull { .. }
        | AppEvent::TurnMetrics { .. }
        | AppEvent::ThreadStateChanged { .. }
        | AppEvent::ChildThreadSpawned { .. }
        | AppEvent::ChildThreadCompleted { .. }
        | AppEvent::MissionThreadSpawned { .. }
        | AppEvent::PlanUpdate { .. }
        | AppEvent::CodeExecuted { .. }
        | AppEvent::Warning { .. }
        | AppEvent::CodeExecutionFailed { .. }
        | AppEvent::LeaseGranted { .. }
        | AppEvent::LeaseRevoked { .. }
        | AppEvent::LeaseExpired { .. }
        | AppEvent::SelfImprovement { .. }
        | AppEvent::OrchestratorRollback { .. } => {}
    }
}
