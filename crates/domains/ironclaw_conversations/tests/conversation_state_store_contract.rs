//! Contract tests for [`ConversationStateStore`] and
//! [`RebornFilesystemConversationServices`].
//!
//! Most surface coverage already lives in `inbound_contract.rs`, which
//! drives the in-memory services + the legacy libsql/postgres adapters.
//! This file targets the [`ScopedFilesystem`] migration specifically —
//! durability across reopen on an in-memory backend, and the
//! cross-tenant isolation regression that mirrors
//! `filesystem_run_state_store_isolates_two_tenants_with_same_user_project_ids`
//! and the other migrated consumer crates' isolation tests.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use ironclaw_conversations::{
    AcceptConversationMessageRequest, AcceptedConversationMessageLookup,
    AcceptedConversationMessageReplay, AdapterInstallationId, AdapterKind,
    ConditionalUnpairOutcome, ConversationBindingService, ConversationMessageRecord,
    ConversationRouteKind, ExpectedExternalActorOwner, ExternalEventId, InboundConversationService,
    InboundMessageContentRef, InboundTurnError, MessageIdempotencyStatus,
    RebornFilesystemConversationServices, ResetConversationRequest, ResolveConversationRequest,
};
use ironclaw_extension_contracts::external::{
    ExternalActorBindingEpoch, ExternalActorRef, ExternalConversationRef,
};
use ironclaw_filesystem::{CasExpectation, InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};

/// Wrap a `RootFilesystem` backend in a `ScopedFilesystem` exposing the
/// `/conversations` alias at the given tenant/user-scoped target. Tests
/// share one backend across multiple wrappers to drive the cross-tenant
/// isolation invariant.
fn scoped_conversations_fs<F>(backend: Arc<F>, tenant: &str, user: &str) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let target = format!("/tenants/{tenant}/users/{user}/conversations");
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/conversations").expect("alias"),
        VirtualPath::new(target).expect("target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn tenant_id(id: &str) -> TenantId {
    TenantId::new(id).unwrap()
}

fn user_id(id: &str) -> UserId {
    UserId::new(id).unwrap()
}

fn telegram() -> AdapterKind {
    AdapterKind::new("telegram").unwrap()
}

fn default_installation() -> AdapterInstallationId {
    AdapterInstallationId::new("default-installation").unwrap()
}

fn external_actor(id: &str) -> ExternalActorRef {
    ExternalActorRef::new("user", id, None::<String>).unwrap()
}

fn external_conversation(id: &str) -> ExternalConversationRef {
    ExternalConversationRef::new(None, id, None, None).unwrap()
}

fn resolve_request(
    tenant: TenantId,
    actor: ExternalActorRef,
    conversation: ExternalConversationRef,
    event_id: &str,
) -> ResolveConversationRequest {
    ResolveConversationRequest {
        tenant_id: tenant,
        adapter_kind: telegram(),
        adapter_installation_id: default_installation(),
        external_actor_ref: actor,
        external_conversation_ref: conversation,
        external_event_id: ExternalEventId::new(event_id).unwrap(),
        route_kind: ConversationRouteKind::Direct,
        requested_agent_id: Some(AgentId::new("agent-a").unwrap()),
        requested_project_id: Some(ProjectId::new("project-a").unwrap()),
    }
}

