// arch-exempt: large_file, whole-path delivery regressions reuse the existing conversation-store contract harness, plan #6175
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AcceptConversationMessageRequest, AcceptedConversationMessage,
    AcceptedConversationMessageLookup, AcceptedConversationMessageReplay, AdapterInstallationId,
    AdapterKind, ConditionalUnpairOutcome, ConversationBindingResolution,
    ConversationBindingService, ConversationInboundClassification, ConversationRouteKind,
    ConversationTurnSubmission, ConversationTurnSubmitter, ExpectedExternalActorOwner,
    ExternalConversationIdentity, ExternalEventId, InMemoryConversationServices,
    InboundConversationService, InboundMessageContentRef, InboundTurnError, InboundTurnRequest,
    InboundTurnService, LinkConversationRequest, LinkedConversationBinding,
    MessageIdempotencyStatus, ReplyTargetBinding, ResetConversationRequest,
    ResolveStoredReplyTargetRequest, StoredReplyTargetAccess, ThreadAccessDecision,
    TurnSubmissionError, TurnSubmissionErrorCategory, TurnSubmissionRetry,
    ValidateReplyTargetRequest,
};
use ironclaw_extension_contracts::external::{
    ExternalActorBindingEpoch, ExternalActorRef, ExternalConversationRef,
};
use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{
    AcceptedMessageRef, IdempotencyKey, ReplyTargetBindingRef, RunProfileId, RunProfileRequest,
    RunProfileVersion, SourceBindingRef, SubmitTurnResponse, TurnActor, TurnRunId, TurnScope,
    TurnStatus,
};
// Dev-only: `ironclaw_turns` is a dev-dependency of this crate, never a normal
// one. The fakes below stand in for the composition adapter that implements the
// submission port, so they mint the same `SubmitTurnRequest` the real adapter
// mints and these tests keep asserting on that exact value. See
// `submit_turn_request` at the bottom of this file and the manifest comment.
use ironclaw_turns::{SubmitTurnRequest, product_context};

#[tokio::test]
async fn paired_actor_without_binding_creates_thread_binding_message_and_submits_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let response = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", Some("thread-1")),
            "telegram-event-1",
        ))
        .await
        .unwrap();

    assert_eq!(response.resolution.tenant_id, tenant());
    assert_eq!(response.resolution.actor.user_id, user("alice"));
    assert_eq!(
        response.accepted_message.idempotency,
        MessageIdempotencyStatus::Inserted
    );
    assert_eq!(coordinator.submissions().len(), 1);
    let submitted = &coordinator.submissions()[0];
    assert_eq!(submitted.scope, response.resolution.turn_scope);
    assert_eq!(submitted.actor, TurnActor::new(user("alice")));
    assert_eq!(
        submitted.accepted_message_ref,
        response.accepted_message.message_ref
    );
}

#[tokio::test]
async fn untrusted_inbound_uses_untrusted_binding_resolution_and_preserves_requested_scope_hints() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let binding = UntrustedOnlyBindingService::new(services.clone());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(binding.clone(), services.clone(), coordinator);

    inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-untrusted-path", None),
            "telegram-event-untrusted-path",
        ))
        .await
        .unwrap();

    assert_eq!(binding.untrusted_calls(), 1);
    assert_eq!(binding.trusted_calls(), 0);
    let resolve_requests = binding.resolve_requests();
    assert_eq!(resolve_requests.len(), 1);
    assert_eq!(resolve_requests[0].requested_agent_id, Some(agent()));
    assert_eq!(resolve_requests[0].requested_project_id, Some(project()));
}

#[tokio::test]
async fn unpaired_external_actor_returns_binding_required_before_message_or_turn_submission() {
    let services = InMemoryConversationServices::default();
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let err = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("unknown-user"),
            external_conversation("chat-1", None),
            "telegram-event-unpaired",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::BindingRequired { .. }));
    assert!(coordinator.submissions().is_empty());
    assert!(services.accepted_messages().await.is_empty());
}

#[tokio::test]
async fn lookup_binding_does_not_create_missing_conversation_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let missing = services
        .lookup_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-lookup-only", None),
            "telegram-event-lookup-only",
        ))
        .await
        .unwrap_err();
    assert!(matches!(missing, InboundTurnError::BindingRequired { .. }));

    let created = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-lookup-only", None),
            "telegram-event-create-after-lookup",
        ))
        .await
        .expect("lookup-only miss must not poison later create");
    assert_eq!(created.actor.user_id, user("alice"));
}

#[tokio::test]
async fn unpair_external_actor_revokes_direct_conversation_bindings() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-1");
    let conversation = external_conversation("chat-unpair-revoke", None);
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "telegram-event-before-unpair",
        ))
        .await
        .expect("first direct binding");

    services
        .unpair_external_actor(&tenant(), &telegram(), &default_installation(), &actor)
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;

    let stale_reply_target = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            telegram(),
            default_installation(),
            actor.clone(),
            first.turn_scope.thread_id.clone(),
            first.reply_target_binding_ref,
        ))
        .await
        .expect_err("old reply target should be revoked with the direct binding");
    assert!(matches!(
        stale_reply_target,
        InboundTurnError::ThreadNotFound { .. }
    ));
    let missing = services
        .lookup_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "telegram-event-after-repair-lookup",
        ))
        .await
        .expect_err("old direct conversation binding should be gone after re-pair");
    assert!(matches!(missing, InboundTurnError::BindingRequired { .. }));

    let rebound = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor,
            conversation,
            "telegram-event-after-repair",
        ))
        .await
        .expect("re-paired actor should create a fresh binding");
    assert_ne!(
        rebound.turn_scope.thread_id, first.turn_scope.thread_id,
        "unpair must not silently reuse the pre-removal Slack DM thread route"
    );
}

#[tokio::test]
async fn reassigning_external_actor_revokes_the_previous_users_direct_route() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-relinked");
    let conversation = external_conversation("chat-relinked", None);
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "telegram-event-before-relink",
        ))
        .await
        .expect("first user's direct binding");

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("bob"),
        )
        .await;

    let missing = services
        .lookup_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "telegram-event-after-relink-lookup",
        ))
        .await
        .expect_err("reassigning an actor must revoke the previous user's direct route");
    assert!(matches!(missing, InboundTurnError::BindingRequired { .. }));

    let rebound = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor,
            conversation,
            "telegram-event-after-relink",
        ))
        .await
        .expect("reassigned actor should create a fresh direct binding");
    assert_eq!(rebound.actor.user_id, user("bob"));
    assert_ne!(
        rebound.turn_scope.thread_id, first.turn_scope.thread_id,
        "the new owner must never inherit the previous user's thread"
    );
}

#[tokio::test]
async fn user_scoped_channel_unpair_revokes_every_owned_actor_and_direct_route() {
    let services = InMemoryConversationServices::default();
    let removed_actor = external_actor("slack-user-reconnected");
    let retained_actor = external_actor("slack-user-other-installation");
    let removed_conversation = external_conversation("slack-dm-reconnected", None);
    let other_installation =
        AdapterInstallationId::new("other-installation").expect("installation");

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            removed_actor.clone(),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            other_installation.clone(),
            retained_actor.clone(),
            user("alice"),
        )
        .await;
    services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            removed_actor.clone(),
            removed_conversation.clone(),
            "slack-event-before-removal",
        ))
        .await
        .expect("old user owns the first direct route");

    let removed = services
        .unpair_external_actors_owned_by(
            &tenant(),
            &telegram(),
            Some(&default_installation()),
            &user("alice"),
        )
        .await
        .expect("user-scoped channel removal");
    assert_eq!(removed, 1);

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            removed_actor.clone(),
            user("bob"),
        )
        .await;
    let rebound = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            removed_actor,
            removed_conversation,
            "slack-event-after-reconnect",
        ))
        .await
        .expect("the same external actor can bind to the new user after removal");
    assert_eq!(rebound.actor.user_id, user("bob"));

    let mut retained_request = resolve_request(
        telegram(),
        retained_actor,
        external_conversation("slack-dm-other-installation", None),
        "slack-event-other-installation",
    );
    retained_request.adapter_installation_id = other_installation;
    let retained = services
        .resolve_or_create_binding(retained_request)
        .await
        .expect("another installation remains paired");
    assert_eq!(retained.actor.user_id, user("alice"));
}

#[tokio::test]
async fn stored_reply_target_revalidates_durable_run_authority_and_revocation() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-stored-route");
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let resolved = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            external_conversation("stored-route-chat", None),
            "stored-route-event",
        ))
        .await
        .expect("direct binding");

    let request = ResolveStoredReplyTargetRequest {
        tenant_id: tenant(),
        actor_user_id: user("alice"),
        current_thread_id: resolved.turn_scope.thread_id.clone(),
        reply_target_binding_ref: resolved.reply_target_binding_ref.clone(),
        access: StoredReplyTargetAccess::ExactOriginActor,
    };
    let target = services
        .resolve_stored_reply_target(request.clone())
        .await
        .expect("current owner may resolve stored target");
    assert_eq!(target.adapter_kind, telegram());
    assert_eq!(target.route_kind, ConversationRouteKind::Direct);
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "stored-route-chat"
    );

    services
        .unpair_external_actor(&tenant(), &telegram(), &default_installation(), &actor)
        .await;
    let error = services
        .resolve_stored_reply_target(request)
        .await
        .expect_err("unpair must revoke the durable route");
    assert!(matches!(error, InboundTurnError::ThreadNotFound { .. }));
}

