#![cfg(any(feature = "libsql", feature = "postgres"))]

//! Durable contract for [`FilesystemCapabilityPolicyDeltaStore`] (issue #5273).
//!
//! The in-tree `#[cfg(test)]` module in `capability_policy_delta.rs` exercises
//! the store over an `InMemoryBackend`. This file mirrors
//! `tests/durable_ledger_contract.rs`: it constructs the same store over the
//! real durable backends (`LibSqlRootFilesystem`, and `PostgresRootFilesystem`
//! when its feature + a server are present) and asserts the durable contract —
//! cross-instance persistence, idempotent delete against the backend's own
//! zero-rows delete, missing-prefix→empty, a `config_patch` JSON round-trip,
//! and the [`StoreBackedPolicyResolver`] fold over the persisted rows.

use std::sync::Arc;
#[cfg(feature = "postgres")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_host_api::{CapabilityId, PermissionMode, TenantId, UserId, VirtualPath};

use ironclaw_capability_policy::{
    Availability, CapabilityDefaultPolicy, CapabilityPolicyDelta, CapabilityPolicyDeltaStore,
    IdentityMode, PolicyResolver, PolicyScope, PolicySubject, StaticCapabilityDefaultPolicySource,
    StoreBackedPolicyResolver,
};
use ironclaw_product_workflow_storage::FilesystemCapabilityPolicyDeltaStore;
use serde_json::json;

const TENANT: &str = "tenant:acme";
const CAP: &str = "nearai.web_search";

fn delta_root(suffix: &str) -> VirtualPath {
    VirtualPath::new(format!(
        "/engine/product_workflow/capability_policy/test_roots/{suffix}"
    ))
    .expect("valid capability policy delta root")
}

fn tenant() -> TenantId {
    TenantId::from_trusted(TENANT.to_string())
}

fn cap() -> CapabilityId {
    CapabilityId::new(CAP).expect("valid capability id")
}

fn subject(user: &str) -> PolicySubject {
    PolicySubject {
        tenant_id: tenant(),
        user_id: UserId::from_trusted(user.to_string()),
    }
}

fn tenant_delta() -> CapabilityPolicyDelta {
    CapabilityPolicyDelta {
        scope: PolicyScope::Tenant,
        capability: cap(),
        availability: Some(Availability::Available),
        identity: Some(IdentityMode::AdminKeyed),
        approval: Some(PermissionMode::Allow),
        config_patch: Some(json!({ "workspace": "acme", "nested": { "a": 1 } })),
    }
}

fn user_delta(user: &str) -> CapabilityPolicyDelta {
    CapabilityPolicyDelta {
        scope: PolicyScope::User {
            user_id: UserId::from_trusted(user.to_string()),
        },
        capability: cap(),
        availability: None,
        identity: None,
        approval: Some(PermissionMode::Deny),
        config_patch: Some(json!({ "verbose": true, "nested": { "b": 2 } })),
    }
}

