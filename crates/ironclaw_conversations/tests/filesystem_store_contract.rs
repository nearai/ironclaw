//! Contract tests for [`FilesystemConversationStateStore`] and
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

use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationBindingService, ConversationRouteKind,
    ExternalActorRef, ExternalConversationRef, ExternalEventId,
    RebornFilesystemConversationServices, ResolveConversationRequest,
};
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, UserId,
    VirtualPath,
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
    Arc::new(ScopedFilesystem::new(backend, mounts))
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
    ExternalActorRef::new("user", id).unwrap()
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