#[tokio::test]
async fn stored_shared_reply_target_is_per_event_and_authority_bound_to_the_pinger() {
    // Ephemeral per-ping (Model A): two pingers on one conversation get
    // DISTINCT threads AND DISTINCT per-event reply targets. Each pinger
    // resolves BOTH access kinds for THEIR OWN reply target — ordinary (route
    // access is shared) and authority-bearing (its origin actor is the pinger).
    // A reply ref from a DIFFERENT event fails the thread-match check as
    // AccessDenied, and a paired user who never resolved gets neither. This
    // preserves the three security properties without a shared thread:
    // impersonation blocked at accept, authority-bearing prompts route only to
    // the pinger, and stale/cross-event reply refs denied. (Each per-event
    // thread has a single participant, so the ordinary-vs-authority split
    // collapses to "the pinger only" for channels; the access-kind logic is
    // unchanged.)
    let services = InMemoryConversationServices::default();
    let alice_actor = external_actor("stored-shared-alice");
    let bob_actor = external_actor("stored-shared-bob");
    for (actor, owner) in [
        (alice_actor.clone(), user("alice")),
        (bob_actor.clone(), user("bob")),
        (external_actor("stored-shared-charlie"), user("charlie")),
    ] {
        services
            .pair_external_actor(tenant(), telegram(), default_installation(), actor, owner)
            .await;
    }
    let conversation = external_conversation("stored-shared-chat", Some("topic-a"));
    let mut alice_request = resolve_request(
        telegram(),
        alice_actor,
        conversation.clone(),
        "stored-shared-alice-event",
    );
    alice_request.route_kind = ConversationRouteKind::Shared;
    let alice_resolution = services
        .resolve_or_create_binding(alice_request)
        .await
        .expect("alice binds the shared conversation thread");
    let mut bob_request = resolve_request(
        telegram(),
        bob_actor,
        conversation,
        "stored-shared-bob-event",
    );
    bob_request.route_kind = ConversationRouteKind::Shared;
    let bob_resolution = services
        .resolve_or_create_binding(bob_request)
        .await
        .expect("bob joins the same shared conversation thread");
    // Distinct pings → distinct ephemeral threads AND distinct per-event reply
    // targets. (Retired: "one conversation, one stored reply target".)
    assert_ne!(
        bob_resolution.turn_scope.thread_id, alice_resolution.turn_scope.thread_id,
        "each pinger gets their own ephemeral thread"
    );
    assert_ne!(
        bob_resolution.reply_target_binding_ref, alice_resolution.reply_target_binding_ref,
        "each ping gets its own per-event reply target"
    );

    // Each pinger resolves BOTH access kinds for THEIR OWN per-event reply
    // target: ordinary (route access is shared) and authority-bearing (its
    // origin actor is the pinger).
    for (owner, resolution) in [
        (user("alice"), &alice_resolution),
        (user("bob"), &bob_resolution),
    ] {
        for access in [
            StoredReplyTargetAccess::OrdinaryReply,
            StoredReplyTargetAccess::ExactOriginActor,
        ] {
            let resolved = services
                .resolve_stored_reply_target(ResolveStoredReplyTargetRequest {
                    tenant_id: tenant(),
                    actor_user_id: owner.clone(),
                    current_thread_id: resolution.turn_scope.thread_id.clone(),
                    reply_target_binding_ref: resolution.reply_target_binding_ref.clone(),
                    access,
                })
                .await
                .expect("the pinger resolves both access kinds for their own reply target");
            assert_eq!(resolved.route_kind, ConversationRouteKind::Shared);
            assert_eq!(resolved.actor_user_id, owner);
        }
    }

    // A reply ref from a DIFFERENT event fails the thread-match check on BOTH
    // access kinds: bob cannot resolve alice's per-event reply target on his
    // own thread (stale/cross-event → AccessDenied). Authority-bearing prompts
    // therefore never leak across pingers.
    for access in [
        StoredReplyTargetAccess::OrdinaryReply,
        StoredReplyTargetAccess::ExactOriginActor,
    ] {
        let error = services
            .resolve_stored_reply_target(ResolveStoredReplyTargetRequest {
                tenant_id: tenant(),
                actor_user_id: user("bob"),
                current_thread_id: bob_resolution.turn_scope.thread_id.clone(),
                reply_target_binding_ref: alice_resolution.reply_target_binding_ref.clone(),
                access,
            })
            .await
            .expect_err("a cross-event reply ref must be denied");
        assert!(matches!(error, InboundTurnError::AccessDenied { .. }));
    }

    // A paired user who never resolved in the conversation is not a participant
    // of any per-event thread: they resolve neither access kind.
    for access in [
        StoredReplyTargetAccess::OrdinaryReply,
        StoredReplyTargetAccess::ExactOriginActor,
    ] {
        let error = services
            .resolve_stored_reply_target(ResolveStoredReplyTargetRequest {
                tenant_id: tenant(),
                actor_user_id: user("charlie"),
                current_thread_id: alice_resolution.turn_scope.thread_id.clone(),
                reply_target_binding_ref: alice_resolution.reply_target_binding_ref.clone(),
                access,
            })
            .await
            .expect_err("a never-joined user must not resolve any reply target");
        assert!(matches!(error, InboundTurnError::AccessDenied { .. }));
    }
}

#[tokio::test]
async fn unpair_external_actor_if_owned_by_revokes_the_expected_epoch_and_direct_route() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-conditional");
    let epoch = ExternalActorBindingEpoch::new("generation-7").expect("epoch");
    services
        .pair_external_actor_with_epoch(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
            epoch.clone(),
        )
        .await
        .expect("pair with epoch");
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            external_conversation("chat-conditional-unpair", None),
            "telegram-event-conditional-unpair",
        ))
        .await
        .expect("first direct binding");
    assert_eq!(first.binding_epoch.as_ref(), Some(&epoch));

    let outcome = services
        .unpair_external_actor_if_owned_by(
            &tenant(),
            &telegram(),
            &default_installation(),
            &actor,
            &ExpectedExternalActorOwner {
                user_id: user("alice"),
                binding_epoch: Some(epoch),
            },
        )
        .await
        .expect("conditional unpair");

    assert_eq!(outcome, ConditionalUnpairOutcome::Unpaired);
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let missing = services
        .lookup_binding(resolve_request(
            telegram(),
            actor,
            external_conversation("chat-conditional-unpair", None),
            "telegram-event-after-conditional-unpair",
        ))
        .await
        .expect_err("matching conditional unpair must revoke the direct route");
    assert!(matches!(missing, InboundTurnError::BindingRequired { .. }));
}

#[tokio::test]
async fn unpair_external_actor_if_owned_by_preserves_a_newer_owner() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-owner-race");
    let old_epoch = ExternalActorBindingEpoch::new("generation-1").expect("epoch");
    let new_epoch = ExternalActorBindingEpoch::new("generation-2").expect("epoch");
    services
        .pair_external_actor_with_epoch(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
            old_epoch.clone(),
        )
        .await
        .expect("old pairing");
    let _original_route = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            external_conversation("chat-owner-race", None),
            "telegram-event-owner-race-old",
        ))
        .await
        .expect("old owner's route");
    services
        .pair_external_actor_with_epoch(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("bob"),
            new_epoch.clone(),
        )
        .await
        .expect("new pairing");

    let outcome = services
        .unpair_external_actor_if_owned_by(
            &tenant(),
            &telegram(),
            &default_installation(),
            &actor,
            &ExpectedExternalActorOwner {
                user_id: user("alice"),
                binding_epoch: Some(old_epoch),
            },
        )
        .await
        .expect("stale conditional unpair");

    assert_eq!(outcome, ConditionalUnpairOutcome::OwnerChanged);
    // Note: the follow-on lookup that re-read the route AS THE NEW OWNER (bob)
    // retired with the ephemeral-per-ping remodel (#7377) — it relied on
    // manually adding bob to the thread's participant set, which no longer
    // exists (threads are single-participant). The conditional-unpair
    // OwnerChanged outcome above is the pin.
}

#[tokio::test]
async fn unpair_external_actor_if_owned_by_is_idempotent_when_absent() {
    let services = InMemoryConversationServices::default();
    let outcome = services
        .unpair_external_actor_if_owned_by(
            &tenant(),
            &telegram(),
            &default_installation(),
            &external_actor("telegram-user-absent"),
            &ExpectedExternalActorOwner {
                user_id: user("alice"),
                binding_epoch: Some(
                    ExternalActorBindingEpoch::new("generation-absent").expect("epoch"),
                ),
            },
        )
        .await
        .expect("absent unpair is idempotent");

    assert_eq!(outcome, ConditionalUnpairOutcome::AlreadyAbsent);
}

#[tokio::test]
async fn pair_external_actor_without_epoch_clears_an_older_epoch_for_the_same_actor_key() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-clear-epoch");
    services
        .pair_external_actor_with_epoch(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
            ExternalActorBindingEpoch::new("generation-old").expect("epoch"),
        )
        .await
        .expect("epoch pairing");

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;

    let resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor,
            external_conversation("chat-clear-epoch", None),
            "telegram-event-clear-epoch",
        ))
        .await
        .expect("epoch-less pairing remains usable");
    assert_eq!(resolution.binding_epoch, None);
}

#[tokio::test]
async fn unpair_external_actor_clears_direct_external_event_routes() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("telegram-user-1");
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            external_conversation("chat-unpair-event-route-old", None),
            "telegram-event-before-unpair-route",
        ))
        .await
        .expect("first direct binding");

    services
        .unpair_external_actor(&tenant(), &telegram(), &default_installation(), &actor)
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;

    let rebound = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor,
            external_conversation("chat-unpair-event-route-new", None),
            "telegram-event-before-unpair-route",
        ))
        .await
        .expect("unpair should remove the stale direct event route");
    assert_ne!(
        rebound.turn_scope.thread_id, first.turn_scope.thread_id,
        "reusing an event id after direct unpair must create a fresh route, not revive stale state"
    );
}

#[tokio::test]
async fn unpair_external_actor_preserves_shared_conversation_routes() {
    let services = InMemoryConversationServices::default();
    let alice_actor = external_actor("alice-telegram");
    let bob_actor = external_actor("bob-telegram");
    let conversation = external_conversation("shared-chat-unpair-preserve", Some("topic-a"));
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            alice_actor.clone(),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            bob_actor.clone(),
            user("bob"),
        )
        .await;
    let mut alice_request = resolve_request(
        telegram(),
        alice_actor.clone(),
        conversation.clone(),
        "shared-chat-alice",
    );
    alice_request.route_kind = ConversationRouteKind::Shared;
    let alice_resolution = services
        .resolve_or_create_binding(alice_request)
        .await
        .expect("alice creates shared binding");
    let mut bob_before_unpair = resolve_request(
        telegram(),
        bob_actor.clone(),
        conversation.clone(),
        "shared-chat-bob-before-unpair",
    );
    bob_before_unpair.route_kind = ConversationRouteKind::Shared;
    let bob_before_unpair = services
        .resolve_or_create_binding(bob_before_unpair)
        .await
        .expect("bob joins the shared binding before alice unpairs");

    services
        .unpair_external_actor(
            &tenant(),
            &telegram(),
            &default_installation(),
            &alice_actor,
        )
        .await;

    let mut bob_after_unpair = resolve_request(
        telegram(),
        bob_actor,
        conversation,
        "shared-chat-bob-after-unpair",
    );
    bob_after_unpair.route_kind = ConversationRouteKind::Shared;
    let bob_after_unpair = services
        .resolve_or_create_binding(bob_after_unpair)
        .await
        .expect("alice unpair must not remove the shared conversation route");

    // Ephemeral per-ping: the surviving invariant is that unpairing alice does
    // NOT remove the shared conversation route — bob can still resolve on it
    // (the `.expect` above). Each ping is its own event, so bob's pings get
    // their OWN pinger-owned threads, distinct from alice's and from each
    // other; there is no shared canonical thread to join.
    assert_ne!(
        bob_before_unpair.turn_scope.thread_id, alice_resolution.turn_scope.thread_id,
        "each pinger gets their own ephemeral thread, never a shared one"
    );
    assert_ne!(
        bob_after_unpair.turn_scope.thread_id, bob_before_unpair.turn_scope.thread_id,
        "distinct pings get distinct ephemeral threads"
    );
    assert_eq!(
        bob_after_unpair.actor.user_id,
        user("bob"),
        "bob's runs keep acting as bob after alice leaves"
    );
    assert_eq!(
        bob_after_unpair
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("bob"),
        "each ephemeral thread is owned by its pinger (owner == actor)"
    );
}

