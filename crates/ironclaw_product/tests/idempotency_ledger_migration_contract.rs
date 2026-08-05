use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use ironclaw_filesystem::{
    CasExpectation, Entry, Filter, InMemoryBackend, Page, RecordKind, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{
    ids::{AgentId, InvocationId, ProjectId, TenantId, UserId},
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_product::{
    AdapterInstallationId, AuthRequirement, DefaultProductSurface, ExternalActorRef,
    ExternalConversationRef, ExternalEventId, FakeConversationBindingService,
    FakeInboundTurnService, IdempotencyDecision, IdempotencyLedger,
    IdempotencyLedgerMigrationError, ParsedProductInbound, ProductAdapterId, ProductInboundAck,
    ProductInboundAction, ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason,
    ProtocolAuthEvidence, RebornFilesystemIdempotencyLedger, TrustedInboundContext,
    UserMessagePayload, migrate_idempotency_ledger_root,
};
use ironclaw_product_contracts::action::{ActionFingerprintKey, SourceBindingKey};

fn path(value: impl Into<String>) -> VirtualPath {
    VirtualPath::new(value).expect("valid fixture path")
}

fn source_root() -> VirtualPath {
    path("/tenants/tenant-a/shared/telegram-product-workflow/idempotency")
}

fn target_root() -> VirtualPath {
    path("/tenants/tenant-a/shared/channel-extensions/telegram/product-workflow/idempotency")
}

// Captured from the 1.0.0-rc.1 `ProductInboundAction` serde contract. Keep this
// literal independent of the current serializer so schema drift cannot make the
// release-pair compatibility test pass by construction.
const RC1_SETTLED_ACTION_WIRE: &str = r#"{
  "action_id": "8e933c3e-87fd-43c3-ae36-98477ecafbc2",
  "fingerprint": {
    "adapter_id": "telegram",
    "installation_id": "default-installation",
    "external_actor_ref": {
      "kind": "user",
      "id": "telegram-user",
      "display_name": null
    },
    "source_binding_key": "space:0:;conversation:6:chat-a;topic:0:;",
    "external_event_id": "event-0001"
  },
  "phase": "settled",
  "dispatch_kind": null,
  "outcome": "no_op",
  "received_at": "2026-01-02T03:04:05Z",
  "settled_at": "2026-01-02T03:04:06Z"
}"#;

fn scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        user_id: UserId::new("operator").expect("user"),
        agent_id: Some(AgentId::new("agent-a").expect("agent")),
        project_id: Some(ProjectId::new("project-a").expect("project")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn fingerprint(suffix: usize) -> ActionFingerprintKey {
    ActionFingerprintKey::new(
        ironclaw_product::ProductAdapterId::new("telegram").expect("adapter"),
        ironclaw_product::AdapterInstallationId::new("default-installation").expect("installation"),
        ironclaw_product::ExternalActorRef::new("user", "telegram-user", None::<String>)
            .expect("actor"),
        SourceBindingKey::new("space:0:;conversation:6:chat-a;topic:0:;").expect("binding"),
        ironclaw_product::ExternalEventId::new(format!("event-{suffix:04}")).expect("event"),
    )
}

fn provider_envelope(suffix: usize) -> ProductInboundEnvelope {
    let installation = AdapterInstallationId::new("default-installation").expect("installation");
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Test".to_string(),
        },
        installation.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("telegram").expect("adapter"),
        installation,
        Utc::now(),
        &evidence,
    )
    .expect("trusted fixture context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("event-{suffix:04}")).expect("event"),
        ExternalActorRef::new("user", "telegram-user", None::<String>).expect("actor"),
        ExternalConversationRef::new(None, "chat-a", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "provider retry must not duplicate this turn",
                Vec::new(),
                ProductTriggerReason::DirectChat,
            )
            .expect("message"),
        ),
    )
    .expect("parsed fixture");
    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

fn scope_suffix(scope: &ResourceScope) -> String {
    let agent = scope.agent_id.as_ref().map_or("_", |id| id.as_str());
    let project = scope.project_id.as_ref().map_or("_", |id| id.as_str());
    let mission = scope.mission_id.as_ref().map_or("_", |id| id.as_str());
    let thread = scope.thread_id.as_ref().map_or("_", |id| id.as_str());
    format!(
        "actions/_scope/{}/{}/{}/{}/{}/{}",
        hex_component(scope.tenant_id.as_str()),
        hex_component(scope.user_id.as_str()),
        hex_component(agent),
        hex_component(project),
        hex_component(mission),
        hex_component(thread)
    )
}

fn action_path(
    root: &VirtualPath,
    scope: &ResourceScope,
    fingerprint: &ActionFingerprintKey,
) -> VirtualPath {
    path(format!(
        "{}/{}/{}/{}/{}/{}/{}/{}.json",
        root.as_str(),
        scope_suffix(scope),
        hex_component(fingerprint.adapter_id.as_str()),
        hex_component(fingerprint.installation_id.as_str()),
        hex_component(fingerprint.external_actor_ref.kind()),
        hex_component(fingerprint.external_actor_ref.id()),
        hex_component(fingerprint.source_binding_key.as_str()),
        hex_component(fingerprint.external_event_id.as_str())
    ))
}