/// Drive the full durable contract through two store instances sharing the same
/// backend + root, mirroring the reopen pattern in `durable_ledger_contract.rs`.
async fn assert_capability_policy_delta_store_contract(
    backend: Arc<dyn RootFilesystem>,
    root: VirtualPath,
) {
    // A never-written store returns `[]`, not an error — guards the
    // missing-prefix path on the real backend (some surface it as NotFound).
    let store = FilesystemCapabilityPolicyDeltaStore::with_root(Arc::clone(&backend), root.clone());
    assert!(
        store
            .deltas_for(&subject("user:bob"), &cap())
            .await
            .expect("missing prefix deltas_for is Ok")
            .is_empty(),
        "deltas_for on an empty store returns []"
    );
    assert!(
        store
            .list_subject_deltas(&subject("user:bob"))
            .await
            .expect("missing prefix list is Ok")
            .is_empty(),
        "list_subject_deltas on an empty store returns []"
    );

    // Idempotent delete: removing an absent delta is a no-op against the real
    // backend's zero-rows delete (NotFound is mapped to Ok).
    store
        .delete_delta(&tenant(), &PolicyScope::Tenant, &cap())
        .await
        .expect("deleting an absent delta is a no-op on the durable backend");

    // Upsert → read-back via deltas_for.
    store
        .upsert_delta(&tenant(), tenant_delta())
        .await
        .expect("upsert tenant delta");
    store
        .upsert_delta(&tenant(), user_delta("user:bob"))
        .await
        .expect("upsert user delta");

    let bob = store
        .deltas_for(&subject("user:bob"), &cap())
        .await
        .expect("deltas_for bob");
    assert_eq!(
        bob.len(),
        2,
        "Bob reads back the tenant row + his own user row"
    );

    // config_patch JSON round-trip: the persisted-then-read row reproduces the
    // exact JSON value (including the nested object) it was written with.
    let bob_user = bob
        .iter()
        .find(|delta| matches!(&delta.scope, PolicyScope::User { .. }))
        .expect("user-scope row present");
    assert_eq!(
        bob_user.config_patch,
        Some(json!({ "verbose": true, "nested": { "b": 2 } })),
        "config_patch round-trips through the durable backend byte-for-byte"
    );
    assert_eq!(bob_user.approval, Some(PermissionMode::Deny));

    let carol = store
        .deltas_for(&subject("user:carol"), &cap())
        .await
        .expect("deltas_for carol");
    assert_eq!(
        carol.len(),
        1,
        "Carol reads only the tenant row, not Bob's user row"
    );
    assert_eq!(carol[0].scope, PolicyScope::Tenant);

    // Cross-instance persistence: a SECOND store over the SAME backend + root
    // reads the rows the first instance wrote — proves they live on the
    // backend, not in per-instance memory.
    drop(store);
    let reopened =
        FilesystemCapabilityPolicyDeltaStore::with_root(Arc::clone(&backend), root.clone());
    let bob_reopened = reopened
        .deltas_for(&subject("user:bob"), &cap())
        .await
        .expect("reopened deltas_for bob");
    assert_eq!(
        bob_reopened.len(),
        2,
        "a fresh store instance reads deltas a prior instance persisted"
    );
    let carol_listed = reopened
        .list_subject_deltas(&subject("user:carol"))
        .await
        .expect("reopened list carol");
    assert_eq!(
        carol_listed.len(),
        1,
        "tenant row survives across instances"
    );

    // StoreBackedPolicyResolver fold over the durable store: default → tenant →
    // user, most specific wins on the replaced dimensions, config_patch
    // deep-merges in scope order.
    let defaults =
        StaticCapabilityDefaultPolicySource::new(CapabilityDefaultPolicy::conservative_fallback());
    let resolver = StoreBackedPolicyResolver::new(defaults, reopened);

    let bob_effective = resolver
        .resolve(&subject("user:bob"), &cap())
        .await
        .expect("resolve bob");
    assert!(
        bob_effective.available,
        "tenant delta flipped availability on"
    );
    assert_eq!(bob_effective.identity, IdentityMode::AdminKeyed);
    assert_eq!(
        bob_effective.approval,
        PermissionMode::Deny,
        "user row wins on approval"
    );
    assert_eq!(
        bob_effective.config,
        json!({ "workspace": "acme", "verbose": true, "nested": { "a": 1, "b": 2 } }),
        "config_patch deep-merges default → tenant → user"
    );

    let carol_effective = resolver
        .resolve(&subject("user:carol"), &cap())
        .await
        .expect("resolve carol");
    assert_eq!(
        carol_effective.approval,
        PermissionMode::Allow,
        "only the tenant row applies to Carol"
    );
    assert_eq!(
        carol_effective.config,
        json!({ "workspace": "acme", "nested": { "a": 1 } })
    );
}

#[cfg(feature = "libsql")]
async fn libsql_filesystem(path: &str) -> Arc<LibSqlRootFilesystem> {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem
        .run_migrations()
        .await
        .expect("run libsql filesystem migrations");
    filesystem
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_capability_policy_delta_store_contract() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("capability-policy-delta.db");
    let db_path = db_path.display().to_string();
    let filesystem = libsql_filesystem(&db_path).await;

    assert_capability_policy_delta_store_contract(filesystem, delta_root("libsql-deltas")).await;
}

#[cfg(feature = "postgres")]
fn unique_suffix(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after unix epoch")
        .as_nanos();
    format!("{name}-{nanos}")
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_capability_policy_delta_store_contract_when_configured() {
    let Some(filesystem) = postgres_filesystem().await else {
        return;
    };
    let root = delta_root(&unique_suffix("postgres-deltas"));
    assert_capability_policy_delta_store_contract(filesystem, root).await;
}

#[cfg(feature = "postgres")]
async fn postgres_filesystem() -> Option<Arc<PostgresRootFilesystem>> {
    let url = match std::env::var("IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!(
                "skipping postgres capability policy delta contract: IRONCLAW_PRODUCT_WORKFLOW_POSTGRES_URL not set"
            );
            return None;
        }
    };
    let config = match url.parse::<tokio_postgres::Config>() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("skipping postgres capability policy delta contract: invalid url ({error})");
            return None;
        }
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .expect("postgres pool builds");
    if let Err(error) = pool.get().await {
        eprintln!(
            "skipping postgres capability policy delta contract: database unavailable ({error})"
        );
        return None;
    }
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool));
    if let Err(error) = filesystem.run_migrations().await {
        eprintln!(
            "skipping postgres capability policy delta contract: filesystem migrations failed ({error})"
        );
        return None;
    }
    Some(filesystem)
}