#[tokio::test]
async fn unpair_external_actor_if_owned_by_preserves_shared_conversation_routes() {
    let services = InMemoryConversationServices::default();
    let alice_actor = external_actor("alice-conditional-shared");
    let bob_actor = external_actor("bob-conditional-shared");
    let alice_epoch = ExternalActorBindingEpoch::new("generation-shared").expect("epoch");
    let conversation = external_conversation("shared-chat-conditional-preserve", Some("topic-a"));
    services
        .pair_external_actor_with_epoch(
            tenant(),
            telegram(),
            default_installation(),
            alice_actor.clone(),
            user("alice"),
            alice_epoch.clone(),
        )
        .await
        .expect("alice pairing");
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            bob_actor.clone(),
            user("bob"),
        )
        .await;
    let mut alice_request = resolve_request(
        telegram(),
        alice_actor.clone(),
        conversation.clone(),
        "shared-conditional-alice",
    );
    alice_request.route_kind = ConversationRouteKind::Shared;
    let alice_resolution = services
        .resolve_or_create_binding(alice_request)
        .await
        .expect("alice creates shared route");

    let outcome = services
        .unpair_external_actor_if_owned_by(
            &tenant(),
            &telegram(),
            &default_installation(),
            &alice_actor,
            &ExpectedExternalActorOwner {
                user_id: user("alice"),
                binding_epoch: Some(alice_epoch),
            },
        )
        .await
        .expect("conditional unpair");
    assert_eq!(outcome, ConditionalUnpairOutcome::Unpaired);

    let mut bob_request = resolve_request(
        telegram(),
        bob_actor,
        conversation,
        "shared-conditional-bob",
    );
    bob_request.route_kind = ConversationRouteKind::Shared;
    let bob_resolution = services
        .resolve_or_create_binding(bob_request)
        .await
        .expect("shared route remains available to bob");
    // Ephemeral per-ping: alice's conditional unpair cannot take the shared
    // conversation route away from the group — bob still resolves on it (the
    // `.expect` above). Bob's ping gets his OWN pinger-owned thread, distinct
    // from alice's; there is no shared canonical thread and no retained owner
    // to inherit.
    assert_ne!(
        bob_resolution.turn_scope.thread_id, alice_resolution.turn_scope.thread_id,
        "bob's ping gets its own ephemeral thread, not alice's"
    );
    assert_eq!(
        bob_resolution
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("bob"),
        "each ephemeral thread is owned by its pinger (owner == actor)"
    );
    assert_eq!(bob_resolution.actor.user_id, user("bob"));
}

#[tokio::test]
async fn lookup_binding_miss_does_not_reserve_external_event_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let missing = services
        .lookup_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-lookup-miss-source", None),
            "telegram-event-lookup-miss-shared",
        ))
        .await
        .unwrap_err();
    assert!(matches!(missing, InboundTurnError::BindingRequired { .. }));

    let created = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-lookup-miss-legitimate", None),
            "telegram-event-lookup-miss-shared",
        ))
        .await
        .expect("lookup-only miss must not reserve the event route");
    assert_eq!(created.actor.user_id, user("alice"));
}

#[tokio::test]
async fn trusted_scope_is_persisted_on_first_bind() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let first = services
        .resolve_or_create_binding_with_trusted_scope(
            resolve_request(
                telegram(),
                external_actor("telegram-user-1"),
                external_conversation("chat-trusted-scope", None),
                "telegram-event-trusted-scope-1",
            ),
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            None,
        )
        .await
        .expect("first bind");
    assert_eq!(
        first.turn_scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent-alpha")
    );
    assert!(!first.turn_scope.has_explicit_thread_owner());

    let second = services
        .resolve_or_create_binding_with_trusted_scope(
            resolve_request(
                telegram(),
                external_actor("telegram-user-1"),
                external_conversation("chat-trusted-scope", None),
                "telegram-event-trusted-scope-2",
            ),
            Some(AgentId::new("agent-beta").unwrap()),
            Some(ProjectId::new("project-beta").unwrap()),
            None,
        )
        .await
        .expect("existing bind");
    assert_eq!(
        second.turn_scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent-alpha")
    );
    assert_eq!(
        second.turn_scope.project_id.as_ref().map(ProjectId::as_str),
        Some("project-alpha")
    );
    assert!(!second.turn_scope.has_explicit_thread_owner());
}

#[tokio::test]
async fn trusted_owner_is_persisted_on_first_bind() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let first = services
        .resolve_or_create_binding_with_trusted_scope(
            resolve_request(
                telegram(),
                external_actor("telegram-user-1"),
                external_conversation("chat-trusted-owner", None),
                "telegram-event-trusted-owner-1",
            ),
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            Some(user("owner-alpha")),
        )
        .await
        .expect("first bind");
    assert_eq!(
        first
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("owner-alpha")
    );

    let second = services
        .resolve_or_create_binding_with_trusted_scope(
            resolve_request(
                telegram(),
                external_actor("telegram-user-1"),
                external_conversation("chat-trusted-owner", None),
                "telegram-event-trusted-owner-2",
            ),
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            Some(user("owner-beta")),
        )
        .await
        .expect("existing bind");
    assert_eq!(
        second
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("owner-alpha")
    );
}

#[tokio::test]
async fn trusted_scope_rejects_existing_unscoped_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let legacy = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-legacy-unscoped", None),
            "telegram-event-legacy-unscoped",
        ))
        .await
        .expect("legacy unscoped bind");
    assert_eq!(legacy.turn_scope.agent_id, None);
    assert_eq!(legacy.turn_scope.project_id, None);

    // The scope-reinterpretation guard: a Direct trusted resolve against the
    // legacy unscoped row conflicts rather than silently re-scoping it.
    let err = services
        .resolve_or_create_binding_with_trusted_scope(
            resolve_request(
                telegram(),
                external_actor("telegram-user-1"),
                external_conversation("chat-legacy-unscoped", None),
                "telegram-event-legacy-trusted-scope",
            ),
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            None,
        )
        .await
        .expect_err("trusted scope must not reinterpret legacy bindings");
    assert!(matches!(err, InboundTurnError::BindingConflict { .. }));

    // Pin changed with the run-acts-as-invoker ruling (#7377): bindings are
    // conversation-keyed on BOTH route kinds, so the owner's Shared-marked
    // trusted resolve addresses the SAME unscoped row — and the guard holds
    // there identically instead of minting a scoped per-actor sibling.
    // Trusted-scope semantics did not change; only the row the shared route
    // reaches did.
    let mut trusted_shared = resolve_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-legacy-unscoped", None),
        "telegram-event-legacy-trusted-shared",
    );
    trusted_shared.route_kind = ConversationRouteKind::Shared;
    let err = services
        .resolve_or_create_binding_with_trusted_scope(
            trusted_shared,
            Some(AgentId::new("agent-alpha").unwrap()),
            Some(ProjectId::new("project-alpha").unwrap()),
            None,
        )
        .await
        .expect_err("the shared route must not re-scope the legacy row either");
    assert!(matches!(err, InboundTurnError::BindingConflict { .. }));

    let legacy_again = services
        .lookup_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-legacy-unscoped", None),
            "telegram-event-legacy-direct-lookup",
        ))
        .await
        .expect("the legacy direct binding is retained untouched");
    assert_eq!(
        legacy_again.turn_scope.thread_id,
        legacy.turn_scope.thread_id
    );
    assert_eq!(legacy_again.turn_scope.agent_id, None);
}

/// Pin changed with the run-acts-as-invoker ruling (#7377), replacing the
/// per-actor "ignore-but-retain" pin (which replaced the trusted-owner
/// backfill pin): shared bindings are conversation-keyed — byte-compatible
/// with legacy pre-upgrade rows — so a Shared resolve RESUMES the existing
/// conversation row instead of forking a per-actor sibling. A trusted owner
/// passed after the fact never re-owns the row, every later paired
/// participant JOINS the one canonical thread acting as themselves, and a
/// Direct probe of the shared conversation stays refused (route-kind
/// mismatch). The restart-path twin with a genuinely operator-owned legacy
/// row lives in `conversation_state_store_contract.rs`
/// (`legacy_operator_owned_shared_binding_resumes_and_is_joined_after_reopen`).
#[tokio::test]
async fn shared_resolve_resumes_legacy_conversation_keyed_binding_without_reowning() {
    let services = InMemoryConversationServices::default();
    for (external, canonical) in [("telegram-user-1", "alice"), ("telegram-user-2", "bob")] {
        services
            .pair_external_actor(
                tenant(),
                telegram(),
                default_installation(),
                external_actor(external),
                user(canonical),
            )
            .await;
    }

    let mut seed = resolve_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-legacy-owner", None),
        "telegram-event-legacy-owner-seed",
    );
    seed.route_kind = ConversationRouteKind::Shared;
    let legacy = services
        .resolve_or_create_binding(seed)
        .await
        .expect("the conversation-keyed shared row (the legacy shape)");
    assert_eq!(
        legacy
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("alice"),
        "a shared thread is owned by whoever bound it first"
    );

    // A later Shared resolve (a distinct event) gets its OWN ephemeral thread;
    // the passed-in trusted subject is ignored on a Shared route — the thread
    // is owned by the pinger, never the trusted subject.
    let mut trusted_owner_shared = resolve_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-legacy-owner", None),
        "telegram-event-legacy-owner-backfill",
    );
    trusted_owner_shared.route_kind = ConversationRouteKind::Shared;
    let resumed = services
        .resolve_or_create_binding_with_trusted_scope(
            trusted_owner_shared,
            None,
            None,
            Some(user("owner-alpha")),
        )
        .await
        .expect("the shared resolve routes via the conversation-keyed binding");
    assert_ne!(
        resumed.turn_scope.thread_id, legacy.turn_scope.thread_id,
        "each ping gets its own ephemeral thread, not a reused one"
    );
    assert_eq!(
        resumed
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("alice"),
        "a Shared route ignores the trusted subject — the thread is owned by the pinger"
    );
    assert_eq!(resumed.actor.user_id, user("alice"));

    // A DIFFERENT paired participant's ping gets THEIR own ephemeral thread,
    // owned by them — there is no shared thread to join.
    let mut bob_request = resolve_request(
        telegram(),
        external_actor("telegram-user-2"),
        external_conversation("chat-legacy-owner", None),
        "telegram-event-legacy-owner-bob",
    );
    bob_request.route_kind = ConversationRouteKind::Shared;
    let bob_resolved = services
        .resolve_or_create_binding(bob_request)
        .await
        .expect("bob's channel ping routes via the conversation-keyed binding");
    assert_ne!(
        bob_resolved.turn_scope.thread_id, legacy.turn_scope.thread_id,
        "each pinger gets their own ephemeral thread, never a shared one"
    );
    assert_eq!(
        bob_resolved
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("bob"),
        "bob's ephemeral thread is owned by bob"
    );
    assert_eq!(bob_resolved.actor.user_id, user("bob"));

    // A Direct-route probe of the same conversation identity is a route-kind
    // mismatch and must not reach the shared row.
    let refused = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-legacy-owner", None),
            "telegram-event-legacy-owner-direct-probe",
        ))
        .await
        .expect_err("a Direct request must not reach the shared row");
    assert!(matches!(refused, InboundTurnError::BindingRequired { .. }));
}