fn old_action_entry(action: &ProductInboundAction) -> Entry {
    Entry::record(
        RecordKind::new("product_workflow_action").expect("old action kind"),
        &serde_json::to_value(action).expect("old action wire"),
    )
    .expect("old action entry")
}

async fn put_old_action(
    backend: &InMemoryBackend,
    root: &VirtualPath,
    scope: &ResourceScope,
    action: &ProductInboundAction,
) {
    backend
        .put(
            &action_path(root, scope, &action.fingerprint),
            old_action_entry(action),
            CasExpectation::Absent,
        )
        .await
        .expect("seed old action");
}

async fn put_frozen_rc1_action(
    backend: &InMemoryBackend,
    root: &VirtualPath,
    scope: &ResourceScope,
    action: &ProductInboundAction,
) {
    let payload = serde_json::from_str(RC1_SETTLED_ACTION_WIRE).expect("frozen rc1 action wire");
    backend
        .put(
            &action_path(root, scope, &action.fingerprint),
            Entry::record(
                RecordKind::new("product_workflow_action").expect("rc1 action kind"),
                &payload,
            )
            .expect("rc1 action entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed frozen rc1 action");
}

fn target_ledger(
    backend: Arc<InMemoryBackend>,
    root: &VirtualPath,
    scope: ResourceScope,
) -> RebornFilesystemIdempotencyLedger<InMemoryBackend> {
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/engine/product_surface/idempotency").expect("alias"),
        root.clone(),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::new(ScopedFilesystem::with_fixed_view(backend, view)),
        scope,
        Duration::seconds(60),
    )
}

fn hex_component(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[tokio::test]
async fn rc1_action_wire_migrates_replays_and_retains_source_on_repeat() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = source_root();
    let target = target_root();
    let scope = scope();
    let received_at = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).unwrap();
    let action: ProductInboundAction =
        serde_json::from_str(RC1_SETTLED_ACTION_WIRE).expect("deserialize frozen rc1 action");
    assert_eq!(action.received_at, received_at);
    put_frozen_rc1_action(backend.as_ref(), &source, &scope, &action).await;
    let source_path = action_path(&source, &scope, &action.fingerprint);
    let source_before = backend
        .get(&source_path)
        .await
        .expect("source read")
        .expect("source action");
    assert_eq!(
        source_before.entry.kind.as_ref().map(|kind| kind.as_str()),
        Some("product_workflow_action")
    );
    let old_wire: serde_json::Value =
        serde_json::from_slice(&source_before.entry.body).expect("old action json");
    assert_eq!(old_wire["fingerprint"]["adapter_id"], "telegram");
    assert_eq!(old_wire["phase"], "settled");

    let first = migrate_idempotency_ledger_root(backend.as_ref(), &source, &target)
        .await
        .expect("migrate old ledger");
    assert_eq!(first.scanned_actions, 1);
    assert_eq!(first.migrated_actions, 1);
    assert_eq!(first.unchanged_actions, 0);

    let replay = target_ledger(Arc::clone(&backend), &target, scope.clone())
        .begin_or_replay(
            action.fingerprint.clone(),
            received_at + Duration::seconds(1),
        )
        .await
        .expect("migrated outcome replays");
    let IdempotencyDecision::Replay(replayed) = replay else {
        panic!("migrated terminal action must replay");
    };
    assert_eq!(replayed.action_id, action.action_id);
    assert_eq!(replayed.outcome, Some(ProductInboundAck::NoOp));

    let target_path = action_path(&target, &scope, &action.fingerprint);
    let target_before_second = backend
        .get(&target_path)
        .await
        .expect("target read")
        .expect("target action");
    assert_eq!(
        target_before_second
            .entry
            .kind
            .as_ref()
            .map(|kind| kind.as_str()),
        Some("product_surface_action")
    );
    let second = migrate_idempotency_ledger_root(backend.as_ref(), &source, &target)
        .await
        .expect("repeat migration");
    assert_eq!(second.migrated_actions, 0);
    assert_eq!(second.unchanged_actions, 1);
    let target_after_second = backend
        .get(&target_path)
        .await
        .expect("target reread")
        .expect("target action");
    assert_eq!(target_after_second.version, target_before_second.version);
    assert_eq!(
        backend
            .get(&source_path)
            .await
            .expect("source reread")
            .expect("source retained")
            .entry,
        source_before.entry
    );
}

