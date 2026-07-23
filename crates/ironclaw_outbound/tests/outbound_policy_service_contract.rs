#![allow(clippy::disallowed_methods)] // test helper constructs FilesystemOutboundStateStore directly (arch-simplification §4.3)
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_outbound::*;
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

fn in_memory_outbound_store() -> FilesystemOutboundStateStore<InMemoryBackend> {
    // §4.3: the deleted `FilesystemOutboundStateStore<ironclaw_filesystem::InMemoryBackend>` is replaced by the one
    // production store over a volatile in-memory backend (mirrors the merged
    // budget-gate/run-state consolidations). A local helper because this crate's
    // own integration tests cannot enable its `test-support` feature.
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/outbound").expect("alias"),
        VirtualPath::new("/engine/outbound").expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let scoped = std::sync::Arc::new(ScopedFilesystem::with_fixed_view(
        std::sync::Arc::new(InMemoryBackend::new()),
        mounts,
    ));
    FilesystemOutboundStateStore::new(scoped)
}

#[tokio::test]
async fn subscription_access_policy_gates_cursor_checkpoint_creation() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);

    let alice = actor("alice");
    let alice_scope = projection_scope_for_user("alice", "thread-1");
    access_policy.allow(alice.clone(), thread_id("thread-1"));

    let record = service
        .authorize_subscription(ProjectionSubscriptionRequest {
            subscription_id: subscription_id("sub-alice"),
            actor: alice.clone(),
            scope: alice_scope.clone(),
            thread_id: thread_id("thread-1"),
            after_cursor: Some(ProjectionCursor::for_scope(
                alice_scope.clone(),
                EventCursor::new(7),
            )),
        })
        .await
        .expect("authorized participant can subscribe");
    assert_eq!(record.actor, alice.clone());

    let loaded = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id("sub-alice"),
            actor: alice,
            scope: alice_scope.clone(),
            thread_id: thread_id("thread-1"),
        })
        .await
        .expect("load cursor");
    assert_eq!(
        loaded,
        Some(ProjectionCursor::for_scope(
            alice_scope,
            EventCursor::new(7)
        ))
    );

    let bob = actor("bob");
    let bob_scope = projection_scope_for_user("bob", "thread-1");
    let denied = service
        .authorize_subscription(ProjectionSubscriptionRequest {
            subscription_id: subscription_id("sub-bob"),
            actor: bob.clone(),
            scope: bob_scope.clone(),
            thread_id: thread_id("thread-1"),
            after_cursor: None,
        })
        .await
        .expect_err("non-participant must not subscribe");
    assert!(matches!(denied, OutboundError::AccessDenied));

    let missing = store
        .load_subscription_cursor(LoadSubscriptionCursorRequest {
            subscription_id: subscription_id("sub-bob"),
            actor: bob,
            scope: bob_scope,
            thread_id: thread_id("thread-1"),
        })
        .await
        .expect("denied subscription was not inserted");
    assert_eq!(missing, None);
}

#[tokio::test]
async fn delivery_preparation_revalidates_each_push_with_one_stable_attempt_identity() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);

    validator.allow(candidate.target.clone());
    let first = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate.clone()))
        .await
        .expect("first authorized delivery attempt");
    let OutboundDeliveryDecision::Authorized { attempt, target } = first else {
        panic!("expected authorized delivery decision");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Prepared);
    assert_eq!(target.target(), &candidate.target);

    let second = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate.clone()))
        .await
        .expect("second authorized delivery attempt");
    assert!(matches!(
        &second,
        OutboundDeliveryDecision::Authorized { .. }
    ));
    let OutboundDeliveryDecision::Authorized {
        attempt: second_attempt,
        ..
    } = second
    else {
        unreachable!("matched above")
    };
    assert_eq!(second_attempt.delivery_id, attempt.delivery_id);
    assert_eq!(
        validator.calls(),
        2,
        "binding is revalidated before every push"
    );

    validator.deny(candidate.target.clone());
    let rejected = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate.clone()))
        .await
        .expect("authorization failure is recorded, not surfaced as send target");
    let OutboundDeliveryDecision::Rejected { attempt } = rejected else {
        panic!("expected rejected delivery decision");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );

    let attempts = store
        .list_delivery_attempts(scope)
        .await
        .expect("list delivery attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].delivery_id, attempt.delivery_id);
    assert_eq!(attempts[0].status, OutboundDeliveryStatus::Prepared);
}