// Removed with ephemeral-per-ping: `shared_route_binds_one_thread_shared_by_actors`
// pinned the retired "one canonical shared thread joined by every paired
// participant" model. There is no shared thread now — each ping resolves onto
// its own pinger-owned ephemeral thread. Distinct-pings-get-distinct-threads
// (with event-idempotent replay across restart) is pinned by
// `shared_channel_pings_get_ephemeral_pinger_owned_threads_idempotent_per_event`
// (conversation_state_store_contract) and
// `stored_shared_reply_target_is_per_event_and_authority_bound_to_the_pinger`;
// owner == actor even with a trusted subject is pinned by
// `shared_route_owner_is_the_actor_even_when_a_trusted_owner_is_passed` below.

/// The trusted-owner parameter exists for host-trusted lanes that bind a
/// conversation FOR a user (the trigger fire path binds Direct conversations
/// owned by the trigger creator). On a SHARED route the owner is always the
/// actor — a configured subject must not be able to claim another user's
/// shared-route thread.
#[tokio::test]
async fn shared_route_owner_is_the_actor_even_when_a_trusted_owner_is_passed() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let mut shared = resolve_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-owner-is-actor", None),
        "telegram-event-owner-is-actor",
    );
    shared.route_kind = ConversationRouteKind::Shared;
    let resolution = services
        .resolve_or_create_binding_with_trusted_scope(shared, None, None, Some(user("operator")))
        .await
        .expect("shared bind succeeds");

    assert_eq!(
        resolution
            .turn_scope
            .explicit_owner_user_id()
            .map(UserId::as_str),
        Some("alice"),
        "a shared-route thread is owned by its actor, never a passed-in subject"
    );
}

#[tokio::test]
async fn pairing_is_scoped_by_tenant_and_adapter_installation() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    let cross_tenant = services
        .resolve_or_create_binding(resolve_request_with(
            TenantId::new("tenant-b").unwrap(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "tenant-b-event-1",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        cross_tenant,
        InboundTurnError::BindingRequired { .. }
    ));

    let cross_installation = services
        .resolve_or_create_binding(resolve_request_with(
            tenant(),
            telegram(),
            AdapterInstallationId::new("other-installation").unwrap(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "other-install-event-1",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        cross_installation,
        InboundTurnError::BindingRequired { .. }
    ));
}

#[tokio::test]
async fn external_ref_keying_cannot_be_collided_with_delimiter_characters() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            ExternalActorRef::new("user;id=x", "y", None::<String>).unwrap(),
            user("alice"),
        )
        .await;

    let colliding_actor = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            ExternalActorRef::new("user", "x;id=y", None::<String>).unwrap(),
            external_conversation("chat-1", None),
            "actor-collision-event",
        ))
        .await
        .unwrap_err();
    assert!(matches!(
        colliding_actor,
        InboundTurnError::BindingRequired { .. }
    ));

    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "a;thread=b", Some("c"), None).unwrap(),
            "conversation-collision-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "a", Some("b;thread=c"), None).unwrap(),
            "conversation-collision-b",
        ))
        .await
        .unwrap();
    assert_ne!(first.turn_scope.thread_id, second.turn_scope.thread_id);
}

#[tokio::test]
async fn per_message_external_ids_do_not_fork_conversation_bindings() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let first = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "chat-1", Some("topic-a"), Some("message-1"))
                .unwrap(),
            "telegram-event-message-1",
        ))
        .await
        .unwrap();
    let second = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            ExternalConversationRef::new(None, "chat-1", Some("topic-a"), Some("message-2"))
                .unwrap(),
            "telegram-event-message-2",
        ))
        .await
        .unwrap();

    assert_eq!(second.resolution.turn_scope, first.resolution.turn_scope);
    assert_eq!(
        second.resolution.source_binding_ref,
        first.resolution.source_binding_ref
    );
    assert_eq!(
        second.resolution.reply_target_binding_ref,
        first.resolution.reply_target_binding_ref
    );
    assert_ne!(
        second.accepted_message.reply_target_binding_ref,
        first.accepted_message.reply_target_binding_ref,
        "accepted inbound messages need message-scoped reply targets even when binding identity is stable"
    );
    let first_target = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            first.resolution.turn_scope.thread_id.clone(),
            first.accepted_message.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap();
    let second_target = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            second.resolution.turn_scope.thread_id.clone(),
            second.accepted_message.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        first_target
            .external_conversation_ref
            .reply_target_message_id(),
        Some("message-1")
    );
    assert_eq!(
        second_target
            .external_conversation_ref
            .reply_target_message_id(),
        Some("message-2")
    );
    assert_eq!(coordinator.submissions().len(), 2);
}

#[tokio::test]
async fn explicit_link_reuses_binding_when_only_external_message_id_changes() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let first = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("workspace-a"),
                "chat-1",
                Some("topic-a"),
                Some("message-1"),
            )
            .unwrap(),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: web_resolution.turn_scope.agent_id.clone(),
            target_project_id: web_resolution.turn_scope.project_id.clone(),
        })
        .await
        .unwrap();
    let replay = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: ExternalConversationRef::new(
                Some("workspace-a"),
                "chat-1",
                Some("topic-a"),
                Some("message-2"),
            )
            .unwrap(),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: web_resolution.turn_scope.thread_id,
            target_agent_id: web_resolution.turn_scope.agent_id,
            target_project_id: web_resolution.turn_scope.project_id,
        })
        .await
        .unwrap();

    assert_eq!(replay.thread_id, first.thread_id);
    assert_eq!(replay.source_binding_ref, first.source_binding_ref);
    assert_eq!(
        replay.reply_target_binding_ref,
        first.reply_target_binding_ref
    );
}

#[tokio::test]
async fn validated_reply_target_preserves_adapter_installation_and_external_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            AdapterInstallationId::new("workspace-a-installation").unwrap(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let conversation_ref = ExternalConversationRef::new(
        Some("workspace-a"),
        "channel-1",
        Some("thread-1"),
        Some("message-1"),
    )
    .unwrap();
    let resolution = services
        .resolve_or_create_binding(resolve_request_with(
            tenant(),
            telegram(),
            AdapterInstallationId::new("workspace-a-installation").unwrap(),
            external_actor("alice-telegram"),
            conversation_ref,
            "telegram-event-1",
        ))
        .await
        .unwrap();

    let target = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            telegram(),
            AdapterInstallationId::new("workspace-a-installation").unwrap(),
            external_actor("alice-telegram"),
            resolution.turn_scope.thread_id.clone(),
            resolution.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap();

    assert_eq!(target.adapter_kind, telegram());
    assert_eq!(
        target.adapter_installation_id,
        AdapterInstallationId::new("workspace-a-installation").unwrap()
    );
    assert_eq!(
        target.external_conversation_ref.space_id(),
        Some("workspace-a")
    );
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "channel-1"
    );
    assert_eq!(
        target.external_conversation_ref.topic_id(),
        Some("thread-1")
    );
    assert_eq!(
        target.external_conversation_ref.reply_target_message_id(),
        None,
        "binding-level reply targets must not preserve stale per-message routing"
    );
}

#[tokio::test]
async fn explicit_link_cannot_cross_tenant_by_reusing_a_thread_id() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            TenantId::new("tenant-b").unwrap(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let tenant_a = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();

    let err = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: TenantId::new("tenant-b").unwrap(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-tenant-b", None),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: tenant_a.turn_scope.thread_id,
            target_agent_id: tenant_a.turn_scope.agent_id,
            target_project_id: tenant_a.turn_scope.project_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::ThreadNotFound { .. }));
}

#[tokio::test]
async fn webui_and_telegram_default_to_separate_threads_for_same_user() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-1",
        ))
        .await
        .unwrap();

    assert_eq!(web_resolution.actor.user_id, user("alice"));
    assert_eq!(telegram_resolution.actor.user_id, user("alice"));
    assert_ne!(
        web_resolution.turn_scope.thread_id, telegram_resolution.turn_scope.thread_id,
        "different product surfaces must not auto-merge conversations for the same user"
    );
}

#[tokio::test]
async fn explicit_link_attaches_conversation_to_existing_thread_after_access_checks() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let link = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: web_resolution.turn_scope.agent_id.clone(),
            target_project_id: web_resolution.turn_scope.project_id.clone(),
        })
        .await
        .unwrap();

    assert_eq!(link.thread_id, web_resolution.turn_scope.thread_id);
    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-2",
        ))
        .await
        .unwrap();
    assert_eq!(
        telegram_resolution.turn_scope.thread_id,
        web_resolution.turn_scope.thread_id
    );
}

#[tokio::test]
async fn repeated_explicit_link_replays_existing_binding_refs() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    let request = LinkConversationRequest {
        tenant_id: tenant(),
        adapter_kind: telegram(),
        adapter_installation_id: default_installation(),
        external_actor_ref: external_actor("alice-telegram"),
        external_conversation_ref: external_conversation("chat-1", None),
        route_kind: ConversationRouteKind::Direct,
        target_thread_id: web_resolution.turn_scope.thread_id.clone(),
        target_agent_id: web_resolution.turn_scope.agent_id.clone(),
        target_project_id: web_resolution.turn_scope.project_id.clone(),
    };

    let first = services
        .link_conversation_to_thread(request.clone())
        .await
        .unwrap();
    let duplicate = services.link_conversation_to_thread(request).await.unwrap();

    assert_eq!(duplicate.source_binding_ref, first.source_binding_ref);
    assert_eq!(
        duplicate.reply_target_binding_ref,
        first.reply_target_binding_ref
    );
}

