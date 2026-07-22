use std::collections::BTreeMap;
use std::sync::Arc;

use ironclaw_extension_host::{
    AdminConfigurationIdempotencyKey, AdminConfigurationRequestDigest,
    AdminConfigurationReserveOutcome, AdminConfigurationStoreError, AdminConfigurationValueRef,
    FilesystemAdminConfigurationStore,
};
use ironclaw_extensions::AdminConfigurationGroupId;
use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, SecretHandle,
    TenantId, UserId, VirtualPath,
};

#[tokio::test]
async fn replace_persists_a_tenant_scoped_group_snapshot() {
    let store = in_memory_store();
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");

    let reservation = reserve(&store, &scope, &group_id, "save-1", 1).await;
    let committed = store
        .commit(
            &scope,
            &reservation,
            values_for_revision(reservation.revision, "client-a"),
        )
        .await
        .expect("first replace must commit");

    assert_eq!(committed.revision, 1);
    let loaded = store
        .get(&scope, &group_id)
        .await
        .expect("load must succeed")
        .expect("record must exist");
    assert_eq!(loaded.tenant_id, scope.tenant_id);
    assert_eq!(loaded.group_id, group_id);
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.values, committed.values);
}

#[tokio::test]
async fn exact_idempotency_replay_survives_later_revisions_without_preparing_again() {
    let store = in_memory_store();
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");
    let first_key = idempotency_key("save-first");
    let first_digest = request_digest(1);

    let first_reservation = match store
        .reserve(&scope, &group_id, &first_key, first_digest, 0)
        .await
        .expect("first reserve")
    {
        AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        AdminConfigurationReserveOutcome::Replay(_) => panic!("first call cannot replay"),
    };
    let first = store
        .commit(
            &scope,
            &first_reservation,
            values_for_revision(first_reservation.revision, "client-a"),
        )
        .await
        .expect("first commit");
    let second_reservation = reserve(&store, &scope, &group_id, "save-second", 2).await;
    let second = store
        .commit(
            &scope,
            &second_reservation,
            values_for_revision(second_reservation.revision, "client-b"),
        )
        .await
        .expect("second commit");
    assert_eq!(second.revision, 2);

    let replayed = store
        .reserve(&scope, &group_id, &first_key, first_digest, 0)
        .await
        .expect("exact replay must succeed");

    assert_eq!(
        replayed,
        AdminConfigurationReserveOutcome::Replay(first),
        "replay must return the original receipt before any secret staging",
    );
    let current = store.get(&scope, &group_id).await.unwrap().unwrap();
    assert_eq!(
        current.revision, 2,
        "replay must not roll back current state"
    );
    assert_eq!(current.values, second.values);
}

