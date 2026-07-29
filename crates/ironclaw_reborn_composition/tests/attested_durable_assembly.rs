//! Gap-2 regression (attested-signing PR13): the production durable assembly
//! seam (`assemble_libsql`) builds the durable `LibSqlAttestedComposition` from
//! a DB handle + RPC endpoints + provider config — the same backend-selection
//! shape the production runtime slice will call. Proves the durable backend
//! assembles cleanly, runs its migrations, and registers the configured
//! providers (so the durable path is not `ProviderMismatch` for a configured
//! provider).
//!
//! Per CLAUDE.md "Test Through the Caller", this drives `assemble_libsql` (the
//! production builder seam) and then the assembled `driver()`, not the
//! lower-level store constructors in isolation.
//!
//! The durable assembly builds its own **durable** gate-binding store from the
//! DB handle; it no longer accepts an arbitrary `Arc<dyn
//! AttestedGateBindingStore>`, so an `InMemoryAttestedGateBindingStore` can no
//! longer be threaded into the durable path (that misconfiguration is now a
//! compile error rather than a silent restart-losing binding).

#![cfg(all(feature = "libsql", feature = "attested-broadcast"))]

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Address, Bytes, TxKind, U256};

use ironclaw_attestation::RenderingSchemaVersion;
use ironclaw_attested_runtime::{AttestedGateBinding, ContinuationError};
use ironclaw_attested_store::ChainRpcEndpoints;
use ironclaw_chain_signing::{ChainKeyId, SecretsKeyStore};
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_reborn_composition::{
    AttestedProvidersConfig, DurableCustody, NearRedirectConfig, assemble_libsql,
};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, SigningProof, TenantId as SigningTenantId, UserId as SigningUserId,
};
use ironclaw_wallet_external::{
    NearAccessKeyScope, NearRedirectProofPayload, encode_near_redirect_proof,
};
use secrecy::SecretString;

const GATE: &str = "gate:pr13-durable";
const TENANT: &str = "tenant1";
const USER: &str = "user1";

fn keystore() -> Arc<SecretsKeyStore> {
    let crypto = SecretsCrypto::new(SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .expect("valid master key");
    Arc::new(SecretsKeyStore::new(crypto))
}

/// A self-consistent NEAR `Transfer` decoded tx on `near:mainnet` whose
/// `signer_id` equals `signer`. Used so the validating durable binding store
/// accepts the binding (its `validate_binding` recomputes the approved hash
/// from this decoded tx + the bound signer and checks chain == decoded network
/// and, for NEAR, decoded `signer_id` == bound account).
fn near_transfer_decoded(signer: &str) -> ironclaw_attestation::DecodedTransaction {
    use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey, NearTransaction};

    ironclaw_chain_signing::near::decode::decode_projected(NearTransaction {
        network: "mainnet".to_string(),
        signer_id: signer.to_string(),
        public_key: NearPublicKey {
            key_type: 0,
            data: vec![0u8; 32],
        },
        receiver_id: "bob.near".to_string(),
        nonce: 1,
        block_hash: Bytes32([0x22u8; 32]),
        actions: vec![NearAction::Transfer {
            deposit: vec![0x01],
        }],
    })
    .expect("project near transfer tx in test")
}

/// A self-consistent EVM (eip155:11155111) decoded tx + the matching approved
/// hash recomputed from the bound signer (`signer`, lowercase hex, no `0x`).
/// EVM recovers the signer at sign time, so `validate_binding` only checks the
/// hash and the chain network for the EVM path.
fn evm_decoded_with_hash(
    signer: &str,
) -> (ironclaw_attestation::DecodedTransaction, ApprovedTxHash) {
    let decoded = ironclaw_chain_signing::evm::decode_eip1559(&TxEip1559 {
        chain_id: 11155111,
        nonce: 1,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0x11)),
        value: U256::from(5u64),
        input: Bytes::new(),
        access_list: Default::default(),
    });
    let hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        signer,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    (decoded, hash)
}