#[tokio::test]
async fn reset_conversation_binding_rotates_once_and_preserves_old_message_history() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("alice-telegram-reset");
    let conversation = external_conversation("chat-reset", None);
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "reset-before",
        ))
        .await
        .expect("initial binding");
    services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: first.turn_scope.thread_id.clone(),
            actor: first.actor.clone(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: actor.clone(),
            source_binding_ref: first.source_binding_ref.clone(),
            reply_target_binding_ref: first.reply_target_binding_ref.clone(),
            external_conversation_ref: conversation.clone(),
            external_event_id: ExternalEventId::new("reset-message").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:reset-message").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .expect("accepted message before reset");

    let reset_request = ResetConversationRequest {
        resolve_request: resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "reset-command-event",
        ),
        expected_thread_id: first.turn_scope.thread_id.clone(),
    };
    let reset = services
        .reset_conversation_binding(reset_request.clone())
        .await
        .expect("reset binding");

    assert_eq!(reset.previous_thread_id, first.turn_scope.thread_id);
    assert_ne!(
        reset.resolution.turn_scope.thread_id,
        reset.previous_thread_id
    );
    let retained = services.accepted_messages().await;
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0].accepted.thread_id, reset.previous_thread_id,
        "reset must retain prior messages on the previous thread, not move them"
    );
    let resolved = services
        .lookup_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation,
            "reset-after-lookup",
        ))
        .await
        .expect("rotated route remains bound");
    assert_eq!(
        resolved.turn_scope.thread_id,
        reset.resolution.turn_scope.thread_id
    );

    let stale_reply = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            telegram(),
            default_installation(),
            actor,
            reset.previous_thread_id.clone(),
            first.reply_target_binding_ref,
        ))
        .await
        .expect_err("reset must revoke the old delivery ref");
    assert!(matches!(
        stale_reply,
        InboundTurnError::ThreadNotFound { .. }
    ));

    let replay = services
        .reset_conversation_binding(reset_request)
        .await
        .expect("duplicate reset event replays");
    assert_eq!(replay, reset);
}

#[tokio::test]
async fn reset_conversation_binding_rejects_a_stale_expected_thread() {
    let services = InMemoryConversationServices::default();
    let actor = external_actor("alice-telegram-stale-reset");
    let conversation = external_conversation("chat-stale-reset", None);
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            actor.clone(),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            actor.clone(),
            conversation.clone(),
            "stale-reset-before",
        ))
        .await
        .expect("initial binding");
    services
        .reset_conversation_binding(ResetConversationRequest {
            resolve_request: resolve_request(
                telegram(),
                actor.clone(),
                conversation.clone(),
                "stale-reset-first-command",
            ),
            expected_thread_id: first.turn_scope.thread_id.clone(),
        })
        .await
        .expect("first reset");

    let error = services
        .reset_conversation_binding(ResetConversationRequest {
            resolve_request: resolve_request(
                telegram(),
                actor,
                conversation,
                "stale-reset-second-command",
            ),
            expected_thread_id: first.turn_scope.thread_id.clone(),
        })
        .await
        .expect_err("stale expected thread must fail closed");
    assert!(matches!(error, InboundTurnError::BindingConflict { .. }));
}

#[tokio::test]
async fn explicit_link_refuses_to_retarget_existing_conversation_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let first_thread = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session-a", None),
            "web-event-a",
        ))
        .await
        .unwrap();
    let second_thread = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session-b", None),
            "web-event-b",
        ))
        .await
        .unwrap();
    services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: first_thread.turn_scope.thread_id,
            target_agent_id: first_thread.turn_scope.agent_id,
            target_project_id: first_thread.turn_scope.project_id,
        })
        .await
        .unwrap();

    let err = services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: second_thread.turn_scope.thread_id,
            target_agent_id: second_thread.turn_scope.agent_id,
            target_project_id: second_thread.turn_scope.project_id,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::BindingConflict { .. }));
}

#[tokio::test]
async fn first_bind_does_not_trust_unvalidated_requested_scope_hints() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;

    let resolution = services
        .resolve_or_create_binding(ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant(),
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            external_conversation_ref: external_conversation("browser-session", None),
            route_kind: ConversationRouteKind::Direct,
            external_event_id: ExternalEventId::new("web-event-scope-hint").unwrap(),
            requested_agent_id: Some(AgentId::new("spoofed-agent").unwrap()),
            requested_project_id: Some(ProjectId::new("spoofed-project").unwrap()),
        })
        .await
        .unwrap();

    assert_eq!(resolution.turn_scope.agent_id, None);
    assert_eq!(resolution.turn_scope.project_id, None);
}

#[tokio::test]
async fn duplicate_external_event_on_different_route_fails_before_second_submit() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "installation-wide-event-1",
        ))
        .await
        .unwrap();

    let err = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-2", None),
            "installation-wide-event-1",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn explicit_link_uses_existing_thread_scope_not_spoofed_link_scope() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;

    let web_resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("browser-session", None),
            "web-event-1",
        ))
        .await
        .unwrap();
    services
        .link_conversation_to_thread(LinkConversationRequest {
            tenant_id: tenant(),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            external_conversation_ref: external_conversation("chat-1", None),
            route_kind: ConversationRouteKind::Direct,
            target_thread_id: web_resolution.turn_scope.thread_id.clone(),
            target_agent_id: Some(AgentId::new("spoofed-agent").unwrap()),
            target_project_id: Some(ProjectId::new("spoofed-project").unwrap()),
        })
        .await
        .unwrap();

    let telegram_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("chat-1", None),
            "telegram-event-2",
        ))
        .await
        .unwrap();
    assert_eq!(telegram_resolution.turn_scope, web_resolution.turn_scope);
}

#[tokio::test]
async fn duplicate_retry_after_submit_failure_survives_pairing_churn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(FailFirstTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-pairing-churn", None),
        "telegram-event-pairing-churn",
    );

    let err = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::TurnSubmissionFailed { .. }));
    services
        .unpair_external_actor(
            &tenant(),
            &telegram(),
            &default_installation(),
            &external_actor("telegram-user-1"),
        )
        .await;

    let retry = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(
        retry.accepted_message.idempotency,
        MessageIdempotencyStatus::Duplicate
    );
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[1].actor,
        TurnActor::new(user("alice"))
    );
}

#[tokio::test]
async fn duplicate_external_event_after_submit_failure_reuses_original_actor() {
    let binding = DriftBindingService::new();
    let session =
        FixedMessageSessionService::new(AcceptedMessageRef::new("message:drift").unwrap());
    let coordinator = Arc::new(FailFirstTurnCoordinator::default());
    let inbound = InboundTurnService::new(binding, session, coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("shared-group-actor"),
        external_conversation("group-chat", None),
        "shared-event-retry",
    );

    let err = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::TurnSubmissionFailed { .. }));

    inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].actor,
        TurnActor::new(user("alice"))
    );
    assert_eq!(
        coordinator.submissions()[1].actor,
        TurnActor::new(user("alice")),
        "duplicate retry must reuse the accepted message actor, not the current resolver actor"
    );
}

#[tokio::test]
async fn permanent_turn_error_does_not_rotate_submit_idempotency_key() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(PermanentFailureTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-permanent-error",
    );

    let first = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    let second = inbound.handle_inbound_turn(request).await.unwrap_err();

    assert!(matches!(
        first,
        InboundTurnError::TurnSubmissionFailed { .. }
    ));
    assert!(matches!(
        second,
        InboundTurnError::TurnSubmissionFailed { .. }
    ));
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].idempotency_key,
        coordinator.submissions()[1].idempotency_key,
        "permanent turn errors should keep the original submit idempotency key for replay"
    );
}

#[tokio::test]
async fn capacity_exceeded_does_not_rotate_submit_idempotency_key() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(CapacityFailureTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-capacity-error",
    );

    let first = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    let second = inbound.handle_inbound_turn(request).await.unwrap_err();

    assert!(matches!(
        first,
        InboundTurnError::TurnSubmissionFailed { .. }
    ));
    assert!(matches!(
        second,
        InboundTurnError::TurnSubmissionFailed { .. }
    ));
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].idempotency_key,
        coordinator.submissions()[1].idempotency_key,
        "capacity errors should keep the original submit idempotency key for replay"
    );
}

#[tokio::test]
async fn turn_submission_failure_preserves_structured_turn_error() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(PermanentFailureTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator);

    let err = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "telegram-event-structured-error",
        ))
        .await
        .unwrap_err();

    let InboundTurnError::TurnSubmissionFailed { error } = err else {
        panic!("expected structured turn submission failure");
    };
    assert_eq!(
        error.category(),
        TurnSubmissionErrorCategory::InvalidRequest
    );
    assert_eq!(error.adapter_status_code(), 400);
    assert_eq!(error.retry(), TurnSubmissionRetry::Permanent);
    assert_eq!(
        error.to_string(),
        "invalid turn request: permanent invalid request",
        "the host's rendered cause must survive the port boundary verbatim"
    );
}

#[tokio::test]
async fn duplicate_external_event_after_transient_submit_failure_retries_same_message_ref() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(FailFirstTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-transient",
    );

    let err = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::TurnSubmissionFailed { .. }));
    assert_eq!(services.accepted_messages().await.len(), 1);
    assert_eq!(coordinator.submissions().len(), 1);

    let retry = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(services.accepted_messages().await.len(), 1);
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].accepted_message_ref,
        coordinator.submissions()[1].accepted_message_ref,
        "adapter retry must reuse the accepted message ref instead of getting stuck after a pre-submit failure"
    );
    assert!(retry.turn_submission.is_some());
}

#[tokio::test]
async fn busy_thread_retry_uses_fresh_submit_key_for_same_accepted_message() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(BusyFirstUniqueKeyCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let mut request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-busy-retry",
    );
    request.requested_run_profile = Some(RunProfileRequest::new("fast-profile").unwrap());
    let original_received_at = request.received_at;

    let err = inbound
        .handle_inbound_turn(request.clone())
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::TurnSubmissionFailed { .. }));

    request.received_at = Utc.with_ymd_and_hms(2026, 5, 6, 12, 30, 0).unwrap();
    request.requested_run_profile = Some(RunProfileRequest::new("slow-profile").unwrap());
    let retry = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(services.accepted_messages().await.len(), 1);
    assert_eq!(coordinator.submissions().len(), 2);
    assert_eq!(
        coordinator.submissions()[0].accepted_message_ref,
        coordinator.submissions()[1].accepted_message_ref
    );
    assert_ne!(
        coordinator.submissions()[0].idempotency_key,
        coordinator.submissions()[1].idempotency_key,
        "busy/admission idempotency replays must not strand the accepted inbound message forever"
    );
    assert_eq!(
        coordinator.submissions()[1].received_at,
        original_received_at
    );
    assert_eq!(
        coordinator.submissions()[1].requested_run_profile,
        Some(RunProfileRequest::new("fast-profile").unwrap())
    );
    assert!(retry.turn_submission.is_some());
}