/// Round-trip durability: a write on services A1 must be visible to a
/// fresh services A2 wrapping the same backend + mount view. This is the
/// filesystem equivalent of the libSQL/Postgres restart-replay tests
/// that the legacy stores carried.
#[tokio::test]
async fn filesystem_conversation_services_round_trip_persisted_state_on_reopen() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");

    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .unwrap();
    services
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user_id("alice"),
        )
        .await
        .unwrap();
    let _ = services
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-1"),
            "event-1",
        ))
        .await
        .unwrap();
    drop(services);

    // Fresh service wrapping the same backend rehydrates the pairing
    // and binding from durable storage. `_ = ...` because the duplicate
    // `external_event_id` is what we'd expect from a retry — the test
    // only cares that the second resolve succeeds (same thread reused),
    // not the precise idempotency status here.
    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .unwrap();
    let resolution = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-1"),
            "event-1",
        ))
        .await
        .unwrap();
    assert_eq!(resolution.tenant_id, tenant_id("tenant-a"));
    assert_eq!(resolution.actor.user_id, user_id("alice"));

    // The durable wrapper forwards the inbound-message half of the contract
    // too, not just binding resolution. `inbound_contract.rs` only ever drives
    // these two methods through `InMemoryConversationServices`, so without
    // this the filesystem-backed `accept_inbound_message` /
    // `replay_accepted_inbound_message` forwarders are never executed —
    // and a broken delegation here would look exactly like a passing suite.
    let accepted = reopened
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant_id("tenant-a"),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("telegram-user-1"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: external_conversation("chat-1"),
            external_event_id: ExternalEventId::new("event-2").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:event-2").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .expect("the durable services accept an inbound message");
    assert_eq!(accepted.tenant_id, tenant_id("tenant-a"));

    let replay = reopened
        .replay_accepted_inbound_message(AcceptedConversationMessageLookup {
            tenant_id: tenant_id("tenant-a"),
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("telegram-user-1"),
            external_conversation_ref: external_conversation("chat-1"),
            external_event_id: ExternalEventId::new("event-2").unwrap(),
        })
        .await
        .expect("the replay lookup succeeds")
        .expect("the accepted message replays by its external event id");
    assert_eq!(
        replay.accepted_message.message_ref, accepted.message_ref,
        "the replay must return the message the accept produced"
    );
    assert_eq!(replay.accepted_message.thread_id, accepted.thread_id);
    assert_eq!(
        accepted.idempotency,
        MessageIdempotencyStatus::Inserted,
        "the first accept inserts"
    );
    assert_eq!(
        replay.accepted_message.idempotency,
        MessageIdempotencyStatus::Duplicate,
        "replaying an already-accepted event reports it as a duplicate, not a fresh insert"
    );

    // WS5 renamed this DTO family (`AcceptedInboundMessage*` ->
    // `AcceptedConversationMessage*`, `ThreadMessageRecord` ->
    // `ConversationMessageRecord`). Serde keys on FIELD names, not type names,
    // so the rename must be invisible on the wire — these types are persisted
    // through the conversation state store, and a rename that silently changed
    // the encoding would strand every durable record written before it. Nothing
    // else in the suite serializes this family, so without this the derives are
    // never even instantiated.
    let record = reopened
        .inner()
        .accepted_messages()
        .await
        .into_iter()
        .find(|record| record.accepted.message_ref == accepted.message_ref)
        .expect("the accepted message is recorded in conversation state");

    let encoded_record = serde_json::to_value(&record).expect("record serializes");
    for field in [
        "accepted",
        "actor",
        "external_event_id",
        "content_ref",
        "received_at",
    ] {
        assert!(
            encoded_record.get(field).is_some(),
            "ConversationMessageRecord must keep its `{field}` wire field after the type rename"
        );
    }
    for field in [
        "tenant_id",
        "thread_id",
        "actor",
        "message_ref",
        "source_binding_ref",
        "reply_target_binding_ref",
        "received_at",
        "idempotency",
    ] {
        assert!(
            encoded_record["accepted"].get(field).is_some(),
            "AcceptedConversationMessage must keep its `{field}` wire field after the type rename"
        );
    }

    assert_eq!(
        serde_json::from_value::<ConversationMessageRecord>(encoded_record)
            .expect("record round-trips"),
        record,
        "ConversationMessageRecord must survive a durable round trip unchanged"
    );

    let encoded_replay = serde_json::to_value(&replay).expect("replay serializes");
    assert_eq!(
        serde_json::from_value::<AcceptedConversationMessageReplay>(encoded_replay)
            .expect("replay round-trips"),
        replay,
        "AcceptedConversationMessageReplay must survive a durable round trip unchanged"
    );

    let lookup = AcceptedConversationMessageLookup {
        tenant_id: tenant_id("tenant-a"),
        adapter_kind: telegram(),
        adapter_installation_id: default_installation(),
        external_actor_ref: external_actor("telegram-user-1"),
        external_conversation_ref: external_conversation("chat-1"),
        external_event_id: ExternalEventId::new("event-2").unwrap(),
    };
    assert_eq!(
        serde_json::from_value::<AcceptedConversationMessageLookup>(
            serde_json::to_value(&lookup).expect("lookup serializes")
        )
        .expect("lookup round-trips"),
        lookup,
        "AcceptedConversationMessageLookup must survive a durable round trip unchanged"
    );
}