#[tokio::test]
async fn idempotency_key_reuse_with_different_input_fails_closed() {
    let store = in_memory_store();
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");
    let key = idempotency_key("save-1");

    let reservation = match store
        .reserve(&scope, &group_id, &key, request_digest(1), 0)
        .await
        .unwrap()
    {
        AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        AdminConfigurationReserveOutcome::Replay(_) => panic!("first call cannot replay"),
    };
    store
        .commit(
            &scope,
            &reservation,
            values_for_revision(reservation.revision, "client-a"),
        )
        .await
        .unwrap();

    let error = store
        .reserve(&scope, &group_id, &key, request_digest(2), 1)
        .await
        .expect_err("same key with different digest must fail");
    assert_eq!(error, AdminConfigurationStoreError::IdempotencyConflict);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reservations_are_unique_and_serialized_by_backend_cas() {
    let store = Arc::new(in_memory_store());
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");
    let mut tasks = Vec::new();

    for index in 0_u8..12 {
        let store = Arc::clone(&store);
        let scope = scope.clone();
        let group_id = group_id.clone();
        tasks.push(tokio::spawn(async move {
            store
                .reserve(
                    &scope,
                    &group_id,
                    &idempotency_key(&format!("save-{index}")),
                    request_digest(index),
                    0,
                )
                .await
        }));
    }

    let mut revisions = Vec::new();
    for task in tasks {
        let outcome = task.await.unwrap().unwrap();
        let AdminConfigurationReserveOutcome::Reserved(reservation) = outcome else {
            panic!("unique key cannot replay");
        };
        revisions.push(reservation.revision);
    }
    revisions.sort_unstable();
    assert_eq!(revisions, (1_u64..=12).collect::<Vec<_>>());
}

#[tokio::test]
async fn stale_concurrent_writer_cannot_overwrite_a_committed_revision() {
    let store = in_memory_store();
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");
    let first = match store
        .reserve(
            &scope,
            &group_id,
            &idempotency_key("writer-a"),
            request_digest(1),
            0,
        )
        .await
        .unwrap()
    {
        AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        AdminConfigurationReserveOutcome::Replay(_) => panic!("new key cannot replay"),
    };
    let stale = match store
        .reserve(
            &scope,
            &group_id,
            &idempotency_key("writer-b"),
            request_digest(2),
            0,
        )
        .await
        .unwrap()
    {
        AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        AdminConfigurationReserveOutcome::Replay(_) => panic!("new key cannot replay"),
    };
    store
        .commit(
            &scope,
            &first,
            values_for_revision(first.revision, "client-a"),
        )
        .await
        .unwrap();

    let error = store
        .commit(
            &scope,
            &stale,
            values_for_revision(stale.revision, "client-b"),
        )
        .await
        .unwrap_err();

    assert_eq!(
        error,
        AdminConfigurationStoreError::RevisionConflict {
            expected: 0,
            actual: 1,
        }
    );
    assert_eq!(
        store.get(&scope, &group_id).await.unwrap().unwrap().values,
        values_for_revision(first.revision, "client-a"),
    );
}

#[tokio::test]
async fn repeated_large_saves_do_not_multiply_values_inside_the_group_record() {
    let store = in_memory_store();
    let scope = sample_scope("tenant-a", "operator-a");
    let group_id = group_id("vendor.example");
    let large_value = "x".repeat(16 * 1024);

    for index in 0_u8..80 {
        let reservation = reserve(
            &store,
            &scope,
            &group_id,
            &format!("large-save-{index}"),
            index,
        )
        .await;
        store
            .commit(
                &scope,
                &reservation,
                BTreeMap::from([(
                    SecretHandle::new("client_id").unwrap(),
                    AdminConfigurationValueRef::Inline(large_value.clone()),
                )]),
            )
            .await
            .unwrap();
    }

    assert_eq!(
        store
            .get(&scope, &group_id)
            .await
            .unwrap()
            .unwrap()
            .revision,
        80
    );
}

#[tokio::test]
async fn stored_tenant_mismatch_is_not_disclosed() {
    let backend = Arc::new(InMemoryBackend::new());
    let store = FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::clone(&backend)));
    let tenant_a = sample_scope("tenant-a", "operator-a");
    let tenant_b = sample_scope("tenant-b", "operator-b");
    let group_id = group_id("vendor.example");
    let reservation = reserve(&store, &tenant_a, &group_id, "save-a", 1).await;
    store
        .commit(
            &tenant_a,
            &reservation,
            values_for_revision(reservation.revision, "client-a"),
        )
        .await
        .unwrap();

    assert_eq!(store.get(&tenant_b, &group_id).await.unwrap(), None);
}

fn values_for_revision(
    revision: u64,
    client_id: &str,
) -> BTreeMap<SecretHandle, AdminConfigurationValueRef> {
    BTreeMap::from([
        (
            SecretHandle::new("client_id").unwrap(),
            AdminConfigurationValueRef::Inline(client_id.to_string()),
        ),
        (
            SecretHandle::new("client_secret").unwrap(),
            AdminConfigurationValueRef::Secret(
                SecretHandle::new(format!("admincfg-client-secret-r{revision}")).unwrap(),
            ),
        ),
    ])
}

fn in_memory_store() -> FilesystemAdminConfigurationStore<InMemoryBackend> {
    FilesystemAdminConfigurationStore::new(scoped_admin_fs(Arc::new(InMemoryBackend::new())))
}

fn scoped_admin_fs<F>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/extension-admin-configuration").unwrap(),
        VirtualPath::new("/engine/tenants/test/admin-configuration").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    Arc::new(ScopedFilesystem::with_fixed_view(backend, view))
}

fn sample_scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn group_id(value: &str) -> AdminConfigurationGroupId {
    AdminConfigurationGroupId::new(value).unwrap()
}

fn idempotency_key(value: &str) -> AdminConfigurationIdempotencyKey {
    AdminConfigurationIdempotencyKey::new(value).unwrap()
}

fn request_digest(discriminator: u8) -> AdminConfigurationRequestDigest {
    AdminConfigurationRequestDigest::from_bytes([discriminator; 32])
}

async fn reserve<F>(
    store: &FilesystemAdminConfigurationStore<F>,
    scope: &ResourceScope,
    group_id: &AdminConfigurationGroupId,
    key: &str,
    digest: u8,
) -> ironclaw_extension_host::AdminConfigurationReservation
where
    F: RootFilesystem,
{
    match store
        .reserve(
            scope,
            group_id,
            &idempotency_key(key),
            request_digest(digest),
            store
                .get(scope, group_id)
                .await
                .expect("revision read")
                .map_or(0, |record| record.revision),
        )
        .await
        .expect("reservation must succeed")
    {
        AdminConfigurationReserveOutcome::Reserved(reservation) => reservation,
        AdminConfigurationReserveOutcome::Replay(_) => panic!("new key cannot replay"),
    }
}