#[tokio::test]
async fn delivery_preparation_rejects_validator_target_substitution() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);
    validator.redirect(candidate.target.clone(), reply_ref("reply-other"));

    let err = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate))
        .await
        .expect_err("validator must not substitute a different send target");
    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn delivery_preparation_rejects_scope_candidate_mismatch_before_validator_io() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let other_scope = TurnScope::new(
        TenantId::new("tenant-b").expect("valid tenant"),
        Some(AgentId::new("agent-b").expect("valid agent")),
        Some(ProjectId::new("project-b").expect("valid project")),
        thread_id("thread-1"),
    );
    let candidate = candidate(&other_scope, "reply-default", OutboundPushKind::FinalReply);

    let err = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate))
        .await
        .expect_err("scope/candidate mismatch must fail before validator IO");
    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert_eq!(validator.calls(), 0);
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty(),
        "structurally inconsistent candidates must not leave phantom attempt rows"
    );
}

#[tokio::test]
async fn delivery_preparation_fails_closed_when_candidate_skips_revalidation() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let mut candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);
    candidate.requires_reply_target_revalidation = false;

    let err = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate))
        .await
        .expect_err("delivery must fail closed without revalidation marker");
    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn delivery_preparation_records_transient_validator_error_separately_from_revocation() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);
    validator.fail_transient(candidate.target.clone());

    let rejected = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate.clone()))
        .await
        .expect("transient validator error is classified, not propagated");
    let OutboundDeliveryDecision::Rejected { attempt } = rejected else {
        panic!("expected rejected delivery decision");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(DeliveryFailureKind::TransientValidatorError),
        "transient validator failures must be distinguishable from authorization revocations"
    );

    let attempts = store
        .list_delivery_attempts(scope)
        .await
        .expect("list delivery attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].failure_kind,
        Some(DeliveryFailureKind::TransientValidatorError)
    );
}

#[tokio::test]
async fn delivery_preparation_propagates_validator_caller_bug_errors() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = InvalidRequestValidator;
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);

    let err = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate))
        .await
        .expect_err("caller-bug validator errors must propagate, not be cached as transient");
    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty(),
        "caller-bug errors must not leave a phantom attempt row"
    );
}