#[tokio::test]
async fn filesystem_conversation_services_replay_reset_after_reopen_without_rotating_twice() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let actor = external_actor("telegram-user-reset");
    let conversation = external_conversation("chat-reset");

    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .expect("services");
    services
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            actor.clone(),
            user_id("alice"),
        )
        .await
        .expect("pair actor");
    let first = services
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            actor.clone(),
            conversation.clone(),
            "event-before-reset",
        ))
        .await
        .expect("initial binding");
    let reset_request = ResetConversationRequest {
        resolve_request: resolve_request(tenant_id("tenant-a"), actor, conversation, "event-reset"),
        expected_thread_id: first.turn_scope.thread_id,
    };
    let reset = services
        .reset_conversation_binding(reset_request.clone())
        .await
        .expect("reset binding");
    drop(services);

    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .expect("reopen");
    let replay = reopened
        .reset_conversation_binding(reset_request)
        .await
        .expect("replay reset");
    assert_eq!(replay, reset);
}

#[tokio::test]
async fn filesystem_conversation_services_persist_unpair_revocation_on_reopen() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");

    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .unwrap();
    services
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user_id("alice"),
        )
        .await
        .unwrap();
    let first = services
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-unpair-persisted"),
            "event-before-unpair",
        ))
        .await
        .unwrap();
    services
        .unpair_external_actor(
            &tenant_id("tenant-a"),
            &telegram(),
            &default_installation(),
            &external_actor("telegram-user-1"),
        )
        .await
        .unwrap();
    drop(services);

    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .unwrap();
    reopened
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user_id("alice"),
        )
        .await
        .unwrap();
    let stale = reopened
        .lookup_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-unpair-persisted"),
            "event-after-reopen-lookup",
        ))
        .await
        .expect_err("old direct binding should remain revoked after reopen");
    assert!(matches!(stale, InboundTurnError::BindingRequired { .. }));

    let rebound = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-unpair-persisted"),
            "event-after-reopen-repair",
        ))
        .await
        .unwrap();
    assert_ne!(
        rebound.turn_scope.thread_id, first.turn_scope.thread_id,
        "re-pair after persisted unpair should create a fresh direct route"
    );
}

#[tokio::test]
async fn filesystem_conversation_services_persist_conditional_unpair_epochs_on_reopen() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let actor = external_actor("telegram-user-epoch");
    let first_epoch = ExternalActorBindingEpoch::new("generation-1").expect("epoch");
    let second_epoch = ExternalActorBindingEpoch::new("generation-2").expect("epoch");

    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .expect("services");
    services
        .pair_external_actor_with_epoch(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            actor.clone(),
            user_id("alice"),
            first_epoch.clone(),
        )
        .await
        .expect("first pairing");
    let first = services
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            actor.clone(),
            external_conversation("chat-epoch"),
            "event-epoch-first",
        ))
        .await
        .expect("first binding");
    assert_eq!(first.binding_epoch, Some(first_epoch.clone()));
    services
        .pair_external_actor_with_epoch(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            actor.clone(),
            user_id("alice"),
            second_epoch.clone(),
        )
        .await
        .expect("new generation pairing");
    drop(services);

    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .expect("reopen");
    let stale = reopened
        .unpair_external_actor_if_owned_by(
            &tenant_id("tenant-a"),
            &telegram(),
            &default_installation(),
            &actor,
            &ExpectedExternalActorOwner {
                user_id: user_id("alice"),
                binding_epoch: Some(first_epoch),
            },
        )
        .await
        .expect("stale unpair");
    assert_eq!(stale, ConditionalUnpairOutcome::OwnerChanged);

    let current = reopened
        .lookup_binding(resolve_request(
            tenant_id("tenant-a"),
            actor,
            external_conversation("chat-epoch"),
            "event-epoch-current",
        ))
        .await
        .expect("new generation and route remain");
    assert_eq!(current.turn_scope.thread_id, first.turn_scope.thread_id);
    assert_eq!(current.binding_epoch, Some(second_epoch));
}

