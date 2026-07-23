//! Gap-1 regression (attested-signing PR13): the NEAR-redirect and
//! WalletConnect external-wallet providers, once their ceremony config is
//! present, are REGISTERED in the attested composition's `ProviderRegistry` and
//! reach proof verification through the continuation driver — instead of
//! failing closed as `ProviderMismatch`.
//!
//! Per CLAUDE.md "Test Through the Caller", this drives the assembled
//! `RebornAttestedComposition` (the same `driver()` the WebUI ingress port
//! dispatches through) — not `AttestedProvidersConfig::build_provider_registry`
//! in isolation. The discriminator is the driver's error. An UNregistered
//! provider yields `ContinuationError::ProviderMismatch`; a registered provider
//! lets the proof reach `verify_resume`, where a bad proof surfaces as
//! `ProofRejected` (NOT `ProviderMismatch`).
//!
//! So: with no config the NEAR/WC variants are `ProviderMismatch`; with config
//! present they get past the registry and the SAME bad proof is rejected later.

use std::sync::Arc;

use alloy_consensus::TxEip1559;

use ironclaw_attestation::{DecodedTransaction, RenderingSchemaVersion};
use ironclaw_attested_runtime::{
    AttestedGateBinding, ContinuationError, CustodialMainnetShipGate,
    InMemoryAttestedGateBindingStore,
};
use ironclaw_chain_signing::{ChainKeyId, SecretsKeyStore};
use ironclaw_host_api::{AgentId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_reborn_composition::{AttestedProvidersConfig, LocalDevAttestedComposition};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{
    ActorId, ApprovedTxHash, ChainId, GateRef as SigningGateRef, KeyOrAccountId, ProviderId, RunId,
    ScopeId, SigningContext, SigningProof, TenantId as SigningTenantId, UserId as SigningUserId,
};
use ironclaw_wallet_external::{
    NearAccessKeyScope, NearRedirectProofPayload, ProjectId as WcProjectId,
    WalletConnectProofPayload, encode_near_redirect_proof, encode_walletconnect_proof,
};
use secrecy::SecretString;

const GATE: &str = "gate:pr13-provider-reg";
const TENANT: &str = "tenant1";
const USER: &str = "user1";
const AGENT: &str = "agent1";
const PROJECT: &str = "project1";

fn signing_ctx(chain: &str, account: &str) -> SigningContext {
    SigningContext {
        tenant: SigningTenantId::new(TENANT),
        user: SigningUserId::new(USER),
        scope: ScopeId::new("scope"),
        actor: ActorId::new("actor"),
        run_id: RunId::new("run"),
        gate_ref: SigningGateRef::new(GATE),
        chain_id: ChainId::new(chain),
        key_or_account_id: KeyOrAccountId::new(account),
    }
}

/// A decoded tx self-consistent with `chain` + `account`: a NEAR `Transfer`
/// (signer_id == account, network from `near:<network>`) for `near:*` chains,
/// otherwise an EVM eip1559 tx whose `chain_id` matches the `eip155:<id>` chain.
/// The validating durable/in-memory `put` recomputes the approved hash from this
/// decoded tx + the bound signer and checks the chain matches the decoded
/// network, so the binding must be internally consistent to register.
fn decoded_for(chain: &str, account: &str) -> DecodedTransaction {
    use alloy_primitives::{Address, Bytes, TxKind, U256};

    if let Some(network) = chain.strip_prefix("near:") {
        use ironclaw_attestation::{Bytes32, NearAction, NearPublicKey, NearTransaction};
        return ironclaw_chain_signing::near::decode::decode_projected(NearTransaction {
            network: network.to_string(),
            signer_id: account.to_string(),
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
        .expect("project near transfer tx in test");
    }

    let evm_chain_id: u64 = chain
        .strip_prefix("eip155:")
        .and_then(|id| id.parse().ok())
        .expect("eip155:<id> chain in test");
    ironclaw_chain_signing::evm::decode_eip1559(&TxEip1559 {
        chain_id: evm_chain_id,
        nonce: 1,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0x11)),
        value: U256::from(5u64),
        input: Bytes::new(),
        access_list: Default::default(),
    })
}

/// Build a self-consistent binding for `(provider, chain, account)` and return
/// it alongside the approved hash recomputed from its decoded tx + bound signer
/// — the same hash the matching proof must attest to. Callers fold this hash
/// into both the binding (already done here) and the proof so verification
/// reaches the provider rather than failing the bound-hash pre-check.
fn binding(
    provider_id: ProviderId,
    chain: &str,
    account: &str,
) -> (AttestedGateBinding, ApprovedTxHash) {
    let decoded = decoded_for(chain, account);
    let hash = ironclaw_chain_signing::recompute_approved_hash(
        &decoded,
        account,
        RenderingSchemaVersion::CURRENT,
    )
    .expect("recompute approved hash in test");
    let binding = AttestedGateBinding {
        provider_id,
        context: signing_ctx(chain, account),
        approved_tx_hash: hash,
        decoded,
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        scope: ResourceScope {
            tenant_id: TenantId::new(TENANT).unwrap(),
            user_id: UserId::new(USER).unwrap(),
            agent_id: Some(AgentId::new(AGENT).unwrap()),
            project_id: Some(ProjectId::new(PROJECT).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        schema_version: RenderingSchemaVersion::CURRENT,
    };
    (binding, hash)
}

fn composition(
    bindings: Arc<InMemoryAttestedGateBindingStore>,
    config: AttestedProvidersConfig,
) -> LocalDevAttestedComposition {
    use ironclaw_attestation::InMemorySealedGrantStore;

    let crypto = SecretsCrypto::new(SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .expect("valid local-dev master key");
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let ship_gate = CustodialMainnetShipGate::from_env().build_chain_ship_gate(None);
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let registry = config.build_provider_registry(
        Arc::clone(&grants) as Arc<dyn ironclaw_attestation::SealedGrantStore>
    );
    LocalDevAttestedComposition::new_in_memory(bindings, keystore, ship_gate, grants, registry)
}

/// A deliberately-invalid NEAR proof (empty signature / bogus state). Reaches
/// `verify_resume` only if the provider is registered.
fn bad_near_proof(hash: ApprovedTxHash, account: &str) -> SigningProof {
    SigningProof::NearRedirectProof(
        encode_near_redirect_proof(&NearRedirectProofPayload {
            approved_tx_hash: hash,
            account_id: account.to_string(),
            public_key: vec![0u8; 32],
            signature: vec![0u8; 64],
            access_key_scope: NearAccessKeyScope::FullAccess,
            state: "bogus-state".to_string(),
        })
        .expect("encode near redirect proof in test"),
    )
}

/// A deliberately-invalid WalletConnect proof.
fn bad_wc_proof(hash: ApprovedTxHash, account: &str) -> SigningProof {
    SigningProof::WalletConnectProof(
        encode_walletconnect_proof(&WalletConnectProofPayload {
            session_topic: "topic-bogus".to_string(),
            approved_tx_hash: hash,
            claimed_signer: account.to_string(),
            nonce: vec![0u8; 16],
            signed_payload: vec![0u8; 32],
            signature: vec![0u8; 65],
            public_key: None,
        })
        .expect("encode walletconnect proof in test"),
    )
}

async fn register_and_continue(
    composition: &LocalDevAttestedComposition,
    binding: AttestedGateBinding,
    proof: SigningProof,
) -> Result<(), ContinuationError> {
    let gate_ref = SigningGateRef::new(GATE);
    composition
        .register_attested_gate(gate_ref.clone(), binding, 0, None)
        .await
        .expect("register attested gate");
    composition
        .driver()
        .continue_after_resolved(&gate_ref, &proof)
        .await
        .map(|_| ())
}

#[tokio::test]
async fn near_provider_unregistered_without_config_is_provider_mismatch() {
    let account = "alice.near";
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    // No NEAR config -> provider stays unregistered (fail-closed).
    let comp = composition(Arc::clone(&bindings), AttestedProvidersConfig::default());
    let (gate_binding, hash) = binding(ProviderId::NearRedirect, "near:mainnet", account);
    let err = register_and_continue(&comp, gate_binding, bad_near_proof(hash, account))
        .await
        .expect_err("unregistered NEAR provider must fail closed");
    assert!(
        matches!(err, ContinuationError::ProviderMismatch { bound } if bound == ProviderId::NearRedirect),
        "expected ProviderMismatch, got {err:?}"
    );
}

#[tokio::test]
async fn near_provider_registered_with_config_reaches_verification() {
    let account = "alice.near";
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let config = AttestedProvidersConfig {
        near_redirect: Some(
            ironclaw_reborn_composition::NearRedirectConfig::new(
                "https://wallet.testnet.near.org/sign",
                "https://app.example/near/callback",
                // >=32-byte, high-entropy secret (validated config rejects
                // short / placeholder / low-entropy keys).
                "f3K9pLm2QzR7vWx1Yb4Nc8Hd6Ts0Ug5Ej2Aq",
            )
            .expect("valid near config"),
        ),
        walletconnect: None,
    };
    let comp = composition(Arc::clone(&bindings), config);
    let (gate_binding, hash) = binding(ProviderId::NearRedirect, "near:mainnet", account);
    let err = register_and_continue(&comp, gate_binding, bad_near_proof(hash, account))
        .await
        .expect_err("a bogus proof must still be rejected");
    // The KEY assertion: registered, so NOT ProviderMismatch.
    assert!(
        !matches!(err, ContinuationError::ProviderMismatch { .. }),
        "NEAR provider should be registered with config present; got {err:?}"
    );
}

#[tokio::test]
async fn walletconnect_provider_unregistered_without_config_is_provider_mismatch() {
    let account = "00000000000000000000000000000000000000bb";
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let comp = composition(Arc::clone(&bindings), AttestedProvidersConfig::default());
    let (gate_binding, hash) = binding(ProviderId::WalletConnect, "eip155:1", account);
    let err = register_and_continue(&comp, gate_binding, bad_wc_proof(hash, account))
        .await
        .expect_err("unregistered WalletConnect provider must fail closed");
    assert!(
        matches!(err, ContinuationError::ProviderMismatch { bound } if bound == ProviderId::WalletConnect),
        "expected ProviderMismatch, got {err:?}"
    );
}

#[tokio::test]
async fn walletconnect_provider_registered_with_config_reaches_verification() {
    let account = "00000000000000000000000000000000000000bb";
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let config = AttestedProvidersConfig {
        near_redirect: None,
        walletconnect: Some(
            ironclaw_reborn_composition::WalletConnectConfig::new(
                "00000000000000000000000000000000",
            )
            .expect("valid wc project id"),
        ),
    };
    // Sanity: the project id is a publishable id, constructs cleanly.
    let _ = WcProjectId::from("00000000000000000000000000000000");
    let comp = composition(Arc::clone(&bindings), config);
    let (gate_binding, hash) = binding(ProviderId::WalletConnect, "eip155:1", account);
    let err = register_and_continue(&comp, gate_binding, bad_wc_proof(hash, account))
        .await
        .expect_err("a bogus proof must still be rejected");
    assert!(
        !matches!(err, ContinuationError::ProviderMismatch { .. }),
        "WalletConnect provider should be registered with config present; got {err:?}"
    );
}

/// Drive `register_attested_gate` through the assembled composition (the same
/// caller the PR11 raise side uses) and assert BOTH halves of the raise: the
/// authoritative binding is persisted (readable back through `bindings()`) and
/// the one-shot sealed grant is sealed (a second seal of the same grant key is
/// rejected — proving the CAS slot is occupied, threat #1).
#[tokio::test]
async fn register_attested_gate_seals_grant_and_persists_binding() {
    use ironclaw_attestation::{
        AttestedSigningGrant, GrantKey, InMemorySealedGrantStore, SealedGrantStore,
    };

    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    // Build the composition with a grant store we retain a handle to so we can
    // independently observe that the grant was sealed.
    let crypto = SecretsCrypto::new(SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .expect("valid local-dev master key");
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    let ship_gate = CustodialMainnetShipGate::from_env().build_chain_ship_gate(None);
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let registry = AttestedProvidersConfig::default().build_provider_registry(
        Arc::clone(&grants) as Arc<dyn ironclaw_attestation::SealedGrantStore>
    );
    let composition = LocalDevAttestedComposition::new_in_memory(
        Arc::clone(&bindings),
        keystore,
        ship_gate,
        Arc::clone(&grants),
        registry,
    );

    let gate_ref = SigningGateRef::new(GATE);
    let (gate_binding, _hash) = binding(
        ProviderId::Injected,
        "eip155:1",
        "00000000000000000000000000000000000000bb",
    );
    let grant_key = GrantKey::from_context(&gate_binding.context, gate_binding.approved_tx_hash);

    composition
        .register_attested_gate(gate_ref.clone(), gate_binding, 0, None)
        .await
        .expect("register attested gate");

    // Binding half: the authoritative binding is readable back.
    assert!(
        composition.bindings().get(&gate_ref).await.is_some(),
        "binding must be persisted on raise"
    );

    // Grant half: the one-shot slot is occupied, so re-sealing the same key is
    // rejected (the CAS slot for threat #1 is taken).
    let reseal = grants
        .seal(AttestedSigningGrant::seal(grant_key, 0, None))
        .await;
    assert!(
        reseal.is_err(),
        "the one-shot grant must already be sealed by register_attested_gate"
    );
}

/// Serialize every test that mutates the shared attested-signing env vars so
/// they never race each other (env is process-global).
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const ATTESTED_ENV_VARS: &[&str] = &[
    "ATTESTED_NEAR_WALLET_BASE_URL",
    "ATTESTED_NEAR_CALLBACK_URL",
    "ATTESTED_NEAR_STATE_SECRET",
    "ATTESTED_WALLETCONNECT_PROJECT_ID",
];

/// Clear every attested env var, run `body` with a clean slate, then restore the
/// prior values regardless of panic. Holds [`ENV_LOCK`] for the duration.
fn with_clean_attested_env(body: impl FnOnce()) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
    let saved: Vec<(&str, Option<String>)> = ATTESTED_ENV_VARS
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect();
    // SAFETY: all attested-env mutation in this test binary is serialized
    // through ENV_LOCK, so no other thread reads/writes these vars concurrently.
    unsafe {
        for key in ATTESTED_ENV_VARS {
            std::env::remove_var(key);
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    unsafe {
        for (key, value) in saved {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

/// A 36-char, high-distinct-byte, marker-free `state_secret`.
fn strong_state_secret() -> &'static str {
    "f3K9pLm2QzR7vWx1Yb4Nc8Hd6Ts0Ug5Ej2Aq"
}

/// Partial NEAR config (any strict subset of the three NEAR vars present) is a
/// hard error from `from_env` — a half-configured ceremony fails closed at
/// startup rather than registering a half-wired verifier. This is the
/// build-time caller path: `build_attested_composition` propagates this exact
/// error as `RebornRuntimeError::InvalidArgument`.
#[test]
fn partial_near_config_fails_closed() {
    with_clean_attested_env(|| {
        // Only base URL set.
        unsafe {
            std::env::set_var(
                "ATTESTED_NEAR_WALLET_BASE_URL",
                "https://wallet.near.org/sign",
            );
        }
        assert!(
            AttestedProvidersConfig::from_env().is_err(),
            "base-url-only partial NEAR config must fail closed"
        );

        // Base URL + callback, no state_secret.
        unsafe {
            std::env::set_var("ATTESTED_NEAR_CALLBACK_URL", "https://app.example/cb");
        }
        assert!(
            AttestedProvidersConfig::from_env().is_err(),
            "missing state_secret partial NEAR config must fail closed"
        );

        // All three present + valid -> Ok with the provider configured.
        unsafe {
            std::env::set_var("ATTESTED_NEAR_STATE_SECRET", strong_state_secret());
        }
        let cfg = AttestedProvidersConfig::from_env()
            .expect("complete + valid NEAR config resolves cleanly");
        assert!(cfg.near_redirect.is_some());
    });
}

/// A present-but-invalid attested provider config makes the build path fail
/// (fail-closed at startup), exercised through `from_env` — the same call
/// `build_attested_composition` makes before wrapping the error as
/// `InvalidArgument`.
#[test]
fn build_fails_on_invalid_attested_provider_config() {
    with_clean_attested_env(|| {
        // Malformed WalletConnect project id (not 32 hex chars).
        unsafe {
            std::env::set_var("ATTESTED_WALLETCONNECT_PROJECT_ID", "not-a-valid-id");
        }
        assert!(
            AttestedProvidersConfig::from_env().is_err(),
            "malformed WalletConnect project id must fail the build"
        );
    });

    with_clean_attested_env(|| {
        // Complete NEAR config but a low-entropy placeholder state_secret.
        unsafe {
            std::env::set_var(
                "ATTESTED_NEAR_WALLET_BASE_URL",
                "https://wallet.near.org/sign",
            );
            std::env::set_var("ATTESTED_NEAR_CALLBACK_URL", "https://app.example/cb");
            std::env::set_var(
                "ATTESTED_NEAR_STATE_SECRET",
                "changeme-changeme-changeme-changeme",
            );
        }
        assert!(
            AttestedProvidersConfig::from_env().is_err(),
            "placeholder state_secret must fail the build"
        );
    });
}

/// `from_env` resolves nothing when no attested env vars are set: both
/// providers stay fail-closed.
#[test]
fn from_env_is_fail_closed_when_unset() {
    with_clean_attested_env(|| {
        let config = AttestedProvidersConfig::from_env().expect("unset env resolves cleanly");
        assert!(config.near_redirect.is_none());
        assert!(config.walletconnect.is_none());
    });
}