#[tokio::test]
async fn max_length_accepted_message_ref_is_valid_as_submit_idempotency_key() {
    let binding = InMemoryConversationServices::default();
    binding
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let long_ref = "m".repeat(256);
    let session =
        FixedMessageSessionService::new(AcceptedMessageRef::new(long_ref.clone()).unwrap());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(binding, session, coordinator.clone());

    inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "telegram-event-long-ref",
        ))
        .await
        .unwrap();

    assert_eq!(coordinator.submissions().len(), 1);
    assert_eq!(
        coordinator.submissions()[0].idempotency_key.as_str(),
        long_ref
    );
}

#[tokio::test]
async fn duplicate_external_event_replays_message_and_does_not_submit_duplicate_turn() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());
    let request = inbound_request(
        telegram(),
        external_actor("telegram-user-1"),
        external_conversation("chat-1", None),
        "telegram-event-1",
    );

    let first = inbound.handle_inbound_turn(request.clone()).await.unwrap();
    let duplicate = inbound.handle_inbound_turn(request).await.unwrap();

    assert_eq!(
        duplicate.accepted_message.idempotency,
        MessageIdempotencyStatus::Duplicate
    );
    assert_eq!(
        duplicate.accepted_message.message_ref,
        first.accepted_message.message_ref
    );
    assert_eq!(coordinator.submissions().len(), 1);
    assert_eq!(duplicate.turn_submission, first.turn_submission);
    assert!(duplicate.turn_submission.is_some());
}

#[tokio::test]
async fn direct_route_rejects_borrowed_owner_actor_key() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("bob-web"),
            user("bob"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-private-borrowed-key", None),
            "alice-borrowed-key-event",
        ))
        .await
        .unwrap();

    let err = services
        .validate_reply_target(validate_reply_request(
            user("bob"),
            web(),
            default_installation(),
            external_actor("alice-web"),
            resolution.turn_scope.thread_id.clone(),
            resolution.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id,
            actor: TurnActor::new(user("bob")),
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-private-borrowed-key", None),
            external_event_id: ExternalEventId::new("bob-borrowed-key-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:bob-borrowed-key-event").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn failed_shared_route_probe_is_denied_and_never_reclassifies_direct_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("bob-web"),
            user("bob"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("charlie-web"),
            user("charlie"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-direct-probe", None),
            "alice-direct-probe-event",
        ))
        .await
        .unwrap();

    // Pin changed with the run-acts-as-invoker ruling (#7377): the widen
    // mutation this test once guarded no longer exists in any form. Bindings
    // are conversation-keyed, so bob's Shared-route probe lands on alice's
    // Direct-born row — and is refused outright, even though he was added to
    // the thread's participant set: a record's access class is fixed at
    // birth, and Direct-born rows never admit non-owners on any route.
    let mut bob_probe = resolve_request(
        web(),
        external_actor("bob-web"),
        external_conversation("alice-direct-probe", None),
        "bob-probe-event",
    );
    bob_probe.route_kind = ConversationRouteKind::Shared;
    let err = services
        .resolve_or_create_binding(bob_probe)
        .await
        .expect_err("a shared probe against a Direct-born binding is refused");
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    // The failed probe reclassified nothing: another participant's Shared
    // probe is denied the same way afterwards…
    let mut charlie_probe = resolve_request(
        web(),
        external_actor("charlie-web"),
        external_conversation("alice-direct-probe", None),
        "charlie-after-failed-probe",
    );
    charlie_probe.route_kind = ConversationRouteKind::Shared;
    let err = services
        .resolve_or_create_binding(charlie_probe)
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    // …and the owner's Direct route keeps resolving the untouched binding.
    let owner_again = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-direct-probe", None),
            "alice-after-failed-probes",
        ))
        .await
        .expect("the direct binding still resolves for its owner");
    assert_eq!(
        owner_again.turn_scope.thread_id,
        resolution.turn_scope.thread_id
    );
}

#[tokio::test]
async fn lookup_binding_shared_probe_never_reclassifies_direct_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("bob-web"),
            user("bob"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-direct-lookup-probe", None),
            "alice-direct-lookup-probe-event",
        ))
        .await
        .unwrap();

    // Pin changed with the run-acts-as-invoker ruling (#7377): the widen
    // mutation is gone and lookups never mutate anything. The OWNER's
    // Shared-marked lookup reaches their own conversation-keyed row (owner
    // allowance) without reclassifying it…
    let mut owner_probe = resolve_request(
        web(),
        external_actor("alice-web"),
        external_conversation("alice-direct-lookup-probe", None),
        "alice-direct-lookup-owner-shared-probe",
    );
    owner_probe.route_kind = ConversationRouteKind::Shared;
    let owner_probe = services
        .lookup_binding(owner_probe)
        .await
        .expect("the owner addresses their own binding on either route kind");
    assert_eq!(
        owner_probe.turn_scope.thread_id,
        resolution.turn_scope.thread_id
    );

    // …so bob — a member of the thread's participant set, but not the
    // binding's owner — is still denied on the Shared route afterwards: the
    // record's access class is immutable, and Direct-born rows never admit
    // non-owners.
    let mut bob_probe = resolve_request(
        web(),
        external_actor("bob-web"),
        external_conversation("alice-direct-lookup-probe", None),
        "bob-direct-lookup-shared-probe",
    );
    bob_probe.route_kind = ConversationRouteKind::Shared;
    let err = services
        .lookup_binding(bob_probe)
        .await
        .expect_err("a non-owner shared lookup against a Direct-born binding is denied");
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    // The direct binding is untouched: its owner still resolves it directly.
    let direct_again = services
        .lookup_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-direct-lookup-probe", None),
            "alice-direct-lookup-after-shared-probe",
        ))
        .await
        .expect("the direct binding still resolves for its owner");
    assert_eq!(
        direct_again.turn_scope.thread_id,
        resolution.turn_scope.thread_id
    );
}

#[tokio::test]
async fn shared_route_rejects_wrong_adapter_context() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let mut request = resolve_request(
        telegram(),
        external_actor("alice-telegram"),
        external_conversation("shared-adapter-route", None),
        "shared-adapter-event",
    );
    request.route_kind = ConversationRouteKind::Shared;
    let resolution = services.resolve_or_create_binding(request).await.unwrap();

    let err = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            web(),
            default_installation(),
            external_actor("alice-web"),
            resolution.turn_scope.thread_id.clone(),
            resolution.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("shared-adapter-route", None),
            external_event_id: ExternalEventId::new("wrong-adapter-accept-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:wrong-adapter-accept-event")
                .unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn direct_route_rejects_same_user_different_external_actor_alias() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-primary"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-secondary"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator.clone());

    let first = inbound
        .handle_inbound_turn(inbound_request(
            web(),
            external_actor("alice-primary"),
            external_conversation("alice-private", None),
            "primary-event",
        ))
        .await
        .unwrap();

    let err = inbound
        .handle_inbound_turn(inbound_request(
            web(),
            external_actor("alice-secondary"),
            external_conversation("alice-private", None),
            "secondary-event",
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));

    let err = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            web(),
            default_installation(),
            external_actor("alice-secondary"),
            first.resolution.turn_scope.thread_id.clone(),
            first.accepted_message.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn duplicate_external_event_route_is_reserved_before_binding_creation() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user("alice"),
        )
        .await;

    services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-1", None),
            "installation-event-before-accept",
        ))
        .await
        .unwrap();

    let err = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("telegram-user-1"),
            external_conversation("chat-2", None),
            "installation-event-before-accept",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn failed_resolve_does_not_reserve_external_event_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("bob-telegram"),
            user("bob"),
        )
        .await;
    let _alice_direct = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("alice-direct-poison-source", None),
            "alice-direct-poison-source-event",
        ))
        .await
        .unwrap();

    // Per-actor shared threads: bob's shared resolve no longer fails against
    // alice's direct binding (it binds his own thread), so the failing probe
    // this test needs is an UNPAIRED actor's — the fail-closed arm every
    // route shape still has.
    let mut denied = resolve_request(
        telegram(),
        external_actor("mallory-telegram"),
        external_conversation("alice-direct-poison-source", None),
        "denied-event-must-not-reserve-route",
    );
    denied.route_kind = ConversationRouteKind::Shared;
    assert!(matches!(
        services
            .resolve_or_create_binding(denied)
            .await
            .unwrap_err(),
        InboundTurnError::BindingRequired { .. }
    ));

    let legitimate = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            external_conversation("alice-legitimate-after-denied", None),
            "denied-event-must-not-reserve-route",
        ))
        .await
        .unwrap();
    assert_eq!(legitimate.actor, TurnActor::new(user("alice")));
}

// Removed with ephemeral-per-ping:
// `shared_route_marker_on_direct_born_binding_never_admits_other_participants`
// pinned the retired shared-thread admission model (an owner's Shared marker
// reaching a shared thread that manually-seeded participants join). There is no
// shared thread now; that a binding's access class is fixed at birth (no widen
// mutation) is pinned structurally by `ReplyRouteAccess::allows` and by
// `reborn_retired_taxonomy.rs`, and
// `bound_group_message_from_non_participant_is_denied` below covers
// non-participant refusal.

// Note: `shared_group_participant_can_send_on_existing_binding` retired with
// the ephemeral-per-ping remodel (#7377). Its whole point was the retired
// shared-thread JOIN model — a second actor (bob) manually added to the
// participant set could then send on the FIRST binder's shared binding. There
// is no shared thread now: each ping mints its own event-keyed, pinger-owned
// thread at the product layer, and a foreign actor hitting an existing shared
// binding is refused (`bound_group_message_from_non_participant_is_denied`
// below still pins that refusal).