#[tokio::test]
async fn filesystem_conversation_services_reopen_snapshot_without_pairing_epochs() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .expect("services");
    services
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-legacy-snapshot"),
            user_id("alice"),
        )
        .await
        .expect("pair actor");
    drop(services);

    let state_path = VirtualPath::new("/tenants/tenant-a/users/alice/conversations/state.json")
        .expect("state path");
    let mut versioned = backend
        .get(&state_path)
        .await
        .expect("read state")
        .expect("stored state");
    let mut state: serde_json::Value =
        serde_json::from_slice(&versioned.entry.body).expect("state json");
    state
        .as_object_mut()
        .expect("state object")
        .remove("pairing_epochs");
    versioned.entry.body = serde_json::to_vec(&state).expect("legacy state json");
    backend
        .put(
            &state_path,
            versioned.entry,
            CasExpectation::Version(versioned.version),
        )
        .await
        .expect("write legacy snapshot");

    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .expect("old snapshots remain readable");
    let resolution = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-legacy-snapshot"),
            external_conversation("chat-legacy-snapshot"),
            "event-legacy-snapshot",
        ))
        .await
        .expect("legacy epoch-less pairing remains usable");
    assert_eq!(resolution.actor.user_id, user_id("alice"));
    assert_eq!(resolution.binding_epoch, None);
}

/// Regression for the `ScopedFilesystem` migration: two
/// [`RebornFilesystemConversationServices`] instances share one
/// underlying [`RootFilesystem`] but each is constructed with a
/// [`MountView`] whose `/conversations` alias resolves to a different
/// tenant-scoped target. The pairing and binding produced under tenant
/// A's services must not be visible from tenant B's services, even
/// though the in-store path is the same (`/conversations/state.json`)
/// and the `(user_id, project_id)` tuple is identical.
///
/// Before this migration, the conversation state stores held the
/// substrate handle directly (an `Arc<libsql::Database>` /
/// `deadpool_postgres::Pool`) and tenant scoping was a property of the
/// caller — any composition layer that forgot to construct per-tenant
/// substrates would silently share storage. With the structural
/// `ScopedFilesystem` wrapping, two services over the same backend
/// cannot see each other's state.
#[tokio::test]
async fn filesystem_conversation_state_store_isolates_two_tenants_with_same_user_project_ids() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped_a = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let scoped_b = scoped_conversations_fs(Arc::clone(&backend), "tenant-b", "alice");

    let services_a = RebornFilesystemConversationServices::new(scoped_a)
        .await
        .unwrap();
    let services_b = RebornFilesystemConversationServices::new(scoped_b)
        .await
        .unwrap();

    // Pair the same `(adapter, external_actor, user_id)` tuple on both
    // services — but each service uses its own `tenant_id` for the
    // pairing key. The only thing keeping the two states apart is the
    // mount-time tenant prefix on each service's MountView.
    services_a
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user_id("alice"),
        )
        .await
        .unwrap();

    // Tenant A can resolve a binding for its paired actor.
    let resolution_a = services_a
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-1"),
            external_conversation("chat-1"),
            "event-a",
        ))
        .await
        .unwrap();
    assert_eq!(resolution_a.actor.user_id, user_id("alice"));

    // Tenant B's services do NOT see tenant A's pairing — resolving the
    // identical external actor on tenant B must fail with
    // `BindingRequired`, fail-closed semantics tested by the unpaired
    // case in `inbound_contract.rs`.
    let err = services_b
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-b"),
            external_actor("telegram-user-1"),
            external_conversation("chat-1"),
            "event-b",
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            ironclaw_conversations::InboundTurnError::BindingRequired { .. }
        ),
        "tenant B must NOT see tenant A's pairing (cross-tenant leak); got {err:?}",
    );

    // Pair tenant B's external actor (same key value, different
    // tenant), verify resolution succeeds on B without re-exposing A's
    // state. We also pair under tenant_id("tenant-b") so the binding
    // key matches B's scope.
    services_b
        .pair_external_actor(
            tenant_id("tenant-b"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-1"),
            user_id("alice"),
        )
        .await
        .unwrap();
    let resolution_b = services_b
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-b"),
            external_actor("telegram-user-1"),
            external_conversation("chat-1"),
            "event-b",
        ))
        .await
        .unwrap();
    assert_eq!(resolution_b.tenant_id, tenant_id("tenant-b"));
    assert_eq!(resolution_b.actor.user_id, user_id("alice"));
    // Tenants must hold distinct thread ids even though the external
    // conversation id matches — first-contact binding always materializes
    // a fresh thread per (tenant, mount target) and the two services
    // cannot see each other's bindings.
    assert_ne!(
        resolution_a.turn_scope.thread_id, resolution_b.turn_scope.thread_id,
        "cross-tenant first-contact bindings must produce distinct thread ids"
    );
}