#[tokio::test]
async fn durable_libsql_assembles_and_drives() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("attested.db"))
            .build()
            .await
            .expect("build libsql db"),
    );

    // NEAR configured; RPC endpoints unset for NEAR (broadcast would fail
    // closed, which is the safe default — the test stops at proof verification).
    // The state_secret is a >=32-byte, high-entropy key (the validated config
    // rejects short / placeholder / low-entropy secrets).
    let providers = AttestedProvidersConfig {
        near_redirect: Some(
            NearRedirectConfig::new(
                "https://wallet.testnet.near.org/sign",
                "https://app.example/near/callback",
                "f3K9pLm2QzR7vWx1Yb4Nc8Hd6Ts0Ug5Ej2Aq",
            )
            .expect("valid near config"),
        ),
        walletconnect: None,
    };

    let composition = assemble_libsql(
        Arc::clone(&db),
        DurableCustody::from_keystore(keystore()),
        ChainRpcEndpoints::default(),
        providers,
    )
    .await
    .expect("durable libsql composition assembles");

    // Register a NEAR gate over the durable stores, then drive the durable
    // driver with a bogus proof: the configured NEAR provider is registered, so
    // the failure is NOT ProviderMismatch.
    //
    // The binding is self-consistent (the validating durable `put` recomputes
    // the approved hash from the decoded tx + bound signer and checks the chain
    // matches the decoded tx's network): a NEAR decoded tx whose `signer_id`
    // equals the bound account, on `near:mainnet`, with the hash recomputed via
    // the same `recompute_approved_hash` the production raise side uses.
    let account = "alice.near";
    let decoded = near_transfer_decoded(account);
    let hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        account,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    let gate_ref = SigningGateRef::new(GATE);
    composition
        .register_attested_gate(
            gate_ref.clone(),
            AttestedGateBinding {
                provider_id: ProviderId::NearRedirect,
                context: SigningContext {
                    tenant: SigningTenantId::new(TENANT),
                    user: SigningUserId::new(USER),
                    scope: ScopeId::new("scope"),
                    actor: ActorId::new("actor"),
                    run_id: RunId::new("run"),
                    gate_ref: gate_ref.clone(),
                    chain_id: ChainId::new("near:mainnet"),
                    key_or_account_id: KeyOrAccountId::new(account),
                },
                approved_tx_hash: hash,
                decoded,
                chain: ChainKeyId::new("near:mainnet").expect("valid chain id in test"),
                scope: ResourceScope {
                    tenant_id: TenantId::new(TENANT).unwrap(),
                    user_id: UserId::new(USER).unwrap(),
                    agent_id: Some(AgentId::new("agent1").unwrap()),
                    project_id: Some(ProjectId::new("project1").unwrap()),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                },
                schema_version: RenderingSchemaVersion::CURRENT,
            },
            0,
            None,
        )
        .await
        .expect("register attested gate on durable stores");

    let proof = SigningProof::NearRedirectProof(
        encode_near_redirect_proof(&NearRedirectProofPayload {
            approved_tx_hash: hash,
            account_id: account.to_string(),
            public_key: vec![0u8; 32],
            signature: vec![0u8; 64],
            access_key_scope: NearAccessKeyScope::FullAccess,
            state: "bogus".to_string(),
        })
        .expect("encode near redirect proof in test"),
    );

    let err = composition
        .driver()
        .continue_after_resolved(&gate_ref, &proof)
        .await
        .expect_err("bogus proof rejected");
    assert!(
        !matches!(err, ContinuationError::ProviderMismatch { .. }),
        "configured NEAR provider must be registered on the durable path; got {err:?}"
    );
}

/// The durable assembly builds a durable, restart-surviving binding store: a
/// binding registered through one assembly is visible to a fresh assembly over
/// the SAME db file (the in-memory store would lose it). This is the regression
/// guard for "durable assembly accepts an in-memory binding store".
#[tokio::test]
async fn durable_libsql_binding_survives_reassembly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("attested.db");

    let providers = AttestedProvidersConfig::default();
    // EVM injected-wallet binding: signer is a lowercase-hex address (no `0x`),
    // and the binding is self-consistent so the validating durable `put`
    // accepts it (hash recomputed from the decoded tx + bound signer; chain ==
    // the decoded tx's network).
    let account = "00000000000000000000000000000000000000bb";
    let (decoded, hash) = evm_decoded_with_hash(account);
    let gate_ref = SigningGateRef::new(GATE);

    let binding = AttestedGateBinding {
        provider_id: ProviderId::Injected,
        context: SigningContext {
            tenant: SigningTenantId::new(TENANT),
            user: SigningUserId::new(USER),
            scope: ScopeId::new("scope"),
            actor: ActorId::new("actor"),
            run_id: RunId::new("run"),
            gate_ref: gate_ref.clone(),
            chain_id: ChainId::new("eip155:11155111"),
            key_or_account_id: KeyOrAccountId::new(account),
        },
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new("eip155:11155111").expect("valid chain id in test"),
        scope: ResourceScope {
            tenant_id: TenantId::new(TENANT).unwrap(),
            user_id: UserId::new(USER).unwrap(),
            agent_id: Some(AgentId::new("agent1").unwrap()),
            project_id: Some(ProjectId::new("project1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        schema_version: RenderingSchemaVersion::CURRENT,
    };

    {
        let db = Arc::new(
            libsql::Builder::new_local(&db_path)
                .build()
                .await
                .expect("build libsql db"),
        );
        let composition = assemble_libsql(
            Arc::clone(&db),
            DurableCustody::from_keystore(keystore()),
            ChainRpcEndpoints::default(),
            providers.clone(),
        )
        .await
        .expect("first assembly");
        composition
            .register_attested_gate(gate_ref.clone(), binding.clone(), 0, None)
            .await
            .expect("persist binding durably");
    }

    // Fresh assembly over the SAME db file: a durable store re-hydrates the
    // binding from the table; an in-memory store would return None.
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .expect("rebuild libsql db"),
    );
    let composition = assemble_libsql(
        Arc::clone(&db),
        DurableCustody::from_keystore(keystore()),
        ChainRpcEndpoints::default(),
        providers,
    )
    .await
    .expect("second assembly");
    let recovered = composition.bindings().get(&gate_ref).await;
    assert!(
        recovered.is_some(),
        "durable binding must survive a process restart / re-assembly"
    );
}

/// A present-but-malformed RPC endpoint URL is rejected at assembly time (it
/// would otherwise be stored verbatim and only fail on first broadcast).
#[tokio::test]
async fn durable_libsql_rejects_malformed_rpc_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("attested.db"))
            .build()
            .await
            .expect("build libsql db"),
    );
    let endpoints = ChainRpcEndpoints {
        evm: Some("not a url".to_string()),
        solana: None,
        near: None,
    };
    let result = assemble_libsql(
        Arc::clone(&db),
        DurableCustody::from_keystore(keystore()),
        endpoints,
        AttestedProvidersConfig::default(),
    )
    .await;
    // A startup misconfiguration is surfaced as `Config` (not `Broadcast`) so an
    // operator can distinguish boot-time misconfig from a runtime RPC outage.
    assert!(
        matches!(result, Err(ContinuationError::Config { .. })),
        "malformed RPC URL must be rejected at assembly time as a Config error"
    );
}

