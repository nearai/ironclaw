use std::sync::Arc;

use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationBindingService, ConversationRouteKind,
    ConversationStateMigrationError, ExternalEventId, RebornFilesystemConversationServices,
    ResolveConversationRequest, migrate_conversation_state_root,
};
use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};
use ironclaw_filesystem::{
    CasExpectation, Entry, InMemoryBackend, RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
};

fn path(value: &str) -> VirtualPath {
    VirtualPath::new(value).expect("valid fixture path")
}

fn scoped_at(
    backend: Arc<InMemoryBackend>,
    root: &VirtualPath,
) -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/conversations").expect("alias"),
        root.clone(),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, view))
}

fn tenant() -> TenantId {
    TenantId::new("tenant-a").expect("tenant")
}

fn actor(id: &str) -> ExternalActorRef {
    ExternalActorRef::new("user", id, None::<String>).expect("actor")
}

fn conversation(id: &str) -> ExternalConversationRef {
    ExternalConversationRef::new(None, id, Some("topic-a"), None).expect("conversation")
}

fn request(actor_id: &str, conversation_id: &str, event_id: &str) -> ResolveConversationRequest {
    ResolveConversationRequest {
        tenant_id: tenant(),
        adapter_kind: AdapterKind::new("telegram").expect("adapter"),
        adapter_installation_id: AdapterInstallationId::new("default-installation")
            .expect("installation"),
        external_actor_ref: actor(actor_id),
        external_conversation_ref: conversation(conversation_id),
        external_event_id: ExternalEventId::new(event_id).expect("event"),
        route_kind: ConversationRouteKind::Direct,
        requested_agent_id: Some(AgentId::new("agent-a").expect("agent")),
        requested_project_id: Some(ProjectId::new("project-a").expect("project")),
    }
}

async fn seed_binding(
    backend: Arc<InMemoryBackend>,
    root: &VirtualPath,
    actor_id: &str,
    conversation_id: &str,
    event_id: &str,
) {
    let services = RebornFilesystemConversationServices::new(scoped_at(backend, root))
        .await
        .expect("services");
    services
        .pair_external_actor(
            tenant(),
            AdapterKind::new("telegram").expect("adapter"),
            AdapterInstallationId::new("default-installation").expect("installation"),
            actor(actor_id),
            UserId::new(actor_id).expect("user"),
        )
        .await
        .expect("pair");
    services
        .resolve_or_create_binding(request(actor_id, conversation_id, event_id))
        .await
        .expect("binding");
}

fn state_path(root: &VirtualPath) -> VirtualPath {
    path(&format!("{}/state.json", root.as_str()))
}

#[tokio::test]
async fn rc1_conversation_state_merges_losslessly_and_second_pass_is_a_noop() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = path("/tenants/tenant-a/shared/telegram-conversations");
    let target = path("/tenants/tenant-a/shared/channel-extensions/telegram/conversations");
    seed_binding(
        Arc::clone(&backend),
        &source,
        "source-user",
        "source-chat",
        "source-event",
    )
    .await;
    seed_binding(
        Arc::clone(&backend),
        &target,
        "target-user",
        "target-chat",
        "target-event",
    )
    .await;

    let source_before = backend
        .get(&state_path(&source))
        .await
        .expect("source read")
        .expect("source state");
    let released_wire: serde_json::Value =
        serde_json::from_slice(&source_before.entry.body).expect("released wire");
    let conversation_ref = &released_wire["bindings"][0][1]["external_conversation_ref"];
    assert_eq!(conversation_ref["thread_id"], "topic-a");
    assert!(conversation_ref.get("topic_id").is_none());
    assert!(conversation_ref.get("reply_target_message_id").is_none());

    let first = migrate_conversation_state_root(backend.as_ref(), &source, &target)
        .await
        .expect("first migration");
    assert!(first.source_present);
    assert!(first.target_present);
    assert!(first.target_written);
    assert!(first.inserted_items > 0);

    let source_after = backend
        .get(&state_path(&source))
        .await
        .expect("source reread")
        .expect("source retained");
    assert_eq!(source_after.entry.body, source_before.entry.body);

    let reopened =
        RebornFilesystemConversationServices::new(scoped_at(Arc::clone(&backend), &target))
            .await
            .expect("reopen migrated target");
    let migrated = reopened
        .lookup_binding(request(
            "source-user",
            "source-chat",
            "source-event-after-migration",
        ))
        .await
        .expect("source binding remains resolvable");
    assert_eq!(migrated.actor.user_id.as_str(), "source-user");

    let target_before_second = backend
        .get(&state_path(&target))
        .await
        .expect("target read")
        .expect("target state");
    let second = migrate_conversation_state_root(backend.as_ref(), &source, &target)
        .await
        .expect("second migration");
    assert_eq!(second.inserted_items, 0);
    assert_eq!(second.unchanged_items, second.source_items);
    assert!(!second.target_written);
    let target_after_second = backend
        .get(&state_path(&target))
        .await
        .expect("target reread")
        .expect("target state");
    assert_eq!(target_after_second.version, target_before_second.version);
    assert_eq!(
        target_after_second.entry.body,
        target_before_second.entry.body
    );
}