/// A threaded route: a topic inside the conversation plus the per-event reply
/// target. This is the shape the `thread_id` -> `topic_id` rename touched, so
/// it is the shape the durable-grammar tests drive.
fn threaded_conversation(id: &str, topic: &str, reply_target: &str) -> ExternalConversationRef {
    ExternalConversationRef::new(Some("space-1"), id, Some(topic), Some(reply_target))
        .expect("threaded conversation")
}

/// Collect every `(key, value)` pair in a JSON document, at any depth.
fn json_fields(value: &serde_json::Value, out: &mut Vec<(String, serde_json::Value)>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                out.push((key.clone(), child.clone()));
                json_fields(child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                json_fields(item, out);
            }
        }
        _ => {}
    }
}

async fn read_state_document(
    backend: &InMemoryBackend,
    tenant: &str,
    user: &str,
) -> serde_json::Value {
    let state_path = VirtualPath::new(format!(
        "/tenants/{tenant}/users/{user}/conversations/state.json"
    ))
    .expect("state path");
    let versioned = backend
        .get(&state_path)
        .await
        .expect("read state")
        .expect("stored state");
    serde_json::from_slice(&versioned.entry.body).expect("state json")
}

/// Drive a paired actor, a threaded binding and an accepted message through the
/// durable services, and return the persisted document.
async fn seed_threaded_state(
    backend: Arc<InMemoryBackend>,
    scoped: Arc<ScopedFilesystem<InMemoryBackend>>,
    conversation: &ExternalConversationRef,
) -> serde_json::Value {
    let services = RebornFilesystemConversationServices::new(Arc::clone(&scoped))
        .await
        .expect("services");
    services
        .pair_external_actor(
            tenant_id("tenant-a"),
            telegram(),
            default_installation(),
            external_actor("telegram-user-threaded"),
            user_id("alice"),
        )
        .await
        .expect("pair actor");
    let resolution = services
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-threaded"),
            conversation.clone(),
            "event-threaded-1",
        ))
        .await
        .expect("resolve threaded binding");
    services
        .accept_inbound_message(AcceptConversationMessageRequest {
            tenant_id: tenant_id("tenant-a"),
            thread_id: resolution.turn_scope.thread_id,
            actor: resolution.actor,
            adapter_kind: telegram(),
            adapter_installation_id: default_installation(),
            external_actor_ref: external_actor("telegram-user-threaded"),
            source_binding_ref: resolution.source_binding_ref,
            reply_target_binding_ref: resolution.reply_target_binding_ref,
            external_conversation_ref: conversation.clone(),
            external_event_id: ExternalEventId::new("event-threaded-2").unwrap(),
            route_kind: ConversationRouteKind::Direct,
            content_ref: InboundMessageContentRef::new("content:event-threaded-2").unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
            requested_run_profile: None,
        })
        .await
        .expect("accept a threaded message");
    drop(services);
    read_state_document(&backend, "tenant-a", "alice").await
}

/// The rollback half of the `thread_id` -> `topic_id` rename, asserted on the
/// *document* rather than on a surrogate struct.
///
/// `stored_refs.rs`'s own tests prove the adapter maps the grammar correctly,
/// but they declare their own record types — so an adapter that was never
/// attached, or attached to only some of the fields that carry a ref, leaves
/// them green. This drives the real `StoredConversationState` through the real
/// services and then reads every key in the persisted document, so a missing or
/// misplaced annotation at *any* site fails here.
///
/// The assertion is deliberately the absence of the canonical spelling: a
/// released binary's readers name `thread_id`/`message_id` and carry no
/// aliases, so a document written in the canonical spelling reads back with
/// `None` for both, with no error — and because the topic keys `BindingKey`,
/// two threaded bindings in one conversation would collapse onto one key and
/// the earlier one would be dropped.
#[tokio::test]
async fn filesystem_conversation_services_persist_external_refs_in_the_durable_grammar() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let conversation = threaded_conversation("chat-threaded", "topic-1", "msg-1700.1");
    let state = seed_threaded_state(Arc::clone(&backend), Arc::clone(&scoped), &conversation).await;

    let mut fields = Vec::new();
    json_fields(&state, &mut fields);
    assert!(
        !fields.is_empty(),
        "the persisted document scanned empty — the walk is broken"
    );

    let canonical: Vec<&String> = fields
        .iter()
        .map(|(key, _)| key)
        .filter(|key| key.as_str() == "topic_id" || key.as_str() == "reply_target_message_id")
        .collect();
    assert!(
        canonical.is_empty(),
        "durable records must keep the released grammar so a rollback can still read them; \
         found the canonical spelling at {canonical:?} in {state}"
    );

    // Non-vacuity: the topic and the reply target must actually be in there
    // under the durable names, or "no canonical spelling" would pass on an
    // empty document.
    assert!(
        fields
            .iter()
            .any(|(key, value)| key == "thread_id" && value == "topic-1"),
        "the external topic must persist as `thread_id`: {state}"
    );
    assert!(
        fields
            .iter()
            .any(|(key, value)| key == "message_id" && value == "msg-1700.1"),
        "the reply target must persist as `message_id`: {state}"
    );

    // And the route still resolves after a reopen, which is what the grammar
    // is protecting.
    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .expect("reopen");
    let resolution = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-threaded"),
            conversation,
            "event-threaded-3",
        ))
        .await
        .expect("the threaded binding survives a reopen");
    assert_eq!(resolution.actor.user_id, user_id("alice"));
}