#[tokio::test]
async fn provider_retry_after_migration_replays_without_a_second_turn_submission() {
    let backend = Arc::new(InMemoryBackend::new());
    let migration_scope = scope();
    let source = source_root();
    let target = target_root();
    let inbound = Arc::new(FakeInboundTurnService::new());
    let envelope = provider_envelope(77);

    let rc1_surface = DefaultProductSurface::new(
        inbound.clone(),
        Arc::new(target_ledger(
            Arc::clone(&backend),
            &source,
            migration_scope.clone(),
        )),
        Arc::new(FakeConversationBindingService::new()),
    );
    let first = rc1_surface
        .submit_inbound(envelope.clone())
        .await
        .expect("rc1 provider delivery accepted");
    assert!(matches!(first, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);

    migrate_idempotency_ledger_root(backend.as_ref(), &source, &target)
        .await
        .expect("migrate settled rc1 provider outcome");
    let upgraded_surface = DefaultProductSurface::new(
        inbound.clone(),
        Arc::new(target_ledger(backend, &target, migration_scope)),
        Arc::new(FakeConversationBindingService::new()),
    );
    let retry = upgraded_surface
        .submit_inbound(envelope)
        .await
        .expect("provider retry replays migrated outcome");
    assert!(matches!(retry, ProductInboundAck::Duplicate { .. }));
    assert_eq!(
        inbound.accepted_count(),
        1,
        "the migrated idempotency decision must stop a second message/turn submission"
    );
}

#[tokio::test]
async fn divergent_or_malformed_action_fails_before_any_new_target_write() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = source_root();
    let target = target_root();
    let scope = scope();
    let source_action = ProductInboundAction::begin(fingerprint(1), Utc::now());
    put_old_action(backend.as_ref(), &source, &scope, &source_action).await;

    let divergent = ProductInboundAction::begin(source_action.fingerprint.clone(), Utc::now());
    backend
        .put(
            &action_path(&target, &scope, &source_action.fingerprint),
            Entry::record(
                RecordKind::new("product_surface_action").expect("new kind"),
                &serde_json::to_value(divergent).expect("divergent wire"),
            )
            .expect("divergent entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed divergent target");
    let target_before = backend
        .get(&action_path(&target, &scope, &source_action.fingerprint))
        .await
        .expect("target read")
        .expect("target action");

    let error = migrate_idempotency_ledger_root(backend.as_ref(), &source, &target)
        .await
        .expect_err("divergent action must fail closed");
    assert_eq!(error, IdempotencyLedgerMigrationError::Conflict);
    let target_after = backend
        .get(&action_path(&target, &scope, &source_action.fingerprint))
        .await
        .expect("target reread")
        .expect("target action");
    assert_eq!(target_after, target_before);

    let malformed_source = path(format!(
        "{}/actions/_scope/a/b/c/d/e/f/not-an-action.json",
        source.as_str()
    ));
    backend
        .put(
            &malformed_source,
            Entry::record(
                RecordKind::new("product_workflow_action").expect("old kind"),
                &serde_json::json!({"broken": true}),
            )
            .expect("malformed entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed malformed source");
    let error = migrate_idempotency_ledger_root(
        backend.as_ref(),
        &source,
        &path("/tenants/tenant-a/shared/channel-extensions/telegram/empty-target"),
    )
    .await
    .expect_err("malformed source must fail closed");
    assert_eq!(error, IdempotencyLedgerMigrationError::MalformedSource);
}

#[tokio::test]
async fn migration_reads_more_than_one_backend_page_and_skips_old_prune_lease() {
    let backend = Arc::new(InMemoryBackend::new());
    let source = source_root();
    let target = target_root();
    let scope = scope();
    let action_count = Page::MAX_LIMIT as usize + 1;
    for suffix in 0..action_count {
        let action = ProductInboundAction::begin(fingerprint(suffix), Utc::now());
        put_old_action(backend.as_ref(), &source, &scope, &action).await;
    }
    let lease_path = path(format!(
        "{}/{}/_control/prune_lease.json",
        source.as_str(),
        scope_suffix(&scope)
    ));
    backend
        .put(
            &lease_path,
            Entry::record(
                RecordKind::new("product_workflow_prune_lease").expect("old lease kind"),
                &serde_json::json!({"expires_at_ms": Utc::now().timestamp_millis()}),
            )
            .expect("lease entry"),
            CasExpectation::Absent,
        )
        .await
        .expect("seed old lease");

    let report = migrate_idempotency_ledger_root(backend.as_ref(), &source, &target)
        .await
        .expect("paged migration");
    assert_eq!(report.scanned_actions, action_count);
    assert_eq!(report.migrated_actions, action_count);
    assert_eq!(report.skipped_transient_leases, 1);
    let migrated = backend
        .query(&target, &Filter::All, Page::new(0, Page::MAX_LIMIT))
        .await
        .expect("first target page");
    assert_eq!(migrated.len(), Page::MAX_LIMIT as usize);
    let migrated_tail = backend
        .query(
            &target,
            &Filter::All,
            Page::new(u64::from(Page::MAX_LIMIT), Page::MAX_LIMIT),
        )
        .await
        .expect("second target page");
    assert_eq!(migrated_tail.len(), 1);
}