/// A present RPC endpoint pointing at the cloud-metadata host is rejected
/// (SSRF / credential-exfil hardening).
#[tokio::test]
async fn durable_libsql_rejects_internal_metadata_rpc_url() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("attested.db"))
            .build()
            .await
            .expect("build libsql db"),
    );
    let endpoints = ChainRpcEndpoints {
        evm: Some("http://169.254.169.254/latest/meta-data".to_string()),
        solana: None,
        near: None,
    };
    let result = assemble_libsql(
        Arc::clone(&db),
        DurableCustody::from_keystore(keystore()),
        endpoints,
        AttestedProvidersConfig::default(),
    )
    .await;
    assert!(
        matches!(result, Err(ContinuationError::Config { .. })),
        "metadata-host RPC URL must be rejected as a Config error"
    );
}

/// Env-gated Postgres durable-assembly smoke test: builds the durable PG
/// composition (running grant/ledger/binding migrations) against the PG test
/// URL. Skips when `IRONCLAW_TEST_POSTGRES_URL` is unset so CI without a PG
/// instance stays green.
#[cfg(feature = "postgres")]
#[tokio::test]
async fn durable_postgres_assembles_and_runs_migrations() {
    let Ok(pg_url) = std::env::var("IRONCLAW_TEST_POSTGRES_URL") else {
        eprintln!("skipping: IRONCLAW_TEST_POSTGRES_URL unset");
        return;
    };

    let mut cfg = deadpool_postgres::Config::new();
    cfg.url = Some(pg_url);
    let pool = cfg
        .create_pool(
            Some(deadpool_postgres::Runtime::Tokio1),
            tokio_postgres::NoTls,
        )
        .expect("create pg pool");

    let composition = ironclaw_reborn_composition::assemble_postgres(
        pool,
        DurableCustody::from_keystore(keystore()),
        ChainRpcEndpoints::default(),
        AttestedProvidersConfig::default(),
    )
    .await
    .expect("durable postgres composition assembles + migrates");

    // Migrations ran: a binding write/read round-trips through the durable
    // store rather than erroring on a missing table. The binding is
    // self-consistent so the validating durable `put` accepts it.
    let account = "00000000000000000000000000000000000000bb";
    let (decoded, hash) = evm_decoded_with_hash(account);
    let gate_ref = SigningGateRef::new("gate:pr13-pg");
    let binding = AttestedGateBinding {
        provider_id: ProviderId::Injected,
        context: SigningContext {
            tenant: SigningTenantId::new(TENANT),
            user: SigningUserId::new(USER),
            scope: ScopeId::new("scope"),
            actor: ActorId::new("actor"),
            run_id: RunId::new("run"),
            gate_ref: gate_ref.clone(),
            chain_id: ChainId::new("eip155:11155111"),
            key_or_account_id: KeyOrAccountId::new(account),
        },
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new("eip155:11155111").expect("valid chain id in test"),
        scope: ResourceScope {
            tenant_id: TenantId::new(TENANT).unwrap(),
            user_id: UserId::new(USER).unwrap(),
            agent_id: Some(AgentId::new("agent1").unwrap()),
            project_id: Some(ProjectId::new("project1").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    composition
        .register_attested_gate(gate_ref.clone(), binding, 0, None)
        .await
        .expect("binding write round-trips (migrations present)");
    assert!(composition.bindings().get(&gate_ref).await.is_some());
}
