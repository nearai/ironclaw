//! Adversarial integration tests driving the [`CustodialSigner`] call site
//! (not just the helpers): both enforcement points, broadcast idempotency,
//! exact-chain binding (incl. same-family cross-chain), the KMS-vs-hot-key
//! ship-gate path, approve-A/sign-B drift, and untrusted-metadata policy.

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Bytes, TxKind, U256};

use ironclaw_attestation::{
    AttestedSigningGrant, DecodedTransaction, GrantKey, InMemorySealedGrantStore,
    InMemorySigningLedger, RenderingSchemaVersion, SealedGrantStore, SigningLedger,
    SigningLedgerState,
};
use ironclaw_chain_signing::{
    ChainKeyBinding, ChainKeyId, ChainSigningError, CustodialSignRequest, CustodialSigner,
    DenyFirstCustodyPolicy, KeyStore, LocalKmsSigner, SecretsKeyStore, ShipGate, SignatureAlg, evm,
    recompute_approved_hash,
};
use ironclaw_host_api::{
    InvocationId, ProjectId, ResourceScope, TenantId as HostTenantId, UserId as HostUserId,
};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{
    ActorId, ChainId, GateRef, KeyOrAccountId, RunId, ScopeId, SigningContext, TenantId, UserId,
};
use k256::ecdsa::SigningKey;
use secrecy::SecretString;

const SCHEMA: RenderingSchemaVersion = RenderingSchemaVersion::CURRENT;
const TESTNET_CHAIN: &str = "eip155:11155111"; // sepolia: hot-key allowed
const MAINNET_CHAIN: &str = "eip155:1";

fn crypto() -> SecretsCrypto {
    SecretsCrypto::new(SecretString::from(
        "0123456789abcdef0123456789ABCDEF".to_string(),
    ))
    .unwrap()
}

fn host_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: HostTenantId::new("default").unwrap(),
        user_id: HostUserId::new("alice").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("bootstrap").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn ctx(chain: &str) -> SigningContext {
    SigningContext {
        tenant: TenantId::new("default"),
        user: UserId::new("alice"),
        scope: ScopeId::new("scope-x"),
        actor: ActorId::new("actor-1"),
        run_id: RunId::new("run-1"),
        gate_ref: GateRef::new("gate:tx-1"),
        chain_id: ChainId::new(chain),
        key_or_account_id: KeyOrAccountId::new("custodial"),
    }
}

/// An EIP-1559 sample tx for `chain_id`.
fn sample_tx(chain_id: u64) -> TxEip1559 {
    TxEip1559 {
        chain_id,
        nonce: 3,
        gas_limit: 21000,
        max_fee_per_gas: 100,
        max_priority_fee_per_gas: 2,
        to: TxKind::Call(alloy_primitives::address!(
            "00000000000000000000000000000000000000aa"
        )),
        value: U256::from(1000u64),
        access_list: Default::default(),
        input: Bytes::new(),
    }
}

fn signing_key() -> SigningKey {
    SigningKey::from_slice(&[0x11u8; 32]).unwrap()
}

fn binding(chain: &str, addr_hex: String, kms_key_ref: Option<String>) -> ChainKeyBinding {
    ChainKeyBinding {
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        public_address_hex: addr_hex,
        evm_chain_id: chain.strip_prefix("eip155:").and_then(|s| s.parse().ok()),
        derivation_path: "m/44'/60'/0'/0/0".into(),
        kms_key_ref,
    }
}

/// Build a fully-wired hot-key signer plus a bound key and (optionally) a sealed
/// grant for the happy path.
struct Fixture {
    signer: CustodialSigner<SecretsKeyStore, InMemorySealedGrantStore, InMemorySigningLedger>,
    grants: Arc<InMemorySealedGrantStore>,
    ledger: Arc<InMemorySigningLedger>,
    req: CustodialSignRequest,
}

