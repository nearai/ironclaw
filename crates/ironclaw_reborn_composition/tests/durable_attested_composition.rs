//! Backend selection: the attested-signing composition assembles over the
//! DURABLE libSQL stores + the real per-chain broadcaster, not just the
//! in-memory reference impl. This is the production wiring shape — the same
//! `RebornAttestedComposition::assemble` seam the runtime uses, but
//! monomorphized to the durable grant store / ledger and
//! `MultiChainBroadcaster`. Proves the generics line up and the durable
//! stores migrate cleanly.

#![cfg(all(feature = "libsql", feature = "attested-broadcast"))]

use std::sync::Arc;

use ironclaw_attested_runtime::{
    AttestedGateBindingStore, CustodialMainnetShipGate, InMemoryAttestedGateBindingStore,
    ProviderRegistry,
};
use ironclaw_attested_store::{
    ChainRpcEndpoints, LibSqlSealedGrantStore, LibSqlSigningLedger, MultiChainBroadcaster,
};
use ironclaw_chain_signing::SecretsKeyStore;
use ironclaw_reborn_composition::{LibSqlAttestedComposition, RebornAttestedComposition};
use ironclaw_secrets::SecretsCrypto;

#[tokio::test]
async fn durable_libsql_attested_composition_assembles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("attested.db"))
            .build()
            .await
            .expect("build libsql db"),
    );

    let grants = Arc::new(LibSqlSealedGrantStore::new(Arc::clone(&db)));
    grants.run_migrations().await.expect("grant migrations");
    let ledger = Arc::new(LibSqlSigningLedger::new(Arc::clone(&db)));
    ledger.run_migrations().await.expect("ledger migrations");

    // Real per-chain broadcaster, endpoints from "config" (none wired in this
    // test; broadcast would fail closed, which is the safe default).
    let broadcaster = Arc::new(
        MultiChainBroadcaster::from_endpoints(ChainRpcEndpoints {
            evm: Some("https://rpc.invalid/evm".to_string()),
            solana: None,
            near: None,
        })
        .expect("broadcaster"),
    );

    let crypto = SecretsCrypto::new(secrecy::SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .expect("master key");
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let ship_gate = CustodialMainnetShipGate::from_env().build_chain_ship_gate(None);
    let bindings: Arc<dyn AttestedGateBindingStore> =
        Arc::new(InMemoryAttestedGateBindingStore::new());

    let composition: LibSqlAttestedComposition = RebornAttestedComposition::assemble(
        bindings,
        keystore,
        ship_gate,
        grants,
        ledger,
        broadcaster,
        ProviderRegistry::new(),
    );

    // The driver is wired and ready to dispatch a resolved attested gate.
    let _driver = composition.driver();
}