#[tokio::test]
async fn bound_group_message_from_non_participant_is_denied() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("bob-telegram"),
            user("bob"),
        )
        .await;
    let group = external_conversation("group-1", Some("topic-a"));
    let alice_resolution = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("alice-telegram"),
            group.clone(),
            "group-event-1",
        ))
        .await
        .unwrap();
    assert_eq!(alice_resolution.access, ThreadAccessDecision::Allowed);

    let err = services
        .resolve_or_create_binding(resolve_request(
            telegram(),
            external_actor("bob-telegram"),
            group,
            "group-event-2",
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn reply_target_validation_rejects_same_thread_different_actor_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser", None),
            "alice-web-event-owner",
        ))
        .await
        .unwrap();

    let err = services
        .validate_reply_target(validate_reply_request(
            user("bob"),
            web(),
            default_installation(),
            external_actor("bob-web"),
            resolution.turn_scope.thread_id.clone(),
            resolution.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn accept_inbound_message_rejects_stale_message_scoped_reply_ref() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            telegram(),
            default_installation(),
            external_actor("alice-telegram"),
            user("alice"),
        )
        .await;
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = InboundTurnService::new(services.clone(), services.clone(), coordinator);
    let group = external_conversation("stale-reply-ref-group", Some("topic-a"));
    let first = inbound
        .handle_inbound_turn(inbound_request(
            telegram(),
            external_actor("alice-telegram"),
            group.clone(),
            "stale-reply-ref-first",
        ))
        .await
        .unwrap();
    let mut widen = inbound_request(
        telegram(),
        external_actor("alice-telegram"),
        group.clone(),
        "stale-reply-ref-widen",
    );
    widen.route_kind = ConversationRouteKind::Shared;
    inbound.handle_inbound_turn(widen).await.unwrap();

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: first.resolution.turn_scope.thread_id,
            actor: first.resolution.actor,
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-telegram"),
            source_binding_ref: first.resolution.source_binding_ref,
            reply_target_binding_ref: first.accepted_message.reply_target_binding_ref,
            external_conversation_ref: group,
            external_event_id: ExternalEventId::new("stale-reply-ref-next").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:stale-reply-ref-next").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 2, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn message_scoped_reply_target_rejects_same_thread_different_actor_route() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser", None),
            "alice-web-event-message-owner",
        ))
        .await
        .unwrap();
    let accepted = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id.clone(),
            actor: resolution.actor,
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: ExternalConversationRef::new(
                None,
                "alice-browser",
                None,
                Some("message-1"),
            )
            .unwrap(),
            external_event_id: ExternalEventId::new("alice-web-event-message-owner").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:alice-web-event-message-owner")
                .unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap();

    let err = services
        .validate_reply_target(validate_reply_request(
            user("bob"),
            web(),
            default_installation(),
            external_actor("bob-web"),
            accepted.thread_id.clone(),
            accepted.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn reply_target_validation_rejects_same_actor_wrong_thread_refs() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-b", None),
            "alice-event-b",
        ))
        .await
        .unwrap();

    let err = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            web(),
            default_installation(),
            external_actor("alice-web"),
            first.turn_scope.thread_id.clone(),
            second.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn accept_inbound_message_rejects_external_route_mismatch() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-browser-b", None),
            external_event_id: ExternalEventId::new("route-mismatch-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:route-mismatch-event").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn duplicate_accept_rejects_external_route_mismatch() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();

    services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id.clone(),
            actor: resolution.actor.clone(),
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref.clone(),
            reply_target_binding_ref: resolution.reply_target_binding_ref.clone(),
            external_conversation_ref: external_conversation("alice-browser-a", None),
            external_event_id: ExternalEventId::new("duplicate-route-mismatch-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:duplicate-route-mismatch-event")
                .unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap();

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-browser-b", None),
            external_event_id: ExternalEventId::new("duplicate-route-mismatch-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:duplicate-route-mismatch-event")
                .unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 2, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[tokio::test]
async fn accept_inbound_message_rejects_mixed_source_and_reply_bindings() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    let first = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-a", None),
            "alice-event-a",
        ))
        .await
        .unwrap();
    let second = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser-b", None),
            "alice-event-b",
        ))
        .await
        .unwrap();

    let err = services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant(),
            thread_id: first.turn_scope.thread_id,
            actor: first.actor,
            adapter_kind: web(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("alice-web"),
            source_binding_ref: first.source_binding_ref,
            reply_target_binding_ref: second.reply_target_binding_ref,
            external_conversation_ref: external_conversation("alice-browser-a", None),
            external_event_id: ExternalEventId::new("mixed-binding-event").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:mixed-binding-event").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 1, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .unwrap_err();

    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

#[test]
fn serde_deserialization_revalidates_external_ref_invariants() {
    assert!(serde_json::from_str::<AdapterKind>("\"\"").is_err());
    assert!(
        serde_json::from_str::<AdapterInstallationId>(&format!("\"{}\"", "x".repeat(513))).is_err()
    );
    assert!(serde_json::from_str::<ExternalEventId>("\"event\\u0000id\"").is_err());
    assert!(serde_json::from_str::<InboundMessageContentRef>("\"\"").is_err());
    assert!(serde_json::from_str::<ExternalActorBindingEpoch>("\"\"").is_err());
    assert!(serde_json::from_str::<ExternalActorBindingEpoch>("\"bad\\u0000epoch\"").is_err());
    // The external actor/conversation pair is now the canonical
    // `ironclaw_extension_contracts` type, so these invariants are asserted
    // against that type's field spelling (`topic_id` /
    // `reply_target_message_id`). The *durable* spelling this crate's own
    // records use — which still accepts the pre-unification `thread_id` /
    // `message_id` — is pinned by `stored_refs::tests`.
    assert!(serde_json::from_str::<ExternalActorRef>(r#"{"kind":"user","id":""}"#).is_err());
    assert!(
        serde_json::from_str::<ExternalConversationRef>(
            r#"{"space_id":null,"conversation_id":"chat-1","topic_id":"ok","reply_target_message_id":"bad\u0001"}"#
        )
        .is_err()
    );
    assert!(
        serde_json::from_str::<ExternalConversationIdentity>(
            r#"{"space_id":null,"conversation_id":"","topic_id":null}"#
        )
        .is_err()
    );
    // The route identity keeps reading the pre-unification `thread_id` key, so
    // durable binding keys written by released builds resolve unchanged.
    assert!(
        serde_json::from_str::<ExternalConversationIdentity>(
            r#"{"space_id":null,"conversation_id":"chat-1","thread_id":"topic-9"}"#
        )
        .is_ok()
    );
}

#[tokio::test]
async fn reply_target_validation_is_scoped_to_actor_and_binding() {
    let services = InMemoryConversationServices::default();
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("alice-web"),
            user("alice"),
        )
        .await;
    services
        .pair_external_actor(
            tenant(),
            web(),
            default_installation(),
            external_actor("bob-web"),
            user("bob"),
        )
        .await;
    let alice = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("alice-web"),
            external_conversation("alice-browser", None),
            "alice-event-1",
        ))
        .await
        .unwrap();
    let bob = services
        .resolve_or_create_binding(resolve_request(
            web(),
            external_actor("bob-web"),
            external_conversation("bob-browser", None),
            "bob-event-1",
        ))
        .await
        .unwrap();

    let target = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            web(),
            default_installation(),
            external_actor("alice-web"),
            alice.turn_scope.thread_id.clone(),
            alice.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(
        target.external_conversation_ref.conversation_id(),
        "alice-browser"
    );

    let err = services
        .validate_reply_target(validate_reply_request(
            user("alice"),
            web(),
            default_installation(),
            external_actor("alice-web"),
            bob.turn_scope.thread_id.clone(),
            bob.reply_target_binding_ref.clone(),
        ))
        .await
        .unwrap_err();
    assert!(matches!(err, InboundTurnError::AccessDenied { .. }));
}

fn validate_reply_request(
    actor_user_id: UserId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
    current_thread_id: ThreadId,
    reply_target_binding_ref: ReplyTargetBindingRef,
) -> ValidateReplyTargetRequest {
    ValidateReplyTargetRequest {
        tenant_id: tenant(),
        actor_user_id,
        adapter_kind,
        adapter_installation_id,
        external_actor_ref,
        current_thread_id,
        reply_target_binding_ref,
    }
}