async fn fixture(seal_grant: bool, mutate_after_approval: bool) -> Fixture {
    let chain = TESTNET_CHAIN;
    let tx = sample_tx(11155111);
    let key = signing_key();
    let bound = evm::address_of(&key);
    let bound_hex = hex::encode(bound.as_slice());

    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(chain, bound_hex, None),
            key.to_bytes().to_vec(),
        )
        .await
        .unwrap();

    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();

    let persisted = if mutate_after_approval {
        let mut d = decoded.clone();
        if let DecodedTransaction::Evm(evm_tx) = &mut d {
            evm_tx.value = vec![0xff, 0xff]; // change the value post-approval
        }
        d
    } else {
        decoded
    };

    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);

    ledger.create(&context.gate_ref).await.unwrap();

    if seal_grant {
        let gk = GrantKey::from_context(&context, approved);
        grants
            .seal(AttestedSigningGrant::seal(gk, 0, None))
            .await
            .unwrap();
    }

    let signer = CustodialSigner::new(
        Arc::clone(&keystore),
        Arc::clone(&grants),
        Arc::clone(&ledger),
        ShipGate::new(false, None), // testnet: hot key allowed
        Arc::new(DenyFirstCustodyPolicy),
    );

    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded: persisted,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };

    Fixture {
        signer,
        grants,
        ledger,
        req,
    }
}

#[tokio::test]
async fn happy_path_signs_and_advances_ledger() {
    let f = fixture(true, false).await;
    let out = f.signer.sign_evm(&f.req).await.expect("sign");
    assert!(out.signer.starts_with("0x"));
    assert!(!out.signature.is_empty());
    assert_eq!(
        f.ledger.state(&f.req.context.gate_ref).await.unwrap(),
        SigningLedgerState::Signed
    );
}

#[tokio::test]
async fn refuses_without_a_claimed_grant() {
    let f = fixture(false, false).await;
    let err = f.signer.sign_evm(&f.req).await.unwrap_err();
    assert!(matches!(err, ChainSigningError::Grant(_)), "got {err:?}");
    assert_eq!(
        f.ledger.state(&f.req.context.gate_ref).await.unwrap(),
        SigningLedgerState::Approved
    );
}

#[tokio::test]
async fn second_signing_of_same_grant_is_refused_one_shot() {
    let f = fixture(true, false).await;
    f.signer.sign_evm(&f.req).await.expect("first sign");
    let err = f
        .grants
        .claim(&GrantKey::from_context(
            &f.req.context,
            f.req.approved_tx_hash,
        ))
        .await
        .unwrap_err();
    assert_eq!(err, ironclaw_attestation::GrantError::AlreadyClaimed);
}

#[tokio::test]
async fn sign_time_hash_recheck_fails_closed_without_consuming_key() {
    // Persisted decoded tx mutated after approval => recomputed hash diverges.
    let f = fixture(true, true).await;
    let err = f.signer.sign_evm(&f.req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::ApprovedHashMismatch),
        "expected ApprovedHashMismatch, got {err:?}"
    );
    assert_eq!(
        f.ledger.state(&f.req.context.gate_ref).await.unwrap(),
        SigningLedgerState::Approved
    );
}

/// Review finding #1: the signer reconstructs the signable tx from `req.decoded`
/// (the same decoded tx the approved hash was computed over). There is NO
/// caller-supplied "tx B" to sign — so the only way to make the signer sign
/// different bytes than were approved is to mutate `decoded`, which the hash
/// re-check catches (above). This test pins the property that the produced
/// signature recovers a signer over the digest rebuilt from `decoded`, i.e. the
/// approved bytes — proven by the happy path producing a valid bound signature
/// while the mutated-decoded case fails closed.
#[tokio::test]
async fn signs_exactly_the_decoded_tx_not_a_separate_payload() {
    // Approve decoded-A; then present decoded-B (different value) WITHOUT a
    // grant for B. Because the signer derives everything from `decoded`, the
    // grant for A cannot authorize B (the GrantKey binds the approved hash of
    // B, which has no sealed grant), and signing is refused.
    let f = fixture(true, false).await;
    let tx_b = TxEip1559 {
        value: U256::from(999_999u64),
        ..sample_tx(11155111)
    };
    let decoded_b = evm::decode_eip1559(&tx_b);
    let approved_b = recompute_approved_hash(&decoded_b, "custodial", SCHEMA).unwrap();
    let req_b = CustodialSignRequest {
        context: ctx(TESTNET_CHAIN),
        scope: host_scope(),
        chain: ChainKeyId::new(TESTNET_CHAIN).expect("valid chain id in test"),
        decoded: decoded_b,
        approved_tx_hash: approved_b, // hash of B, but no grant sealed for B
        schema_version: SCHEMA,
    };
    let err = f.signer.sign_evm(&req_b).await.unwrap_err();
    assert!(matches!(err, ChainSigningError::Grant(_)), "got {err:?}");
}

