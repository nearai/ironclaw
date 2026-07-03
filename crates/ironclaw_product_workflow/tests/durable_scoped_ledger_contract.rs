// The generic filesystem-backed ledger folded in from
// `ironclaw_product_workflow_storage` lives behind the `storage` feature.
#![cfg(feature = "storage")]

use std::num::NonZeroUsize;
use std::sync::Arc;

use chrono::{Duration, Utc};
use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RecordKind, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
    ResourceScope, ScopedPath, TenantId, UserId, VirtualPath,
};
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_product_workflow::RebornFilesystemIdempotencyLedger;
use ironclaw_product_workflow::{
    ActionFingerprintKey, IdempotencyDecision, IdempotencyLedger, ProductWorkflowError,
};

#[path = "durable_ledger_support/mod.rs"]
mod support;

use support::*;

fn scoped_custom_root(suffix: &str) -> ScopedPath {
    ScopedPath::new(format!(
        "/engine/product_workflow/idempotency/test_roots/{suffix}"
    ))
    .expect("valid scoped custom ledger root")
}

fn scoped_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let backend = Arc::new(InMemoryBackend::new());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/engine").expect("engine alias"),
        VirtualPath::new("/engine/scoped-workflow-storage").expect("engine target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn resource_scope(user_id: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant:workflow-storage").expect("tenant"),
        user_id: UserId::new(user_id).expect("user"),
        agent_id: Some(AgentId::new("agent:workflow-storage").expect("agent")),
        project_id: Some(ProjectId::new("project:workflow-storage").expect("project")),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn scoped_prune_lease_path(scope: &ResourceScope) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/_control/prune_lease.json",
        scoped_ledger_root(scope).as_str()
    ))
    .expect("valid prune lease path")
}

fn scoped_action_path(scope: &ResourceScope, fingerprint: &ActionFingerprintKey) -> ScopedPath {
    ScopedPath::new(format!(
        "{}/{}/{}/{}/{}/{}/{}.json",
        scoped_ledger_root(scope).as_str(),
        hex_component(fingerprint.adapter_id.as_str()),
        hex_component(fingerprint.installation_id.as_str()),
        hex_component(fingerprint.external_actor_ref.kind()),
        hex_component(fingerprint.external_actor_ref.id()),
        hex_component(fingerprint.source_binding_key.as_str()),
        hex_component(fingerprint.external_event_id.as_str())
    ))
    .expect("valid scoped action path")
}

fn scoped_ledger_root(scope: &ResourceScope) -> ScopedPath {
    let agent_id = scope
        .agent_id
        .as_ref()
        .map(|agent_id| agent_id.as_str())
        .unwrap_or("_");
    let project_id = scope
        .project_id
        .as_ref()
        .map(|project_id| project_id.as_str())
        .unwrap_or("_");
    let mission_id = scope
        .mission_id
        .as_ref()
        .map(|mission_id| mission_id.as_str())
        .unwrap_or("_");
    let thread_id = scope
        .thread_id
        .as_ref()
        .map(|thread_id| thread_id.as_str())
        .unwrap_or("_");
    ScopedPath::new(format!(
        "/engine/product_workflow/idempotency/actions/_scope/{}/{}/{}/{}/{}/{}",
        hex_component(scope.tenant_id.as_str()),
        hex_component(scope.user_id.as_str()),
        hex_component(agent_id),
        hex_component(project_id),
        hex_component(mission_id),
        hex_component(thread_id)
    ))
    .expect("valid ledger root")
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

async fn write_scoped_prune_lease(
    filesystem: &ScopedFilesystem<InMemoryBackend>,
    scope: &ResourceScope,
    expires_at_ms: i64,
) {
    let payload = serde_json::json!({ "expires_at_ms": expires_at_ms });
    let entry = Entry::record(
        RecordKind::new("product_workflow_prune_lease").expect("valid record kind"),
        &payload,
    )
    .expect("valid prune lease entry");
    filesystem
        .put(
            scope,
            &scoped_prune_lease_path(scope),
            entry,
            CasExpectation::Absent,
        )
        .await
        .expect("write prune lease");
}

#[tokio::test]
async fn scoped_filesystem_settled_action_replays() {
    let filesystem = scoped_filesystem();
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        resource_scope("user:scoped-replay"),
        Duration::seconds(10),
    );
    let reopened = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        resource_scope("user:scoped-replay"),
        Duration::seconds(10),
    );

    assert_settled_action_survives_reopen_and_replays(&ledger, &reopened, "scoped-replay").await;
}