fn inbound_request(
    adapter_kind: AdapterKind,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> InboundTurnRequest {
    InboundTurnRequest {
        tenant_id: tenant(),
        adapter_kind,
        adapter_installation_id: default_installation(),
        external_actor_ref,
        external_conversation_ref,
        external_event_id: ExternalEventId::new(external_event_id).unwrap(),
        route_kind: ConversationRouteKind::Direct,
        content_ref: InboundMessageContentRef::new(format!("content:{external_event_id}")).unwrap(),
        requested_agent_id: Some(agent()),
        requested_project_id: Some(project()),
        received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
        requested_run_profile: None,
    }
}

fn resolve_request(
    adapter_kind: AdapterKind,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> ironclaw_conversations::ResolveConversationRequest {
    resolve_request_with(
        tenant(),
        adapter_kind,
        default_installation(),
        external_actor_ref,
        external_conversation_ref,
        external_event_id,
    )
}

fn resolve_request_with(
    tenant_id: TenantId,
    adapter_kind: AdapterKind,
    adapter_installation_id: AdapterInstallationId,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    external_event_id: &str,
) -> ironclaw_conversations::ResolveConversationRequest {
    ironclaw_conversations::ResolveConversationRequest {
        tenant_id,
        adapter_kind,
        adapter_installation_id,
        external_actor_ref,
        external_conversation_ref,
        external_event_id: ExternalEventId::new(external_event_id).unwrap(),
        route_kind: ConversationRouteKind::Direct,
        requested_agent_id: Some(agent()),
        requested_project_id: Some(project()),
    }
}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").unwrap()
}

fn user(id: &str) -> UserId {
    UserId::new(id).unwrap()
}

fn agent() -> AgentId {
    AgentId::new("agent-a").unwrap()
}

fn project() -> ProjectId {
    ProjectId::new("project-a").unwrap()
}

fn telegram() -> AdapterKind {
    AdapterKind::new("telegram").unwrap()
}

fn web() -> AdapterKind {
    AdapterKind::new("web").unwrap()
}

fn default_installation() -> AdapterInstallationId {
    AdapterInstallationId::new("default-installation").unwrap()
}

fn external_actor(id: &str) -> ExternalActorRef {
    ExternalActorRef::new("user", id, None::<String>).unwrap()
}

fn external_conversation(
    conversation_id: &str,
    thread_id: Option<&str>,
) -> ExternalConversationRef {
    ExternalConversationRef::new(None, conversation_id, thread_id, None).unwrap()
}

struct FixedMessageSessionService {
    message_ref: AcceptedMessageRef,
    accepted: Mutex<Option<AcceptedConversationMessage>>,
    submitted: Mutex<Option<SubmitTurnResponse>>,
}

impl FixedMessageSessionService {
    fn new(message_ref: AcceptedMessageRef) -> Self {
        Self {
            message_ref,
            accepted: Mutex::new(None),
            submitted: Mutex::new(None),
        }
    }
}

#[async_trait]
impl InboundConversationService for FixedMessageSessionService {
    async fn accept_inbound_message(
        &self,
        request: AcceptConversationMessageRequest,
    ) -> Result<AcceptedConversationMessage, InboundTurnError> {
        let mut accepted = self.accepted.lock().unwrap();
        if let Some(existing) = accepted.clone() {
            let mut duplicate = existing;
            duplicate.idempotency = MessageIdempotencyStatus::Duplicate;
            return Ok(duplicate);
        }
        let message = AcceptedConversationMessage {
            tenant_id: request.tenant_id,
            thread_id: request.thread_id,
            actor: request.actor,
            message_ref: self.message_ref.clone(),
            source_binding_ref: request.source_binding_ref,
            reply_target_binding_ref: request.reply_target_binding_ref,
            received_at: request.received_at,
            requested_run_profile: request.requested_run_profile,
            idempotency: MessageIdempotencyStatus::Inserted,
        };
        *accepted = Some(message.clone());
        Ok(message)
    }

    async fn replay_accepted_inbound_message(
        &self,
        _lookup: AcceptedConversationMessageLookup,
    ) -> Result<Option<AcceptedConversationMessageReplay>, InboundTurnError> {
        Ok(None)
    }

    async fn inbound_message_turn_submission(
        &self,
        _message_ref: &AcceptedMessageRef,
    ) -> Result<Option<SubmitTurnResponse>, InboundTurnError> {
        Ok(self.submitted.lock().unwrap().clone())
    }

    async fn inbound_message_turn_submission_key(
        &self,
        message_ref: &AcceptedMessageRef,
    ) -> Result<IdempotencyKey, InboundTurnError> {
        IdempotencyKey::new(message_ref.as_str().to_string())
            .map_err(|reason| InboundTurnError::InvalidCanonicalRef { reason })
    }

    async fn rotate_inbound_message_turn_submission_key(
        &self,
        _message_ref: &AcceptedMessageRef,
    ) -> Result<(), InboundTurnError> {
        Ok(())
    }

    async fn mark_inbound_message_turn_submitted(
        &self,
        _message_ref: &AcceptedMessageRef,
        response: SubmitTurnResponse,
    ) -> Result<(), InboundTurnError> {
        *self.submitted.lock().unwrap() = Some(response);
        Ok(())
    }
}

#[derive(Clone)]
struct DriftBindingService {
    calls: Arc<Mutex<usize>>,
}

impl DriftBindingService {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait]
impl ConversationBindingService for DriftBindingService {
    async fn resolve_or_create_binding(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        let user_id = if *calls == 1 {
            user("alice")
        } else {
            user("bob")
        };
        Ok(ConversationBindingResolution {
            tenant_id: request.tenant_id.clone(),
            actor: TurnActor::new(user_id),
            binding_epoch: None,
            turn_scope: TurnScope::new(
                request.tenant_id,
                None,
                None,
                ThreadId::new("shared-thread").unwrap(),
            ),
            source_binding_ref: SourceBindingRef::new("source:shared").unwrap(),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:shared").unwrap(),
            access: ThreadAccessDecision::Allowed,
        })
    }

    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
        _trusted_agent_id: Option<AgentId>,
        _trusted_project_id: Option<ProjectId>,
        _trusted_owner_user_id: Option<UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.resolve_or_create_binding(request).await
    }

    async fn lookup_binding(
        &self,
        _request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        unimplemented!("not used by inbound service tests")
    }

    async fn link_conversation_to_thread(
        &self,
        _request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError> {
        unimplemented!("not used by inbound service tests")
    }

    async fn validate_reply_target(
        &self,
        _request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
        unimplemented!("not used by inbound service tests")
    }
}

#[derive(Clone)]
struct UntrustedOnlyBindingService {
    inner: InMemoryConversationServices,
    resolve_requests: Arc<Mutex<Vec<ironclaw_conversations::ResolveConversationRequest>>>,
    untrusted_calls: Arc<Mutex<usize>>,
    trusted_calls: Arc<Mutex<usize>>,
}

impl UntrustedOnlyBindingService {
    fn new(inner: InMemoryConversationServices) -> Self {
        Self {
            inner,
            resolve_requests: Arc::new(Mutex::new(Vec::new())),
            untrusted_calls: Arc::new(Mutex::new(0)),
            trusted_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn untrusted_calls(&self) -> usize {
        *self.untrusted_calls.lock().unwrap()
    }

    fn trusted_calls(&self) -> usize {
        *self.trusted_calls.lock().unwrap()
    }

    fn resolve_requests(&self) -> Vec<ironclaw_conversations::ResolveConversationRequest> {
        self.resolve_requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ConversationBindingService for UntrustedOnlyBindingService {
    async fn resolve_or_create_binding(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        *self.untrusted_calls.lock().unwrap() += 1;
        self.resolve_requests.lock().unwrap().push(request.clone());
        self.inner.resolve_or_create_binding(request).await
    }

    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        _request: ironclaw_conversations::ResolveConversationRequest,
        _trusted_agent_id: Option<AgentId>,
        _trusted_project_id: Option<ProjectId>,
        _trusted_owner_user_id: Option<UserId>,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        *self.trusted_calls.lock().unwrap() += 1;
        panic!("untrusted inbound must not call trusted resolver path")
    }

    async fn lookup_binding(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<ConversationBindingResolution, InboundTurnError> {
        self.inner.lookup_binding(request).await
    }

    async fn link_conversation_to_thread(
        &self,
        request: LinkConversationRequest,
    ) -> Result<LinkedConversationBinding, InboundTurnError> {
        self.inner.link_conversation_to_thread(request).await
    }

    async fn validate_reply_target(
        &self,
        request: ValidateReplyTargetRequest,
    ) -> Result<ReplyTargetBinding, InboundTurnError> {
        self.inner.validate_reply_target(request).await
    }
}

#[derive(Default)]
struct RecordingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl RecordingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct FailFirstTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

#[derive(Default)]
struct BusyFirstUniqueKeyCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

#[derive(Default)]
struct PermanentFailureTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

#[derive(Default)]
struct CapacityFailureTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
}

impl BusyFirstUniqueKeyCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

impl FailFirstTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

impl PermanentFailureTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

impl CapacityFailureTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

/// Mirror of the production port adapter
/// (`ironclaw_composition::automation::conversation_turn_submitter`): it
/// derives the owner and resolves the classification through the same
/// `product_context::resolve_inbound` call, producing the same
/// `SubmitTurnRequest` the real adapter hands the coordinator. The fakes below
/// record that request, so every product-context, run-profile and
/// idempotency-key assertion in this file keeps asserting on the exact value a
/// coordinator receives rather than a paraphrase of it. The production copy is
/// pinned by that adapter's own seam tests.
fn submit_turn_request(submission: ConversationTurnSubmission) -> SubmitTurnRequest {
    let is_trusted_trigger = matches!(
        submission.classification,
        ConversationInboundClassification::TrustedTrigger
    );
    let mut product_context = product_context::resolve_inbound(
        inbound_classification(submission.classification),
        submission.origin_adapter,
        submission.surface_type,
        submission.scope.product_owner(&submission.actor),
    );
    if is_trusted_trigger {
        product_context.execution_policy = submission.execution_policy;
    }
    SubmitTurnRequest {
        requested_model: None,
        output_contract: None,
        scope: submission.scope,
        actor: submission.actor,
        accepted_message_ref: submission.accepted_message_ref,
        requested_run_profile: submission.requested_run_profile,
        idempotency_key: submission.idempotency_key,
        received_at: submission.received_at,
        requested_run_id: None,
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: Some(product_context),
    }
}

fn inbound_classification(
    classification: ConversationInboundClassification,
) -> product_context::InboundClassification {
    match classification {
        ConversationInboundClassification::TrustedTrigger => {
            product_context::InboundClassification::TrustedTrigger
        }
        ConversationInboundClassification::TrustedOther => {
            product_context::InboundClassification::TrustedOther
        }
        ConversationInboundClassification::Untrusted => {
            product_context::InboundClassification::Untrusted
        }
    }
}

/// The submission port's rendering of `TurnError::InvalidRequest` — the
/// `(category, retry, detail)` triple the production adapter maps that failure
/// onto, pinned there by
/// `conversation_turn_submitter_maps_every_turn_error_to_its_class`.
fn permanent_invalid_request_failure() -> TurnSubmissionError {
    TurnSubmissionError::new(
        TurnSubmissionErrorCategory::InvalidRequest,
        TurnSubmissionRetry::Permanent,
        "invalid turn request: permanent invalid request",
    )
}

/// The port's rendering of `TurnError::capacity_exceeded(SubmitTurn, 1)`:
/// retryable, but WITHOUT rotating the submit idempotency key.
fn capacity_exceeded_failure() -> TurnSubmissionError {
    TurnSubmissionError::new(
        TurnSubmissionErrorCategory::CapacityExceeded,
        TurnSubmissionRetry::RetryableWithSameKey,
        "turn capacity exceeded for submit_turn: cap 1",
    )
}

/// The port's rendering of `TurnError::Unavailable` — transient, and the submit
/// idempotency key must rotate before the retry.
fn transient_unavailable_failure() -> TurnSubmissionError {
    TurnSubmissionError::new(
        TurnSubmissionErrorCategory::Unavailable,
        TurnSubmissionRetry::RetryableAfterKeyRotation,
        "turn service unavailable: transient outage",
    )
}

/// The port's rendering of `TurnError::ThreadBusy` — transient, key rotates.
fn thread_busy_failure() -> TurnSubmissionError {
    TurnSubmissionError::new(
        TurnSubmissionErrorCategory::ThreadBusy,
        TurnSubmissionRetry::RetryableAfterKeyRotation,
        "thread already has an active run",
    )
}

#[async_trait]
impl ConversationTurnSubmitter for PermanentFailureTurnCoordinator {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        self.submissions
            .lock()
            .unwrap()
            .push(submit_turn_request(submission));
        Err(permanent_invalid_request_failure())
    }
}

#[async_trait]
impl ConversationTurnSubmitter for CapacityFailureTurnCoordinator {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        self.submissions
            .lock()
            .unwrap()
            .push(submit_turn_request(submission));
        Err(capacity_exceeded_failure())
    }
}

#[async_trait]
impl ConversationTurnSubmitter for BusyFirstUniqueKeyCoordinator {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        let request = submit_turn_request(submission);
        let mut submissions = self.submissions.lock().unwrap();
        submissions.push(request.clone());
        if submissions.len() == 1 {
            return Err(thread_busy_failure());
        }
        Ok(accepted_response(request))
    }
}

#[async_trait]
impl ConversationTurnSubmitter for FailFirstTurnCoordinator {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        let request = submit_turn_request(submission);
        let mut submissions = self.submissions.lock().unwrap();
        submissions.push(request.clone());
        if submissions.len() == 1 {
            return Err(transient_unavailable_failure());
        }
        Ok(accepted_response(request))
    }
}

#[async_trait]
impl ConversationTurnSubmitter for RecordingTurnCoordinator {
    async fn submit_conversation_turn(
        &self,
        submission: ConversationTurnSubmission,
    ) -> Result<SubmitTurnResponse, TurnSubmissionError> {
        let request = submit_turn_request(submission);
        self.submissions.lock().unwrap().push(request.clone());
        Ok(accepted_response(request))
    }
}

fn accepted_response(request: SubmitTurnRequest) -> SubmitTurnResponse {
    SubmitTurnResponse::Accepted {
        turn_id: ironclaw_host_api::turn::TurnId::new(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Queued,
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        event_cursor: ironclaw_host_api::turn::EventCursor(1),
        accepted_message_ref: request.accepted_message_ref,
    }
}