#[tokio::test]
async fn evm_signer_binding_rejects_wrong_bound_account() {
    let chain = TESTNET_CHAIN;
    let tx = sample_tx(11155111);
    let key = signing_key();
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            // Wrong bound address (all 0xbb) — does not match the key.
            binding(chain, "bb".repeat(20), None),
            key.to_bytes().to_vec(),
        )
        .await
        .unwrap();

    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(false, None),
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };

    let err = signer.sign_evm(&req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::SignerMismatch),
        "got {err:?}"
    );
}

#[tokio::test]
async fn broadcast_idempotency_blocks_resigning_after_submitted() {
    let f = fixture(true, false).await;
    f.signer.sign_evm(&f.req).await.expect("sign");
    f.signer
        .mark_broadcast_submitted(&f.req.context)
        .await
        .expect("broadcast submitted");

    let err = f
        .ledger
        .advance(&f.req.context.gate_ref, SigningLedgerState::Signing)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        ironclaw_attestation::LedgerError::InvalidTransition {
            from: SigningLedgerState::BroadcastSubmitted,
            to: SigningLedgerState::Signing,
        }
    );

    f.signer
        .finalize(&f.req.context, SigningLedgerState::Finalized)
        .await
        .expect("finalize");
}

#[tokio::test]
async fn wrong_chain_family_key_cannot_sign_other_chain_tx() {
    // Key bound to a Solana chain id; present an EVM tx for signing.
    let solana_chain = "solana:devnet";
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(solana_chain, "00".repeat(32), None),
            vec![5u8; 32],
        )
        .await
        .unwrap();

    let tx = sample_tx(11155111);
    let decoded = evm::decode_eip1559(&tx); // EVM tx
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let mut context = ctx(solana_chain);
    context.gate_ref = GateRef::new("gate:confused");
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(false, None),
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(solana_chain).expect("valid chain id in test"), // Solana-bound key
        decoded,                                                               // EVM tx
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };

    let err = signer.sign_evm(&req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::ChainMismatch { .. }),
        "got {err:?}"
    );
}

/// Review finding #2: SAME-FAMILY cross-chain. An `eip155:11155111` (sepolia)
/// key/context must NOT sign an `eip155:1` (mainnet) tx — family is identical,
/// only the full chain id differs. Exact-equality binding rejects it.
#[tokio::test]
async fn same_family_cross_chain_id_is_rejected() {
    let key_chain = TESTNET_CHAIN; // eip155:11155111
    let key = signing_key();
    let bound = evm::address_of(&key);
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(key_chain, hex::encode(bound.as_slice()), None),
            key.to_bytes().to_vec(),
        )
        .await
        .unwrap();

    // The decoded tx and context are for mainnet (eip155:1), but the key is
    // bound to sepolia.
    let tx = sample_tx(1);
    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(key_chain); // context says sepolia, tx says mainnet
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(false, None),
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(key_chain).expect("valid chain id in test"),
        decoded,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };

    let err = signer.sign_evm(&req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::ChainMismatch { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn ship_gate_refuses_mainnet_hot_key() {
    let chain = MAINNET_CHAIN;
    let tx = sample_tx(1);
    let key = signing_key();
    let bound = evm::address_of(&key);
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(chain, hex::encode(bound.as_slice()), None),
            key.to_bytes().to_vec(),
        )
        .await
        .unwrap();
    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(true, None), // opt-in but no KMS
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };
    let err = signer.sign_evm(&req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::ShipGateRefused { .. }),
        "got {err:?}"
    );
}