/// The upgrade half, and the reason the aliases are not dead code: a document
/// written in the canonical spelling — by an intermediate build of this rename
/// — must still load, with its topic and reply target intact.
///
/// Rewriting is scoped to objects that carry a `conversation_id`, so the
/// canonical `ThreadId` fields elsewhere in the document (`BindingRecord`,
/// `ReplyTargetRecord`, `ThreadKey` all have a real `thread_id`) are left
/// alone. If the rewrite ever stopped finding anything, the assertion below it
/// would still have to hold, so the test cannot pass vacuously.
#[tokio::test]
async fn filesystem_conversation_services_reopen_pre_unification_external_refs() {
    let backend = Arc::new(InMemoryBackend::new());
    let scoped = scoped_conversations_fs(Arc::clone(&backend), "tenant-a", "alice");
    let conversation = threaded_conversation("chat-canonical", "topic-9", "msg-1900.9");
    let mut state =
        seed_threaded_state(Arc::clone(&backend), Arc::clone(&scoped), &conversation).await;

    fn rewrite_refs_to_canonical(value: &mut serde_json::Value, rewritten: &mut usize) {
        match value {
            serde_json::Value::Object(map) => {
                if map.contains_key("conversation_id") {
                    if let Some(topic) = map.remove("thread_id") {
                        map.insert("topic_id".to_string(), topic);
                        *rewritten += 1;
                    }
                    if let Some(reply_target) = map.remove("message_id") {
                        map.insert("reply_target_message_id".to_string(), reply_target);
                        *rewritten += 1;
                    }
                }
                for child in map.values_mut() {
                    rewrite_refs_to_canonical(child, rewritten);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    rewrite_refs_to_canonical(item, rewritten);
                }
            }
            _ => {}
        }
    }

    let mut rewritten = 0usize;
    rewrite_refs_to_canonical(&mut state, &mut rewritten);
    assert!(
        rewritten > 0,
        "the rewrite found no external ref to re-spell, so the reopen below would \
         prove nothing: {state}"
    );

    let state_path = VirtualPath::new("/tenants/tenant-a/users/alice/conversations/state.json")
        .expect("state path");
    let mut versioned = backend
        .get(&state_path)
        .await
        .expect("read state")
        .expect("stored state");
    versioned.entry.body = serde_json::to_vec(&state).expect("canonical state json");
    backend
        .put(
            &state_path,
            versioned.entry,
            CasExpectation::Version(versioned.version),
        )
        .await
        .expect("write canonically spelled snapshot");

    let reopened = RebornFilesystemConversationServices::new(scoped)
        .await
        .expect("a canonically spelled snapshot remains readable");
    let resolution = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-threaded"),
            conversation,
            "event-canonical-1",
        ))
        .await
        .expect("the threaded route survives the alias path");
    assert_eq!(resolution.actor.user_id, user_id("alice"));

    // The topic must have survived as a topic, not have been dropped into the
    // conversation root: a route with a *different* topic in the same
    // conversation must not resolve to the same binding.
    let other_topic = reopened
        .resolve_or_create_binding(resolve_request(
            tenant_id("tenant-a"),
            external_actor("telegram-user-threaded"),
            threaded_conversation("chat-canonical", "topic-other", "msg-1900.9"),
            "event-canonical-2",
        ))
        .await
        .expect("a second topic resolves");
    assert_ne!(
        resolution.turn_scope.thread_id, other_topic.turn_scope.thread_id,
        "two topics in one conversation must key different bindings — if the topic had \
         been read as None, both would collapse onto the conversation root"
    );
}
