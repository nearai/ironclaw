use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_outbound::*;
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};

#[tokio::test]
async fn subscription_access_policy_gates_cursor_checkpoint_creation() {
    let store = InMemoryOutboundStateStore::default();
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
async fn delivery_preparation_revalidates_each_push_and_records_auth_failure_without_target() {
    let store = InMemoryOutboundStateStore::default();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);

    validator.allow(candidate.target.clone());
    let first = service
        .prepare_delivery_attempt(PrepareOutboundDeliveryRequest {
            scope: scope.clone(),
            candidate: candidate.clone(),
            attempted_at: now(),
        })
        .await
        .expect("first authorized delivery attempt");
    let OutboundDeliveryDecision::Authorized { attempt, target } = first else {
        panic!("expected authorized delivery decision");
    };
    assert_eq!(attempt.status, OutboundDeliveryStatus::Pending);
    assert_eq!(target.target, candidate.target);

    let second = service
        .prepare_delivery_attempt(PrepareOutboundDeliveryRequest {
            scope: scope.clone(),
            candidate: candidate.clone(),
            attempted_at: now(),
        })
        .await
        .expect("second authorized delivery attempt");
    assert!(matches!(
        second,
        OutboundDeliveryDecision::Authorized { .. }
    ));
    assert_eq!(
        validator.calls(),
        2,
        "binding is revalidated before every push"
    );

    validator.deny(candidate.target.clone());
    let rejected = service
        .prepare_delivery_attempt(PrepareOutboundDeliveryRequest {
            scope: scope.clone(),
            candidate: candidate.clone(),
            attempted_at: now(),
        })
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
    assert_eq!(attempts.len(), 3);
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.status == OutboundDeliveryStatus::Pending)
            .count(),
        2
    );
    assert_eq!(
        attempts
            .iter()
            .filter(|attempt| attempt.status == OutboundDeliveryStatus::Failed)
            .count(),
        1
    );
}

#[tokio::test]
async fn delivery_preparation_rejects_validator_target_substitution() {
    let store = InMemoryOutboundStateStore::default();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);
    validator.redirect(candidate.target.clone(), reply_ref("reply-other"));

    let err = service
        .prepare_delivery_attempt(PrepareOutboundDeliveryRequest {
            scope: scope.clone(),
            candidate,
            attempted_at: now(),
        })
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
async fn delivery_preparation_fails_closed_when_candidate_skips_revalidation() {
    let store = InMemoryOutboundStateStore::default();
    let access_policy = FakeThreadProjectionAccessPolicy::default();
    let validator = FakeReplyTargetBindingValidator::default();
    let service = OutboundPolicyService::new(&store, &access_policy, &validator);
    let scope = turn_scope("thread-1");
    let mut candidate = candidate(&scope, "reply-default", OutboundPushKind::FinalReply);
    candidate.requires_reply_target_revalidation = false;

    let err = service
        .prepare_delivery_attempt(PrepareOutboundDeliveryRequest {
            scope: scope.clone(),
            candidate,
            attempted_at: now(),
        })
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
    ) -> Result<ThreadProjectionAccessGrant, OutboundError> {
        if self
            .allowed
            .lock()
            .expect("fake access policy lock poisoned")
            .contains(&(request.actor.clone(), request.thread_id.clone()))
        {
            Ok(ThreadProjectionAccessGrant {
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
    redirects: Mutex<HashMap<ReplyTargetBindingRef, ReplyTargetBindingRef>>,
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

    fn redirect(&self, from: ReplyTargetBindingRef, to: ReplyTargetBindingRef) {
        self.redirects
            .lock()
            .expect("fake validator lock poisoned")
            .insert(from, to);
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
    ) -> Result<ValidatedReplyTargetBinding, OutboundError> {
        *self.calls.lock().expect("fake validator lock poisoned") += 1;
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
            return Ok(ValidatedReplyTargetBinding { target });
        }
        if self
            .allowed
            .lock()
            .expect("fake validator lock poisoned")
            .contains(&request.candidate.target)
        {
            Ok(ValidatedReplyTargetBinding {
                target: request.candidate.target,
            })
        } else {
            Err(OutboundError::AccessDenied)
        }
    }
}

fn candidate(scope: &TurnScope, target: &str, kind: OutboundPushKind) -> OutboundPushCandidate {
    OutboundPushCandidate {
        thread_id: scope.thread_id.clone(),
        turn_run_id: Some(TurnRunId::new()),
        target: reply_ref(target),
        kind,
        projection_ref: ProjectionUpdateRef::new("projection:update-1")
            .expect("valid projection ref"),
        requires_reply_target_revalidation: true,
    }
}

fn subscription_id(value: &str) -> ProjectionSubscriptionId {
    ProjectionSubscriptionId::new(value).expect("valid subscription id")
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-a").expect("valid tenant"),
        Some(AgentId::new("agent-a").expect("valid agent")),
        Some(ProjectId::new("project-a").expect("valid project")),
        thread_id(thread),
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

fn now() -> ironclaw_host_api::Timestamp {
    chrono::Utc::now()
}