/// Review finding #3: even with a secure KMS *configured*, mainnet hot-key
/// signing must be refused — the mainnet path is KMS-only. We prove it by wiring
/// a secure KMS but binding the key WITHOUT a `kms_key_ref` (so it could only be
/// signed hot): the signer routes mainnet to the KMS path and fails closed for
/// the missing key_ref rather than falling back to the hot key.
#[tokio::test]
async fn secure_kms_configured_but_mainnet_hot_key_is_refused() {
    let chain = MAINNET_CHAIN;
    let tx = sample_tx(1);
    let key = signing_key();
    let bound = evm::address_of(&key);
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    // Bound as a hot key (no kms_key_ref) even though a KMS is wired.
    keystore
        .bind(
            &host_scope(),
            binding(chain, hex::encode(bound.as_slice()), None),
            key.to_bytes().to_vec(),
        )
        .await
        .unwrap();
    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let kms: Arc<dyn ironclaw_chain_signing::KmsSigner> =
        Arc::new(LocalKmsSigner::new("secure-kms"));
    let signer = CustodialSigner::with_kms(
        keystore,
        grants,
        ledger,
        ShipGate::new(true, Some(kms.as_ref())),
        kms,
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };
    let err = signer.sign_evm(&req).await.unwrap_err();
    assert!(
        matches!(err, ChainSigningError::KeyStore { .. }),
        "mainnet without a KMS key_ref must fail closed, got {err:?}"
    );
}

/// Review finding #3 (positive): mainnet signing SUCCEEDS through the KMS
/// key-ref path with NO private-key bytes in the IronClaw process — the key
/// lives in the `LocalKmsSigner` reference backend; only a key_ref + digest
/// cross the boundary.
#[tokio::test]
async fn mainnet_signs_via_kms_key_ref_path() {
    let chain = MAINNET_CHAIN;
    let tx = sample_tx(1);
    let key = signing_key();
    let bound = evm::address_of(&key);

    // The KMS holds the key behind its sealed boundary, referenced by "kms-evm".
    let kms_backend = LocalKmsSigner::new("secure-kms");
    kms_backend
        .import_key("kms-evm", SignatureAlg::Secp256k1, key.to_bytes().to_vec())
        .unwrap();
    let kms: Arc<dyn ironclaw_chain_signing::KmsSigner> = Arc::new(kms_backend);

    // The keystore holds NO usable key for this chain — only the public binding
    // plus the KMS key_ref. (We bind dummy bytes the signer must never use on
    // the KMS path.)
    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(
                chain,
                hex::encode(bound.as_slice()),
                Some("kms-evm".to_string()),
            ),
            vec![0u8; 32], // never consumed on the KMS path
        )
        .await
        .unwrap();

    let decoded = evm::decode_eip1559(&tx);
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();

    let signer = CustodialSigner::with_kms(
        Arc::clone(&keystore),
        grants,
        Arc::clone(&ledger),
        ShipGate::new(true, Some(kms.as_ref())),
        kms,
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded,
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };
    let out = signer.sign_evm(&req).await.expect("kms sign");
    // The recovered signer equals the bound account — the KMS signature is
    // ecrecover-bound exactly like the hot path.
    assert_eq!(out.signer, format!("0x{}", hex::encode(bound.as_slice())));
    assert_eq!(
        ledger.state(&req.context.gate_ref).await.unwrap(),
        SigningLedgerState::Signed
    );
}