#[tokio::test]
async fn scoped_filesystem_settled_action_isolated_across_scopes() {
    let filesystem = scoped_filesystem();
    let alpha = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        resource_scope("user:scope-alpha"),
        Duration::seconds(10),
    );
    let beta = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        resource_scope("user:scope-beta"),
        Duration::seconds(10),
    );
    let received_at = Utc::now();
    let shared = fingerprint("scoped-shared");

    settle_noop(&alpha, shared.clone(), received_at).await;

    assert!(matches!(
        beta.begin_or_replay(shared.clone(), received_at + Duration::seconds(1))
            .await
            .expect("beta must not see alpha action"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        alpha
            .begin_or_replay(shared, received_at + Duration::seconds(1))
            .await
            .expect("alpha still replays its own action"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn scoped_filesystem_in_flight_action_blocks_until_lease_expires() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-lease"),
        Duration::seconds(10),
    );

    assert_in_flight_action_blocks_until_lease_expires(&ledger, "scoped-lease").await;
}

#[tokio::test]
async fn scoped_filesystem_fresh_prune_lease_skips_retention() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-fresh-prune-lease");
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        scope.clone(),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(2).expect("non-zero interval"));
    let received_at = Utc::now();
    let oldest = fingerprint("scoped-fresh-prune-lease-oldest");
    let newest = fingerprint("scoped-fresh-prune-lease-newest");

    settle_noop(&ledger, oldest.clone(), received_at).await;
    write_scoped_prune_lease(
        filesystem.as_ref(),
        &scope,
        (Utc::now() + Duration::seconds(30)).timestamp_millis(),
    )
    .await;
    settle_noop(&ledger, newest, received_at + Duration::seconds(1)).await;

    assert!(matches!(
        ledger
            .begin_or_replay(oldest, received_at + Duration::seconds(2))
            .await
            .expect("fresh prune lease skips retention"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn scoped_filesystem_expired_prune_lease_allows_retention() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-expired-prune-lease");
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        scope.clone(),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(2).expect("non-zero interval"));
    let received_at = Utc::now();
    let oldest = fingerprint("scoped-expired-prune-lease-oldest");
    let newest = fingerprint("scoped-expired-prune-lease-newest");

    settle_noop(&ledger, oldest.clone(), received_at).await;
    write_scoped_prune_lease(
        filesystem.as_ref(),
        &scope,
        (Utc::now() - Duration::seconds(30)).timestamp_millis(),
    )
    .await;
    settle_noop(&ledger, newest.clone(), received_at + Duration::seconds(1)).await;

    assert!(matches!(
        ledger
            .begin_or_replay(oldest, received_at + Duration::seconds(2))
            .await
            .expect("expired prune lease allows retention"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(newest, received_at + Duration::seconds(2))
            .await
            .expect("newest remains available for replay"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn scoped_filesystem_prunes_terminal_replay() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-terminal-and-stale-prune");
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        scope.clone(),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));
    let received_at = Utc::now();
    let terminal_replay = fingerprint("scoped-terminal-replay-prune");
    let newest = fingerprint("scoped-terminal-replay-newest");

    let IdempotencyDecision::New(mut terminal_action) = ledger
        .begin_or_replay(terminal_replay.clone(), received_at)
        .await
        .expect("begin terminal replay")
    else {
        panic!("expected terminal replay action");
    };
    terminal_action.mark_deduplicated(ProductInboundAck::NoOp);
    ledger
        .settle(terminal_action)
        .await
        .expect("settle terminal replay");
    settle_noop(&ledger, newest.clone(), received_at + Duration::seconds(1)).await;

    assert!(matches!(
        ledger
            .begin_or_replay(terminal_replay, received_at + Duration::seconds(2))
            .await
            .expect("terminal replay was pruned"),
        IdempotencyDecision::New(_)
    ));
    assert!(matches!(
        ledger
            .begin_or_replay(newest, received_at + Duration::seconds(2))
            .await
            .expect("newest remains available for replay"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn scoped_filesystem_corrupt_action_record_returns_transient() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-corrupt-action");
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        scope.clone(),
        Duration::seconds(10),
    );
    let fingerprint = fingerprint("scoped-corrupt-action");
    let path = scoped_action_path(&scope, &fingerprint);
    filesystem
        .put(
            &scope,
            &path,
            Entry::bytes(b"not-json".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed corrupt action record");

    let error = ledger
        .begin_or_replay(fingerprint, Utc::now())
        .await
        .expect_err("corrupt action record returns transient");
    assert!(matches!(error, ProductWorkflowError::Transient { .. }));
}

#[tokio::test]
async fn scoped_filesystem_duplicate_reservation_contention_serializes() {
    let filesystem = scoped_filesystem();
    let first = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        Arc::clone(&filesystem),
        resource_scope("user:scoped-contention"),
        Duration::seconds(10),
    );
    let second = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        resource_scope("user:scoped-contention"),
        Duration::seconds(10),
    );

    assert_duplicate_reservation_contention_serializes(&first, &second, "scoped-contention").await;
}

#[tokio::test]
async fn scoped_filesystem_settled_entry_limit_prunes_oldest() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-retention"),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"));

    assert_settled_entry_limit_prunes_oldest(&ledger, "scoped-retention").await;
}

#[tokio::test]
async fn scoped_filesystem_settled_prune_interval_defers_until_interval() {
    let ledger = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        scoped_filesystem(),
        resource_scope("user:scoped-prune-interval"),
        Duration::seconds(10),
    )
    .with_settled_entry_limit(NonZeroUsize::new(1).expect("non-zero limit"))
    .with_settled_prune_interval(NonZeroUsize::new(3).expect("non-zero interval"));

    assert_settled_prune_interval_defers_until_interval(&ledger, "scoped-prune-interval").await;
}

#[tokio::test]
async fn scoped_filesystem_custom_root_isolated_from_default_root() {
    let filesystem = scoped_filesystem();
    let scope = resource_scope("user:scoped-custom-root");
    let custom = RebornFilesystemIdempotencyLedger::with_root(
        Arc::clone(&filesystem),
        scope.clone(),
        scoped_custom_root("scoped"),
        Duration::seconds(60),
    );
    let default = RebornFilesystemIdempotencyLedger::with_in_flight_lease(
        filesystem,
        scope,
        Duration::seconds(60),
    );

    assert_custom_root_isolated_from_default_root(&custom, &default, "scoped-custom-root").await;
}
