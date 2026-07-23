//! Durable [`AttestedGateBindingStore`] backends: the authoritative binding
//! must survive a store reopen (durability) and be readable from the sync
//! [`SyncBindingRead`] path the resume port uses (no split-brain with a
//! separate in-memory store). libSQL runs against a local temp file.

#![cfg(all(feature = "integration", feature = "libsql"))]

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use ironclaw_attestation::{DecodedTransaction, RenderingSchemaVersion};
use ironclaw_attested_runtime::{AttestedGateBinding, AttestedGateBindingStore, SyncBindingRead};
use ironclaw_attested_store::LibSqlAttestedGateBindingStore;
use ironclaw_chain_signing::{ChainKeyId, evm};
use ironclaw_host_api::{ResourceScope, TenantId, UserId};
use ironclaw_signing_provider::{
    ActorId, ChainId, GateRef, KeyOrAccountId, ProviderId, RunId, ScopeId, SigningContext,
    TenantId as SigningTenantId, UserId as SigningUserId,
};

const GATE: &str = "gate:durable-binding";
// The gate-bound signer carried in `SigningContext.key_or_account_id`; folded
// into the approved-tx hash exactly as the approval path does (WYSIWYS).
const SIGNER: &str = "0000000000000000000000000000000000000000";

fn sample_binding() -> AttestedGateBinding {
    let tx = TxEip1559 {
        chain_id: 11155111,
        nonce: 1,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0x11)),
        value: U256::from(5u64),
        input: Bytes::new(),
        access_list: Default::default(),
    };
    let decoded: DecodedTransaction = evm::decode_eip1559(&tx);
    let approved_tx_hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        SIGNER,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    AttestedGateBinding {
        provider_id: ProviderId::Injected,
        context: SigningContext {
            tenant: SigningTenantId::new("tenant1"),
            user: SigningUserId::new("user1"),
            scope: ScopeId::new("scope"),
            actor: ActorId::new("actor"),
            run_id: RunId::new("run"),
            gate_ref: GateRef::new(GATE),
            chain_id: ChainId::new("eip155:11155111"),
            key_or_account_id: KeyOrAccountId::new(SIGNER),
        },
        approved_tx_hash,
        decoded,
        chain: ChainKeyId::new("eip155:11155111").expect("valid chain id in test"),
        scope: ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        },
        schema_version: RenderingSchemaVersion::CURRENT,
    }
}

async fn build(path: &std::path::Path) -> LibSqlAttestedGateBindingStore {
    let db = Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build libsql db"),
    );
    LibSqlAttestedGateBindingStore::connect(db)
        .await
        .expect("connect binding store")
}

#[tokio::test]
async fn put_then_async_and_sync_read_agree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.db");
    let store = build(&path).await;
    let gate = GateRef::new(GATE);

    assert!(store.get(&gate).await.is_none());
    assert!(store.get_sync(&gate).is_none());

    let binding = sample_binding();
    store
        .put(gate.clone(), binding.clone())
        .await
        .expect("first put succeeds");

    let via_async = store.get(&gate).await.expect("async read");
    let via_sync = store.get_sync(&gate).expect("sync read");
    assert_eq!(via_async.approved_tx_hash, binding.approved_tx_hash);
    assert_eq!(via_sync.approved_tx_hash, binding.approved_tx_hash);
    assert_eq!(via_sync.chain, binding.chain);
}