/// Review finding #4 (Solana): the custodial Solana sign produces an ed25519
/// signature that verifies against sha256 of PR2's `canonical_signing_bytes`
/// for the SAME decoded tx — proving the signed bytes are the approved bytes.
#[tokio::test]
async fn solana_signs_over_canonical_bytes() {
    use ed25519_dalek::{Signature, SigningKey as EdKey, Verifier, VerifyingKey};
    use ironclaw_attestation::{
        Bytes32, SolanaCompiledInstruction, SolanaMessageHeader, SolanaMessageVersion,
        SolanaTransaction, canonical_signing_bytes,
    };

    let chain = "solana:devnet";
    let ed = EdKey::from_bytes(&[0x42u8; 32]);
    let pubkey = ed.verifying_key().to_bytes();

    let sol = SolanaTransaction {
        cluster: "devnet".into(),
        version: SolanaMessageVersion::Legacy,
        header: SolanaMessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        // Index 0 is the fee payer (the bound signer); index 1 is the program.
        static_account_keys: vec![Bytes32(pubkey), Bytes32([9u8; 32])],
        recent_blockhash: Bytes32([2u8; 32]),
        instructions: vec![SolanaCompiledInstruction {
            program_id_index: 1,
            account_indices: vec![0],
            data: vec![1, 2, 3],
        }],
        address_table_lookups: vec![],
    };
    let decoded = DecodedTransaction::Solana(sol.clone());
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();

    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(chain, hex::encode(pubkey), None),
            ed.to_bytes().to_vec(),
        )
        .await
        .unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();
    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(false, None),
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded: decoded.clone(),
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };
    let out = signer.sign_solana(&req).await.expect("sign");

    // Byte-equality property: the signature verifies against sha256 of PR2's
    // canonical bytes for the same decoded tx.
    let canonical = canonical_signing_bytes(&decoded, SCHEMA).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&canonical);
        let d: [u8; 32] = h.finalize().into();
        d
    };
    let vk = VerifyingKey::from_bytes(&pubkey).unwrap();
    let sig = Signature::from_slice(&out.signature).unwrap();
    vk.verify(&digest, &sig)
        .expect("signature must verify over sha256(canonical_signing_bytes)");
}

/// Review finding #4 (NEAR): same property for NEAR.
#[tokio::test]
async fn near_signs_over_canonical_bytes() {
    use ed25519_dalek::{Signature, SigningKey as EdKey, Verifier, VerifyingKey};
    use ironclaw_attestation::{
        Bytes32, NearAction, NearPublicKey, NearTransaction, canonical_signing_bytes,
    };

    let chain = "near:testnet";
    let ed = EdKey::from_bytes(&[0x55u8; 32]);
    let pubkey = ed.verifying_key().to_bytes();

    let near = NearTransaction {
        network: "testnet".into(),
        signer_id: "alice.testnet".into(),
        public_key: NearPublicKey {
            key_type: 0,
            data: pubkey.to_vec(),
        },
        receiver_id: "bob.testnet".into(),
        nonce: 11,
        block_hash: Bytes32([3u8; 32]),
        actions: vec![NearAction::Transfer {
            deposit: vec![1, 2],
        }],
    };
    let decoded = DecodedTransaction::Near(near.clone());
    let approved = recompute_approved_hash(&decoded, "custodial", SCHEMA).unwrap();

    let keystore = Arc::new(SecretsKeyStore::new(crypto()));
    keystore
        .bind(
            &host_scope(),
            binding(chain, hex::encode(pubkey), None),
            ed.to_bytes().to_vec(),
        )
        .await
        .unwrap();
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ledger = Arc::new(InMemorySigningLedger::new());
    let context = ctx(chain);
    ledger.create(&context.gate_ref).await.unwrap();
    grants
        .seal(AttestedSigningGrant::seal(
            GrantKey::from_context(&context, approved),
            0,
            None,
        ))
        .await
        .unwrap();
    let signer = CustodialSigner::new(
        keystore,
        grants,
        ledger,
        ShipGate::new(false, None),
        Arc::new(DenyFirstCustodyPolicy),
    );
    let req = CustodialSignRequest {
        context,
        scope: host_scope(),
        chain: ChainKeyId::new(chain).expect("valid chain id in test"),
        decoded: decoded.clone(),
        approved_tx_hash: approved,
        schema_version: SCHEMA,
    };
    let out = signer.sign_near(&req).await.expect("sign");

    let canonical = canonical_signing_bytes(&decoded, SCHEMA).unwrap();
    let digest = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(&canonical);
        let d: [u8; 32] = h.finalize().into();
        d
    };
    let vk = VerifyingKey::from_bytes(&pubkey).unwrap();
    let sig = Signature::from_slice(&out.signature).unwrap();
    vk.verify(&digest, &sig)
        .expect("signature must verify over sha256(canonical_signing_bytes)");
}

#[test]
fn untrusted_metadata_rejected_by_policy() {
    let tx = sample_tx(11155111);
    let decoded = evm::decode_eip1559(&tx);
    let DecodedTransaction::Evm(evm_tx) = &decoded else {
        panic!("evm");
    };
    assert!(evm::check_chain_id(evm_tx, 1).is_err());
    assert!(evm::check_chain_id(evm_tx, 11155111).is_ok());
}
