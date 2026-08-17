//! Behavior pins for `TurnCoordinator::prepare_turn` reservations.
//!
//! The prepared-run-id path (`prepare_turn` → `submit_turn { requested_run_id }`)
//! had no direct coverage: no production caller invokes `prepare_turn` today,
//! and none of the reservation semantics — consume-once, the cross-scope
//! `Unauthorized` rejection, the child-run exemption, `abort_prepared_turn`,
//! the capacity cap — were pinned. Unbound turns lean on `submit_turn`
//! admission, so these semantics are pinned FIRST, against unmodified code.

use chrono::Utc;
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_turns::{
    AcceptedMessageRef, DefaultTurnCoordinator, IdempotencyKey, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCapacityResource, TurnCoordinator, TurnError, TurnRunId,
    TurnScope, test_support::in_memory_agent_turn_runtime,
};
use std::sync::Arc;

fn scope(label: &str, thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new(format!("tenant-{label}")).expect("tenant"),
        Some(AgentId::new(format!("agent-{label}")).expect("agent")),
        Some(ProjectId::new(format!("project-{label}")).expect("project")),
        ThreadId::new(thread).expect("thread"),
    )
}

fn coordinator() -> impl TurnCoordinator {
    DefaultTurnCoordinator::new(Arc::new(in_memory_agent_turn_runtime()))
}

fn submit_request(
    scope: TurnScope,
    requested_run_id: Option<TurnRunId>,
    idempotency_key: &str,
) -> SubmitTurnRequest {
    SubmitTurnRequest {
        scope,
        actor: TurnActor::new(UserId::new("user-prepared-run").expect("user")),
        accepted_message_ref: AcceptedMessageRef::new("accepted-prepared-run").expect("accepted"),
        requested_run_profile: None,
        requested_model: None,
        idempotency_key: IdempotencyKey::new(idempotency_key).expect("idempotency key"),
        received_at: Utc::now(),
        requested_run_id,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

/// A run id minted by `prepare_turn` submits under the scope it was prepared
/// for, and the accepted run carries exactly that id.
#[tokio::test]
async fn prepared_run_id_submits_under_its_prepared_scope() {
    let coordinator = coordinator();
    let scope = scope("prepared-same", "thread-prepared-same");

    let prepared = coordinator
        .prepare_turn(scope.clone())
        .await
        .expect("prepare_turn mints a run id");
    let SubmitTurnResponse::Accepted { run_id, .. } = coordinator
        .submit_turn(submit_request(scope, Some(prepared), "prepared-same-key"))
        .await
        .expect("prepared id submits in its own scope");

    assert_eq!(run_id, prepared, "the accepted run keeps the prepared id");
}

/// A prepared run id submitted under a DIFFERENT scope (without a parent run)
/// is rejected `Unauthorized` — a prepared id cannot inject lineage into a
/// foreign scope. The reservation is consumed by that first attempt: a repeat
/// submit with the same id falls through to the store's duplicate-bound check
/// (and succeeds here because no process was created).
#[tokio::test]
async fn prepared_run_id_rejects_cross_scope_submit_and_consumes_the_reservation() {
    let coordinator = coordinator();
    let prepared_scope = scope("prepared-a", "thread-prepared-a");
    let foreign_scope = scope("prepared-b", "thread-prepared-b");

    let prepared = coordinator
        .prepare_turn(prepared_scope)
        .await
        .expect("prepare_turn mints a run id");

    let rejected = coordinator
        .submit_turn(submit_request(
            foreign_scope.clone(),
            Some(prepared),
            "prepared-cross-key",
        ))
        .await;
    assert!(
        matches!(rejected, Err(TurnError::Unauthorized)),
        "cross-scope submit of a prepared id must be Unauthorized, got {rejected:?}"
    );

    // Consume-once: the failed attempt spent the reservation, so the same
    // submit now reaches the store unchecked (today's documented behavior).
    let retried = coordinator
        .submit_turn(submit_request(
            foreign_scope,
            Some(prepared),
            "prepared-cross-key",
        ))
        .await;
    assert!(
        matches!(retried, Ok(SubmitTurnResponse::Accepted { .. })),
        "the reservation is consumed on the first attempt, got {retried:?}"
    );
}

/// The cross-scope check exempts child runs: subagent spawn legitimately
/// prepares a run id under the parent scope and submits it under the child
/// scope with `parent_run_id` set.
#[tokio::test]
async fn prepared_run_id_cross_scope_submit_is_exempt_for_child_runs() {
    let coordinator = coordinator();
    let parent_scope = scope("prepared-parent", "thread-prepared-parent");
    let child_scope = scope("prepared-child", "thread-prepared-child");

    let prepared = coordinator
        .prepare_turn(parent_scope)
        .await
        .expect("prepare_turn mints a run id");

    let mut request = submit_request(child_scope, Some(prepared), "prepared-child-key");
    request.parent_run_id = Some(TurnRunId::new());

    let response = coordinator.submit_turn(request).await;
    assert!(
        matches!(response, Ok(SubmitTurnResponse::Accepted { .. })),
        "child-run submits bypass the cross-scope reservation check, got {response:?}"
    );
}

/// `abort_prepared_turn` releases the reservation: a subsequent cross-scope
/// submit of the (no longer reserved) id is not rejected by the coordinator.
#[tokio::test]
async fn abort_prepared_turn_releases_the_reservation() {
    let coordinator = coordinator();
    let prepared_scope = scope("prepared-abort", "thread-prepared-abort");
    let foreign_scope = scope("prepared-abort-b", "thread-prepared-abort-b");

    let prepared = coordinator
        .prepare_turn(prepared_scope)
        .await
        .expect("prepare_turn mints a run id");
    coordinator
        .abort_prepared_turn(prepared)
        .await
        .expect("abort releases the reservation");

    let response = coordinator
        .submit_turn(submit_request(
            foreign_scope,
            Some(prepared),
            "prepared-abort-key",
        ))
        .await;
    assert!(
        matches!(response, Ok(SubmitTurnResponse::Accepted { .. })),
        "an aborted reservation no longer gates scope, got {response:?}"
    );
}

/// The prepared-id cache is bounded: reservation 4097 is refused with a typed
/// capacity error naming the submit-turn resource.
#[tokio::test]
async fn prepare_turn_reservations_are_capacity_bounded() {
    let coordinator = coordinator();
    let scope = scope("prepared-cap", "thread-prepared-cap");

    for _ in 0..4096 {
        coordinator
            .prepare_turn(scope.clone())
            .await
            .expect("reservations under the cap succeed");
    }

    let over_cap = coordinator.prepare_turn(scope).await;
    match over_cap {
        Err(TurnError::CapacityExceeded { resource, cap }) => {
            assert_eq!(resource, TurnCapacityResource::SubmitTurn);
            assert_eq!(cap, 4096);
        }
        other => panic!("reservation over the cap must be CapacityExceeded, got {other:?}"),
    }
}