#[tokio::test]
async fn binding_is_immutable_after_first_put() {
    // A binding must never change after approval: a second put for the same
    // gate_ref is rejected at the DB level, and both the async and sync read
    // paths keep returning the ORIGINAL binding.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.db");
    let store = build(&path).await;
    let gate = GateRef::new(GATE);

    let original = sample_binding();
    store
        .put(gate.clone(), original.clone())
        .await
        .expect("first put succeeds");
    let original_hash = original.approved_tx_hash;

    // Build a DIFFERENT binding (different approved tx -> different hash) and
    // attempt to overwrite the same gate_ref.
    let mut tampered = sample_binding();
    let tx = TxEip1559 {
        chain_id: 11155111,
        nonce: 99,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0x22)),
        value: U256::from(999u64),
        input: Bytes::new(),
        access_list: Default::default(),
    };
    let decoded: DecodedTransaction = evm::decode_eip1559(&tx);
    tampered.approved_tx_hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        SIGNER,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    tampered.decoded = decoded;
    assert_ne!(
        tampered.approved_tx_hash, original_hash,
        "test setup: tampered binding must differ"
    );

    let rejected = store.put(gate.clone(), tampered).await;
    assert_eq!(
        rejected,
        Err(ironclaw_attested_runtime::BindingError::AlreadyExists),
        "a second put for the same gate_ref must be rejected (immutable binding)"
    );

    // The overwrite was rejected: original binding still stands on both paths.
    let via_async = store.get(&gate).await.expect("async read");
    let via_sync = store.get_sync(&gate).expect("sync read");
    assert_eq!(via_async.approved_tx_hash, original_hash);
    assert_eq!(via_sync.approved_tx_hash, original_hash);

    // And it survives a reopen (the durable row was never updated).
    drop(store);
    let reopened = build(&path).await;
    assert_eq!(
        reopened
            .get_sync(&gate)
            .expect("rehydrated")
            .approved_tx_hash,
        original_hash
    );
}

#[tokio::test]
async fn async_get_falls_back_to_db_on_cache_miss() {
    // Simulate a binding written by another process/replica (or after this
    // instance's `load()` ran): the row is in the durable table but was never
    // write-through into THIS store's cache. The async `get` must fall back to
    // the table, return the row, and warm the cache.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.db");
    let gate = GateRef::new(GATE);
    let binding = sample_binding();

    // Construct the store first (runs load() over an empty/rowless table).
    let store = build(&path).await;
    assert!(
        store.get_sync(&gate).is_none(),
        "cache must be empty before the out-of-band write"
    );

    // Out-of-band write straight to the durable table, bypassing the store's
    // cache entirely — mimics a second replica's put.
    {
        let db = Arc::new(
            libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("build libsql db for out-of-band write"),
        );
        let conn = db.connect().expect("connect for out-of-band write");
        let json = serde_json::to_string(&binding).expect("serialize binding");
        conn.execute(
            "INSERT INTO attested_gate_bindings (gate_ref, binding_json) VALUES (?1, ?2)",
            libsql::params![gate.as_str(), json],
        )
        .await
        .expect("out-of-band insert");
    }

    // Sync path still misses (no DB I/O allowed there) — proves the cache was
    // genuinely not populated for this row.
    assert!(
        store.get_sync(&gate).is_none(),
        "sync read must not see the out-of-band row before async warms the cache"
    );

    // Async read-through: finds the row in the table and returns it.
    let via_async = store
        .get(&gate)
        .await
        .expect("async get must fall back to the db on cache miss");
    assert_eq!(via_async.approved_tx_hash, binding.approved_tx_hash);

    // The read-through warmed the cache, so the sync resume path now sees it.
    let via_sync = store
        .get_sync(&gate)
        .expect("sync read after async read-through warmed the cache");
    assert_eq!(via_sync.approved_tx_hash, binding.approved_tx_hash);
}

#[tokio::test]
async fn binding_survives_store_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("bindings.db");
    let gate = GateRef::new(GATE);
    let binding = sample_binding();

    {
        let store = build(&path).await;
        store
            .put(gate.clone(), binding.clone())
            .await
            .expect("first put succeeds");
    }

    // Reopen: the cache is rehydrated from the durable table, so the sync read
    // path works after a restart (no split-brain).
    let reopened = build(&path).await;
    let rehydrated = reopened.get_sync(&gate).expect("rehydrated sync read");
    assert_eq!(rehydrated.approved_tx_hash, binding.approved_tx_hash);
    assert_eq!(
        rehydrated.context.gate_ref.as_str(),
        binding.context.gate_ref.as_str()
    );
}