#[tokio::test]
async fn divergent_canonical_binding_fails_before_writing_target() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = path("/tenants/tenant-a/shared/slack-conversations");
    let target = path("/tenants/tenant-a/shared/channel-extensions/slack/conversations");
    seed_binding(
        Arc::clone(&backend),
        &source,
        "same-user",
        "same-chat",
        "source-event",
    )
    .await;
    seed_binding(
        Arc::clone(&backend),
        &target,
        "same-user",
        "same-chat",
        "target-event",
    )
    .await;
    let target_before = backend
        .get(&state_path(&target))
        .await
        .expect("target read")
        .expect("target state");

    let error = migrate_conversation_state_root(backend.as_ref(), &source, &target)
        .await
        .expect_err("different canonical thread authorities must conflict");
    assert_eq!(error, ConversationStateMigrationError::Conflict);
    let target_after = backend
        .get(&state_path(&target))
        .await
        .expect("target reread")
        .expect("target state");
    assert_eq!(target_after.version, target_before.version);
    assert_eq!(target_after.entry.body, target_before.entry.body);
}

#[tokio::test]
async fn malformed_or_duplicate_rc1_authority_fails_without_creating_target() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = path("/tenants/tenant-a/shared/telegram-conversations");
    let target = path("/tenants/tenant-a/shared/channel-extensions/telegram/conversations");
    seed_binding(
        Arc::clone(&backend),
        &source,
        "source-user",
        "source-chat",
        "source-event",
    )
    .await;
    let source_path = state_path(&source);
    let original = backend
        .get(&source_path)
        .await
        .expect("source read")
        .expect("source state");
    let mut duplicate: serde_json::Value =
        serde_json::from_slice(&original.entry.body).expect("source json");
    let first_pairing = duplicate["pairings"][0].clone();
    duplicate["pairings"]
        .as_array_mut()
        .expect("pairing array")
        .push(first_pairing);
    backend
        .put(
            &source_path,
            Entry::bytes(serde_json::to_vec(&duplicate).expect("duplicate json")),
            CasExpectation::Version(original.version),
        )
        .await
        .expect("write malformed fixture");

    let error = migrate_conversation_state_root(backend.as_ref(), &source, &target)
        .await
        .expect_err("duplicate authority keys must fail closed");
    assert_eq!(error, ConversationStateMigrationError::MalformedSource);
    assert!(
        backend
            .get(&state_path(&target))
            .await
            .expect("target probe")
            .is_none()
    );
}

#[tokio::test]
async fn duplicate_json_object_key_fails_before_serde_can_collapse_it() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = path("/tenants/tenant-a/shared/telegram-conversations-duplicate-json");
    let target =
        path("/tenants/tenant-a/shared/channel-extensions/telegram/conversations-duplicate-json");
    seed_binding(
        Arc::clone(&backend),
        &source,
        "source-user",
        "source-chat",
        "source-event",
    )
    .await;
    let source_path = state_path(&source);
    let original = backend
        .get(&source_path)
        .await
        .expect("source read")
        .expect("source state");
    let text = String::from_utf8(original.entry.body).expect("source JSON is UTF-8");
    let duplicate = text.replacen('{', "{\"revision\":0,", 1);
    backend
        .put(
            &source_path,
            Entry::bytes(duplicate.into_bytes()),
            CasExpectation::Version(original.version),
        )
        .await
        .expect("write duplicate-key fixture");

    let error = migrate_conversation_state_root(backend.as_ref(), &source, &target)
        .await
        .expect_err("duplicate JSON keys must fail before map materialization");
    assert_eq!(error, ConversationStateMigrationError::MalformedSource);
    assert!(
        backend
            .get(&state_path(&target))
            .await
            .expect("target probe")
            .is_none()
    );
}