#[tokio::test]
async fn communication_delivery_requested_outbound_validates_requested_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request =
        requested_outbound_request("reply:requested", RequestedOutboundKind::ProductMessage);
    validator.allow(reply_ref("reply:requested"));
    store
        .put_communication_preference(preference_record(
            Some("reply:preferred"),
            Some("reply:progress"),
            None,
            None,
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("requested outbound resolves and prepares")
        .expect("requested outbound has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:requested"));
    assert_eq!(attempt.candidate.target, reply_ref("reply:requested"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::FinalReply);
    assert_eq!(attempt.candidate.turn_run_id, Some(turn_run_id()));
    assert_eq!(attempt.status, OutboundDeliveryStatus::Prepared);
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_live_source_route_final_reply_validates_source_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = run_notification_request(
        RunNotificationEventKind::FinalReplyReady,
        RunNotificationOrigin::LiveSourceRoute {
            source_route: SourceRouteContext {
                reply_target_binding_ref: reply_ref("reply:source-route"),
            },
        },
    );
    validator.allow(reply_ref("reply:source-route"));
    store
        .put_communication_preference(preference_record(
            Some("reply:preferred"),
            Some("reply:progress"),
            None,
            None,
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("live source route resolves and prepares")
        .expect("live source route has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:source-route"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::FinalReply);
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_triggered_default_target_validates_preference_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = run_notification_request(
        RunNotificationEventKind::FinalReplyReady,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.allow(reply_ref("reply:triggered-default"));
    store
        .put_communication_preference(preference_record(
            Some("reply:triggered-default"),
            Some("reply:progress"),
            None,
            None,
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("triggered default resolves and prepares")
        .expect("triggered default has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:triggered-default"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::FinalReply);
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_triggered_shared_agent_scope_validates_shared_preference_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = ownerless_agent_scope("thread-1");
    let request = run_notification_request_with_scope(
        scope.clone(),
        RunNotificationEventKind::FinalReplyReady,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.allow(reply_ref("reply:shared-default"));
    store
        .put_communication_preference(preference_record(
            Some("reply:personal-default"),
            Some("reply:personal-progress"),
            None,
            None,
        ))
        .await
        .expect("seed personal preference");
    store
        .put_communication_preference(shared_agent_preference_record(
            Some("reply:shared-default"),
            Some("reply:shared-progress"),
            None,
            None,
        ))
        .await
        .expect("seed shared-agent preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("shared-agent triggered default resolves and prepares")
        .expect("shared-agent triggered default has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:shared-default"));
    assert_eq!(attempt.candidate.target, reply_ref("reply:shared-default"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::FinalReply);
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_lowers_progress_update_to_progress_push_kind() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = run_notification_request(
        RunNotificationEventKind::ProgressUpdate,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.allow(reply_ref("reply:progress"));
    store
        .put_communication_preference(preference_record(
            Some("reply:final"),
            Some("reply:progress"),
            None,
            None,
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("progress update resolves and prepares")
        .expect("progress update has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:progress"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::Progress);
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_lowers_delivery_status_to_delivery_status_push_kind() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = requested_outbound_request("reply:status", RequestedOutboundKind::DeliveryStatus);
    validator.allow(reply_ref("reply:status"));

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("delivery status resolves and prepares")
        .expect("delivery status has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:status"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::DeliveryStatus);
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_auth_prompt_lowers_to_distinct_push_kind() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let request = run_notification_request_with_scope(
        scope.clone(),
        RunNotificationEventKind::AuthRequired,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.allow(reply_ref("reply:auth"));
    store
        .put_communication_preference(preference_record(
            Some("reply:final"),
            Some("reply:progress"),
            None,
            Some("reply:auth"),
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("auth prompt resolves and prepares")
        .expect("auth prompt has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:auth"));
    assert_eq!(attempt.candidate.kind, OutboundPushKind::AuthPrompt);
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_propagates_preference_repository_errors() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let preferences = BackendErrorPreferenceRepository;
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let request = run_notification_request_with_scope(
        scope.clone(),
        RunNotificationEventKind::FinalReplyReady,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );

    let err = service
        .prepare_communication_delivery_attempt(
            prepare_communication_request(request),
            &preferences,
        )
        .await
        .expect_err("preference backend errors propagate through the public seam");

    assert!(matches!(err, OutboundError::Backend));
    assert_eq!(validator.calls(), 0);
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn communication_delivery_triggered_from_source_route_final_reply_prefers_source_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = run_notification_request(
        RunNotificationEventKind::FinalReplyReady,
        RunNotificationOrigin::TriggeredFromSourceRoute {
            trigger: trigger_context(),
            source_route: SourceRouteContext {
                reply_target_binding_ref: reply_ref("reply:source-route"),
            },
        },
    );
    validator.allow(reply_ref("reply:source-route"));
    store
        .put_communication_preference(preference_record(
            Some("reply:triggered-default"),
            Some("reply:progress"),
            None,
            None,
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("triggered source-route resolves and prepares")
        .expect("triggered source-route has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(target.target(), &reply_ref("reply:source-route"));
    assert_eq!(attempt.candidate.target, reply_ref("reply:source-route"));
    assert_eq!(validator.calls(), 1);
}

#[tokio::test]
async fn communication_delivery_system_event_returns_no_delivery_without_records() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let request = run_notification_request_with_scope(
        scope.clone(),
        RunNotificationEventKind::ProgressUpdate,
        RunNotificationOrigin::SystemEvent {
            reason: SystemEventReasonCode::Operator,
        },
    );

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("system event resolves");

    assert!(decision.is_none());
    assert_eq!(validator.calls(), 0);
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn communication_delivery_revoked_target_records_sanitized_failure_without_target() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request =
        requested_outbound_request("reply:revoked", RequestedOutboundKind::ProductMessage);
    let scope = request.scope.clone();
    validator.deny(reply_ref("reply:revoked"));

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("revocation is recorded as rejected")
        .expect("requested outbound has a delivery target");

    let OutboundDeliveryDecision::Rejected { attempt } = decision else {
        panic!("expected rejected delivery");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_exact_owner_validation_rejects_target_substitution() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let request = run_notification_request(
        RunNotificationEventKind::ApprovalNeeded,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.redirect(reply_ref("reply:approval-target"), reply_ref("reply:other"));
    store
        .put_communication_preference(preference_record(
            Some("reply:final"),
            Some("reply:progress"),
            Some("reply:approval-target"),
            Some("reply:auth"),
        ))
        .await
        .expect("seed preference");

    let err = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect_err("validator must not substitute a different prompt target");

    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert_eq!(validator.calls(), 1);
    assert!(
        store
            .list_delivery_attempts(turn_scope("thread-1"))
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

#[tokio::test]
async fn communication_delivery_validator_can_enforce_prompt_actor_context() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let request = run_notification_request_with_scope(
        scope.clone(),
        RunNotificationEventKind::ApprovalNeeded,
        RunNotificationOrigin::Triggered {
            trigger: trigger_context(),
        },
    );
    validator.allow(reply_ref("reply:approval-target"));
    validator.require_actor(actor("exact-owner"));
    store
        .put_communication_preference(preference_record(
            Some("reply:final"),
            Some("reply:progress"),
            Some("reply:approval-target"),
            Some("reply:auth"),
        ))
        .await
        .expect("seed preference");

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("actor mismatch is a validator rejection, not a service error")
        .expect("approval prompt has a delivery target");

    let OutboundDeliveryDecision::Rejected { attempt } = decision else {
        panic!("expected rejected delivery");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_actor_and_modality_forwarded_through_lowering() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let expected_actor = actor("exact-owner");
    let expected_modality = CommunicationModality::Voice;
    let mut request = requested_outbound_request_with_scope(
        scope.clone(),
        "reply:requested",
        RequestedOutboundKind::ProductMessage,
    );
    request.actor = expected_actor.clone();
    request.modality = expected_modality;
    validator.allow(reply_ref("reply:requested"));
    validator.require_actor(expected_actor);
    validator.require_modality(expected_modality);

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("matching actor and modality authorize")
        .expect("requested outbound has a delivery target");

    let OutboundDeliveryDecision::Authorized { attempt, target } = decision else {
        panic!("expected authorized delivery");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Prepared);
    assert_eq!(target.target(), &reply_ref("reply:requested"));
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_validator_can_enforce_requested_modality() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let mut request = requested_outbound_request_with_scope(
        scope.clone(),
        "reply:requested",
        RequestedOutboundKind::ProductMessage,
    );
    request.modality = CommunicationModality::Voice;
    validator.allow(reply_ref("reply:requested"));
    validator.require_modality(CommunicationModality::Text);

    let decision = service
        .prepare_communication_delivery_attempt(prepare_communication_request(request), &store)
        .await
        .expect("modality mismatch is a validator rejection, not a service error")
        .expect("requested outbound has a delivery target");

    let OutboundDeliveryDecision::Rejected { attempt } = decision else {
        panic!("expected rejected delivery");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(DeliveryFailureKind::AuthorizationRevoked)
    );
    assert_eq!(validator.calls(), 1);
    assert_eq!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .as_slice(),
        std::slice::from_ref(&attempt)
    );
}

#[tokio::test]
async fn communication_delivery_scope_candidate_mismatch_rejects_before_validator_io() {
    let store = in_memory_outbound_store();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let other_scope = TurnScope::new(
        TenantId::new("tenant-b").expect("valid tenant"),
        Some(AgentId::new("agent-b").expect("valid agent")),
        Some(ProjectId::new("project-b").expect("valid project")),
        thread_id("thread-b"),
    );
    let candidate = candidate(
        &other_scope,
        "reply:requested",
        OutboundPushKind::FinalReply,
    );

    let err = service
        .prepare_delivery_attempt(prepare_outbound_request(scope.clone(), candidate))
        .await
        .expect_err("scope mismatch must fail before validator IO");
    assert!(matches!(err, OutboundError::InvalidRequest { .. }));
    assert_eq!(validator.calls(), 0);
    assert!(
        store
            .list_delivery_attempts(scope)
            .await
            .expect("list delivery attempts")
            .is_empty()
    );
}

struct InvalidRequestValidator;

#[async_trait]
impl ReplyTargetBindingValidator for InvalidRequestValidator {
    async fn validate_reply_target(
        &self,
        _request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        Err(OutboundError::InvalidRequest {
            reason: "validator received bad input",
        })
    }
}

struct BackendErrorPreferenceRepository;

#[async_trait]
impl CommunicationPreferenceRepository for BackendErrorPreferenceRepository {
    async fn load_communication_preference(
        &self,
        _key: CommunicationPreferenceKey,
    ) -> Result<Option<VersionedCommunicationPreferenceRecord>, OutboundError> {
        Err(OutboundError::Backend)
    }

    async fn write_communication_preference(
        &self,
        _request: WriteCommunicationPreferenceRequest,
    ) -> Result<VersionedCommunicationPreferenceRecord, OutboundError> {
        Err(OutboundError::Backend)
    }
}

#[derive(Default)]
struct FakeThreadProjectionAccessPolicy {
    allowed: Mutex<HashSet<(TurnActor, ThreadId)>>,
}

impl FakeThreadProjectionAccessPolicy {
    fn allow(&self, actor: TurnActor, thread_id: ThreadId) {
        self.allowed
            .lock()
            .expect("fake access policy lock poisoned")
            .insert((actor, thread_id));
    }
}

#[async_trait]
impl ThreadProjectionAccessPolicy for FakeThreadProjectionAccessPolicy {
    async fn authorize_projection_access(
        &self,
        request: ThreadProjectionAccessRequest,
    ) -> Result<ThreadProjectionAccessClaim, OutboundError> {
        if self
            .allowed
            .lock()
            .expect("fake access policy lock poisoned")
            .contains(&(request.actor.clone(), request.thread_id.clone()))
        {
            Ok(ThreadProjectionAccessClaim {
                actor: request.actor,
                scope: request.scope,
                thread_id: request.thread_id,
            })
        } else {
            Err(OutboundError::AccessDenied)
        }
    }
}

#[derive(Default)]
struct FakeReplyTargetBindingValidator {
    allowed: Mutex<HashSet<ReplyTargetBindingRef>>,
    denied: Mutex<HashSet<ReplyTargetBindingRef>>,
    transient: Mutex<HashSet<ReplyTargetBindingRef>>,
    redirects: Mutex<HashMap<ReplyTargetBindingRef, ReplyTargetBindingRef>>,
    required_actor: Mutex<Option<TurnActor>>,
    required_modality: Mutex<Option<CommunicationModality>>,
    calls: Mutex<usize>,
}

impl FakeReplyTargetBindingValidator {
    fn allow(&self, target: ReplyTargetBindingRef) {
        self.allowed
            .lock()
            .expect("fake validator lock poisoned")
            .insert(target);
    }

    fn deny(&self, target: ReplyTargetBindingRef) {
        self.denied
            .lock()
            .expect("fake validator lock poisoned")
            .insert(target);
    }

    fn fail_transient(&self, target: ReplyTargetBindingRef) {
        self.transient
            .lock()
            .expect("fake validator lock poisoned")
            .insert(target);
    }

    fn redirect(&self, from: ReplyTargetBindingRef, to: ReplyTargetBindingRef) {
        self.redirects
            .lock()
            .expect("fake validator lock poisoned")
            .insert(from, to);
    }

    fn require_actor(&self, actor: TurnActor) {
        *self
            .required_actor
            .lock()
            .expect("fake validator lock poisoned") = Some(actor);
    }

    fn require_modality(&self, modality: CommunicationModality) {
        *self
            .required_modality
            .lock()
            .expect("fake validator lock poisoned") = Some(modality);
    }

    fn calls(&self) -> usize {
        *self.calls.lock().expect("fake validator lock poisoned")
    }
}

#[async_trait]
impl ReplyTargetBindingValidator for FakeReplyTargetBindingValidator {
    async fn validate_reply_target(
        &self,
        request: ReplyTargetValidationRequest,
    ) -> Result<ReplyTargetBindingClaim, OutboundError> {
        *self.calls.lock().expect("fake validator lock poisoned") += 1;
        if self
            .required_actor
            .lock()
            .expect("fake validator lock poisoned")
            .as_ref()
            .is_some_and(|actor| actor != &request.actor)
        {
            return Err(OutboundError::AccessDenied);
        }
        if self
            .required_modality
            .lock()
            .expect("fake validator lock poisoned")
            .is_some_and(|modality| modality != request.modality)
        {
            return Err(OutboundError::AccessDenied);
        }
        if self
            .transient
            .lock()
            .expect("fake validator lock poisoned")
            .contains(&request.candidate.target)
        {
            return Err(OutboundError::Backend);
        }
        if self
            .denied
            .lock()
            .expect("fake validator lock poisoned")
            .contains(&request.candidate.target)
        {
            return Err(OutboundError::AccessDenied);
        }
        if let Some(target) = self
            .redirects
            .lock()
            .expect("fake validator lock poisoned")
            .get(&request.candidate.target)
            .cloned()
        {
            return Ok(ReplyTargetBindingClaim::new(target));
        }
        if self
            .allowed
            .lock()
            .expect("fake validator lock poisoned")
            .contains(&request.candidate.target)
        {
            Ok(ReplyTargetBindingClaim::new(request.candidate.target))
        } else {
            Err(OutboundError::AccessDenied)
        }
    }
}

fn candidate(scope: &TurnScope, target: &str, kind: OutboundPushKind) -> OutboundPushCandidate {
    OutboundPushCandidate {
        tenant_id: scope.tenant_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        thread_id: scope.thread_id.clone(),
        turn_run_id: Some(TurnRunId::new()),
        target: reply_ref(target),
        kind,
        projection_ref: ProjectionUpdateRef::new("projection:update-1")
            .expect("valid projection ref"),
        requires_reply_target_revalidation: true,
    }
}

fn prepare_outbound_request(
    scope: TurnScope,
    candidate: OutboundPushCandidate,
) -> PrepareOutboundDeliveryRequest {
    PrepareOutboundDeliveryRequest {
        scope,
        actor: actor("user-a"),
        modality: CommunicationModality::Text,
        candidate,
        attempted_at: now(),
    }
}

fn prepare_communication_request(
    resolution_request: CommunicationDeliveryResolutionRequest,
) -> PrepareCommunicationDeliveryRequest {
    PrepareCommunicationDeliveryRequest {
        resolution_request,
        turn_run_id: Some(turn_run_id()),
        projection_ref: ProjectionUpdateRef::new("projection:update-1")
            .expect("valid projection ref"),
        attempted_at: now(),
    }
}

fn requested_outbound_request(
    target: &str,
    kind: RequestedOutboundKind,
) -> CommunicationDeliveryResolutionRequest {
    requested_outbound_request_with_scope(turn_scope("thread-1"), target, kind)
}

fn requested_outbound_request_with_scope(
    scope: TurnScope,
    target: &str,
    kind: RequestedOutboundKind,
) -> CommunicationDeliveryResolutionRequest {
    CommunicationDeliveryResolutionRequest {
        scope,
        actor: actor("user-a"),
        modality: CommunicationModality::Text,
        intent: CommunicationDeliveryIntent::RequestedOutbound(RequestedOutboundContext {
            requested_target: reply_ref(target),
            requested_kind: kind,
        }),
    }
}

fn run_notification_request(
    event_kind: RunNotificationEventKind,
    origin: RunNotificationOrigin,
) -> CommunicationDeliveryResolutionRequest {
    run_notification_request_with_scope(turn_scope("thread-1"), event_kind, origin)
}

fn run_notification_request_with_scope(
    scope: TurnScope,
    event_kind: RunNotificationEventKind,
    origin: RunNotificationOrigin,
) -> CommunicationDeliveryResolutionRequest {
    CommunicationDeliveryResolutionRequest {
        scope,
        actor: actor("user-a"),
        modality: CommunicationModality::Text,
        intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
            event_kind,
            origin,
        }),
    }
}

fn preference_record(
    final_reply_target: Option<&str>,
    progress_target: Option<&str>,
    approval_prompt_target: Option<&str>,
    auth_prompt_target: Option<&str>,
) -> CommunicationPreferenceRecord {
    CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::personal(
            TenantId::new("tenant-a").expect("valid tenant"),
            UserId::new("user-a").expect("valid user"),
        ),
        final_reply_target: final_reply_target.map(reply_ref),
        progress_target: progress_target.map(reply_ref),
        approval_prompt_target: approval_prompt_target.map(reply_ref),
        auth_prompt_target: auth_prompt_target.map(reply_ref),
        default_modality: Some(CommunicationModality::Text),
        updated_at: now(),
        updated_by: UserId::new("user-a").expect("valid user"),
    }
}

fn shared_agent_preference_record(
    final_reply_target: Option<&str>,
    progress_target: Option<&str>,
    approval_prompt_target: Option<&str>,
    auth_prompt_target: Option<&str>,
) -> CommunicationPreferenceRecord {
    CommunicationPreferenceRecord {
        scope: DeliveryDefaultScope::shared_agent(
            TenantId::new("tenant-a").expect("valid tenant"),
            AgentId::new("agent-a").expect("valid agent"),
            Some(ProjectId::new("project-a").expect("valid project")),
        ),
        final_reply_target: final_reply_target.map(reply_ref),
        progress_target: progress_target.map(reply_ref),
        approval_prompt_target: approval_prompt_target.map(reply_ref),
        auth_prompt_target: auth_prompt_target.map(reply_ref),
        default_modality: Some(CommunicationModality::Text),
        updated_at: now(),
        updated_by: UserId::new("tenant-admin").expect("valid updater"),
    }
}

fn trigger_context() -> TriggerCommunicationContext {
    TriggerCommunicationContext {
        trigger_origin_ref: TriggerOriginRef::new("trigger:daily")
            .expect("valid trigger origin ref"),
        trigger_source_kind: TriggerSourceKind::Schedule,
        fire_slot: TriggerFireSlot::new("2026-05-29T09:00:00Z").expect("valid fire slot"),
    }
}

fn subscription_id(value: &str) -> ProjectionSubscriptionId {
    ProjectionSubscriptionId::new(value).expect("valid subscription id")
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new("tenant-a").expect("valid tenant"),
        Some(AgentId::new("agent-a").expect("valid agent")),
        Some(ProjectId::new("project-a").expect("valid project")),
        thread_id(thread),
        Some(UserId::new("user-a").expect("valid user")),
    )
}

fn ownerless_agent_scope(thread: &str) -> TurnScope {
    TurnScope::new_with_owner(
        TenantId::new("tenant-a").expect("valid tenant"),
        Some(AgentId::new("agent-a").expect("valid agent")),
        Some(ProjectId::new("project-a").expect("valid project")),
        thread_id(thread),
        None,
    )
}

fn projection_scope_for_user(user: &str, thread: &str) -> ProjectionScope {
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new("tenant-a").expect("valid tenant"),
            UserId::new(user).expect("valid user"),
            Some(AgentId::new("agent-a").expect("valid agent")),
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-a").expect("valid project")),
            mission_id: None,
            thread_id: Some(thread_id(thread)),
            process_id: None,
        },
    }
}

fn actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).expect("valid user"))
}

fn thread_id(value: &str) -> ThreadId {
    ThreadId::new(value).expect("valid thread")
}

fn reply_ref(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).expect("valid reply target")
}

fn turn_run_id() -> TurnRunId {
    TurnRunId::parse("11111111-1111-4111-8111-111111111111").expect("valid turn run id")
}

fn now() -> ironclaw_host_api::Timestamp {
    chrono::Utc::now()
}
