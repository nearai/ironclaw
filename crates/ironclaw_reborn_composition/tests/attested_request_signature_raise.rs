//! End-to-end tests for the `request_signature` attested-signing raise path
//! (attested-signing PR14).
//!
//! These drive the REAL composition pieces — the composition-owned
//! [`RebornAttestedRaiseHook`] (the exact `AttestedRaiseHook` trait method
//! `DefaultHostRuntime` calls), the real `RebornAttestedComposition`
//! (`register_attested_gate` → seals the one-shot grant + persists the
//! authoritative binding), the real `ironclaw_chain_signing` custodial signer,
//! and the existing `AttestedSignerContinuationDriver` resolve path — rather
//! than a helper in isolation (CLAUDE.md "Test Through the Caller").
//!
//! Coverage:
//! * custodial `request_signature` → `AttestedSigningRequired` → binding
//!   persisted + grant sealed → the existing resolve path
//!   (`continue_after_resolved`) verifies and continues.
//! * a NEAR / WalletConnect `provider_hint` fails closed (`Failed`, NO gate
//!   raised, NO grant sealed).

use std::sync::Arc;

use alloy_consensus::TxEip1559;
use alloy_primitives::{Address, Bytes, TxKind, U256};

use ironclaw_attestation::{DecodedTransaction, InMemorySealedGrantStore, InMemorySigningLedger};
use ironclaw_attested_runtime::{
    CustodialMainnetShipGate, InMemoryAttestedGateBindingStore, ProviderRegistry,
};
use ironclaw_chain_signing::{ChainKeyBinding, ChainKeyId, KeyStore, SecretsKeyStore, evm};
use ironclaw_host_api::{CapabilityId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_host_runtime::{
    AttestedRaiseHook, AttestedRaiseRequest, RuntimeCapabilityOutcome, RuntimeFailureKind,
};
use ironclaw_reborn_composition::{RebornAttestedComposition, RebornAttestedRaiseHook};
use ironclaw_secrets::SecretsCrypto;
use ironclaw_signing_provider::{GateRef as SigningGateRef, SigningProof};
use serde_json::json;

const DEV_TESTNET_CHAIN: &str = "eip155:11155111"; // sepolia (testnet)

fn owner_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("default").unwrap(),
        user_id: UserId::new("alice").unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("bootstrap").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Build the host execution context the raise hook reads identities from. The
/// scope matches `owner_scope()` so the custodial keystore lookup at resolve
/// time finds the provisioned key.
fn execution_context(scope: ResourceScope) -> ironclaw_host_api::ExecutionContext {
    use ironclaw_host_api::{
        CapabilitySet, CorrelationId, ExecutionContext, ExtensionId, MountView, RuntimeKind,
        TrustClass,
    };
    ExecutionContext {
        invocation_id: scope.invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id: scope.tenant_id.clone(),
        user_id: scope.user_id.clone(),
        agent_id: scope.agent_id.clone(),
        project_id: scope.project_id.clone(),
        mission_id: None,
        thread_id: None,
        extension_id: ExtensionId::new("builtin").unwrap(),
        runtime: RuntimeKind::Wasm,
        trust: TrustClass::UserTrusted,
        grants: CapabilitySet { grants: vec![] },
        mounts: MountView::default(),
        resource_scope: scope,
    }
}

/// A sample EIP-1559 transaction + its SDK-free decoded projection. The raise
/// hook persists the decoded form; resolve re-signs the matching alloy tx.
fn sample_evm() -> (TxEip1559, DecodedTransaction) {
    let tx = TxEip1559 {
        chain_id: 11155111,
        nonce: 7,
        gas_limit: 21_000,
        max_fee_per_gas: 30_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Address::repeat_byte(0xbb)),
        value: U256::from(1_000u64),
        input: Bytes::new(),
        access_list: Default::default(),
    };
    let decoded = evm::decode_eip1559(&tx);
    (tx, decoded)
}

/// Provision an EVM custodial keystore bound to the address derived from
/// `priv_bytes`. Returns the keystore + the lowercase-hex (no `0x`) account.
async fn keystore_with_evm_key(priv_bytes: &[u8; 32]) -> (Arc<SecretsKeyStore>, String) {
    // Per-test random master key (no stable key in source). The in-memory
    // keystore never crosses a process boundary, so a fresh key each run is
    // sufficient and keeps test keying material independent from local-dev.
    let crypto = SecretsCrypto::generate();
    let keystore = Arc::new(SecretsKeyStore::new(crypto));
    // `evm::signing_key_from_bytes` is now crate-private (#4067 raw-key
    // consumption is only reachable inside the guarded custodial flow), so the
    // test derives the public EVM address via k256 directly + the still-public
    // `evm::address_of`. No secret leaves: this is a public-key derivation.
    let key = k256::ecdsa::SigningKey::from_slice(priv_bytes).unwrap();
    let address = evm::address_of(&key);
    let addr_hex = hex::encode(address.as_slice());
    let binding = ChainKeyBinding {
        chain: ChainKeyId::new(DEV_TESTNET_CHAIN).unwrap(),
        public_address_hex: addr_hex.clone(),
        evm_chain_id: Some(11155111),
        derivation_path: "m/44'/60'/0'/0/0".to_string(),
        // Hot-key dev custody: no KMS handle, so the ship-gate permits this key
        // for testnet/dev only (mainnet routes through the KMS path).
        kms_key_ref: None,
    };
    keystore
        .bind(&owner_scope(), binding, priv_bytes.to_vec())
        .await
        .unwrap();
    (keystore, addr_hex)
}

/// Assemble a real in-memory composition with the provisioned custodial
/// keystore (testnet ship-gate permits hot-key dev signing).
fn composition_with_keystore(
    keystore: Arc<SecretsKeyStore>,
) -> Arc<
    RebornAttestedComposition<
        ironclaw_reborn_composition::NoopBroadcaster,
        InMemorySealedGrantStore,
        InMemorySigningLedger,
    >,
> {
    let bindings = Arc::new(InMemoryAttestedGateBindingStore::new());
    let grants = Arc::new(InMemorySealedGrantStore::new());
    let ship_gate = CustodialMainnetShipGate::new(false).build_chain_ship_gate(None);
    Arc::new(RebornAttestedComposition::new_in_memory(
        bindings,
        keystore,
        ship_gate,
        grants,
        ProviderRegistry::new(),
    ))
}

#[tokio::test]
async fn custodial_request_signature_raises_gate_and_existing_resolve_path_continues() {
    let priv_bytes = [0x11u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));

    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();
    let context = execution_context(owner_scope());
    let input = json!({
        "provider_hint": "custodial",
        "signer_account": account,
        "decoded": decoded,
    });

    // Drive the raise through the exact trait method DefaultHostRuntime calls.
    let outcome = hook
        .raise(AttestedRaiseRequest::new(
            capability_id.clone(),
            context,
            input,
        ))
        .await;

    let gate = match outcome {
        RuntimeCapabilityOutcome::AttestedSigningRequired(gate) => gate,
        other => panic!("expected AttestedSigningRequired, got {other:?}"),
    };
    assert_eq!(gate.capability_id, capability_id);
    assert!(!gate.expected_tx_hash.is_empty());

    // The binding the loop's gate ref maps to is `gate:attested-<gate_id>`. The
    // resolve path reads the binding back from the SAME composition's store.
    let gate_ref_str = format!("gate:attested-{}", gate.gate_id.as_str());
    let signing_gate_ref = SigningGateRef::new(gate_ref_str);
    let binding = composition
        .bindings()
        .get_sync(&signing_gate_ref)
        .expect("authoritative binding persisted on raise");
    // The persisted decoded tx is exactly what resolve recomputes the hash from.
    assert_eq!(binding.decoded, decoded);

    // The existing resolve path: drive the real signer-continuation driver. The
    // driver NEVER accepts a caller-supplied tx (#4067 byte-drift defense): it
    // reconstructs the signable FROM the authoritative decoded binding, claims
    // the sealed one-shot grant, re-checks the hash, custodial-signs, and
    // broadcasts.
    //
    // NOTE on the proof type: the custodial branch
    // (`ProviderId::Custodial` at driver.rs) IGNORES the `proof` argument
    // entirely — it reconstructs and re-hashes the signable from the binding
    // and signs with the keystore; the proof is only read on the external-wallet
    // branch (where a wallet attestation must be verified). There is no
    // `SigningProof::Custodial` variant. We pass an empty placeholder here and
    // separately assert the proof is irrelevant for custodial in
    // `custodial_continuation_ignores_proof_type` below.
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);
    let continuation = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &proof)
        .await
        .expect("existing resolve path continues a raised custodial gate");
    // The test composition uses `NoopBroadcaster` (`submits() == false`), which
    // is a dry-run path under #4067: the tx is custodial-SIGNED but never
    // broadcast, so the ledger lands at `Signed` with `NotBroadcast` — the
    // runtime never reports a real `BroadcastSubmitted` for a non-broadcaster.
    assert_eq!(
        continuation.ledger_state,
        ironclaw_attestation::SigningLedgerState::Signed
    );
    assert!(matches!(
        continuation.broadcast,
        ironclaw_attested_runtime::BroadcastDisposition::NotBroadcast { .. }
    ));

    // The one-shot grant was sealed on raise and is now claimed: a replayed
    // continuation must fail closed.
    let replay = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &proof)
        .await;
    assert!(
        replay.is_err(),
        "replayed continuation must fail closed (grant/ledger guard)"
    );
}

#[tokio::test]
async fn malformed_request_signature_params_fail_closed() {
    // The raise path must fail closed (Failed/InvalidInput) — never panic or
    // raise a half-formed gate — when the agent-supplied params cannot be
    // deserialized into RequestSignatureParams. Covers missing fields, wrong
    // types, and a non-object body.
    let priv_bytes = [0x33u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();

    let malformed_inputs = vec![
        // Missing `decoded` field.
        json!({ "provider_hint": "custodial", "signer_account": account }),
        // Missing `signer_account` field.
        json!({ "provider_hint": "custodial", "decoded": decoded }),
        // Unknown provider_hint value (not a valid enum tag).
        json!({ "provider_hint": "bogus", "signer_account": account, "decoded": decoded }),
        // Wrong type for signer_account (number instead of string).
        json!({ "provider_hint": "custodial", "signer_account": 42, "decoded": decoded }),
        // Body is not a JSON object at all.
        json!("not an object"),
        json!(null),
    ];

    for input in malformed_inputs {
        let outcome = hook
            .raise(AttestedRaiseRequest::new(
                capability_id.clone(),
                execution_context(owner_scope()),
                input.clone(),
            ))
            .await;

        match outcome {
            RuntimeCapabilityOutcome::Failed(failure) => {
                assert_eq!(failure.capability_id, capability_id);
                assert_eq!(
                    failure.kind,
                    RuntimeFailureKind::InvalidInput,
                    "malformed input {input} should map to InvalidInput"
                );
            }
            other => panic!("expected Failed for malformed input {input}, got {other:?}"),
        }
    }

    // No binding was persisted for any malformed raise: a fabricated gate ref
    // has no binding, so resolve fails closed with MissingBinding.
    let signing_gate_ref = SigningGateRef::new("gate:attested-malformed-none");
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);
    let err = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &proof)
        .await
        .expect_err("no binding persisted for any malformed raise");
    assert!(matches!(
        err,
        ironclaw_attested_runtime::ContinuationError::MissingBinding
    ));
}

#[tokio::test]
async fn signing_context_uses_user_id_when_no_project_or_agent() {
    // When the execution context has no project_id and no agent_id, the signing
    // context's scope label and actor label both fall back to the user_id. The
    // raise still succeeds end-to-end (custodial) and persists a binding whose
    // signing context reflects the fallback.
    let priv_bytes = [0x44u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();

    // Owner scope (so the keystore key is found) but with no project + no agent.
    let mut scope = owner_scope();
    scope.project_id = None;
    scope.agent_id = None;
    let user_id = scope.user_id.as_str().to_string();

    let input = json!({
        "provider_hint": "custodial",
        "signer_account": account,
        "decoded": decoded,
    });
    let outcome = hook
        .raise(AttestedRaiseRequest::new(
            capability_id.clone(),
            execution_context(scope),
            input,
        ))
        .await;

    let gate = match outcome {
        RuntimeCapabilityOutcome::AttestedSigningRequired(gate) => gate,
        other => panic!("expected AttestedSigningRequired, got {other:?}"),
    };

    let signing_gate_ref = SigningGateRef::new(format!("gate:attested-{}", gate.gate_id.as_str()));
    let binding = composition
        .bindings()
        .get_sync(&signing_gate_ref)
        .expect("binding persisted on raise");
    // Both fall back to the user id when project/agent are absent.
    assert_eq!(binding.context.scope.as_str(), user_id);
    assert_eq!(binding.context.actor.as_str(), user_id);
    assert_eq!(binding.context.user.as_str(), user_id);
}

#[tokio::test]
async fn near_and_walletconnect_provider_hints_fail_closed_without_raising() {
    let priv_bytes = [0x22u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();

    for hint in ["near_redirect", "wallet_connect", "injected"] {
        let input = json!({
            "provider_hint": hint,
            "signer_account": account,
            "decoded": decoded,
        });
        let outcome = hook
            .raise(AttestedRaiseRequest::new(
                capability_id.clone(),
                execution_context(owner_scope()),
                input,
            ))
            .await;

        match outcome {
            RuntimeCapabilityOutcome::Failed(failure) => {
                assert_eq!(failure.capability_id, capability_id);
                assert_eq!(failure.kind, RuntimeFailureKind::Backend);
            }
            other => panic!("expected Failed for hint {hint}, got {other:?}"),
        }
    }

    // No gate was raised and no grant sealed: a fabricated gate ref has no
    // binding, so resolve fails closed with MissingBinding.
    let signing_gate_ref = SigningGateRef::new("gate:attested-does-not-exist");
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);
    let err = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &proof)
        .await
        .expect_err("no binding was persisted for any failed-closed raise");
    assert!(matches!(
        err,
        ironclaw_attested_runtime::ContinuationError::MissingBinding
    ));
}

#[tokio::test]
async fn custodial_continuation_ignores_proof_type() {
    // The custodial resolve branch reconstructs and re-hashes the signable from
    // the authoritative binding and signs with the keystore; it NEVER reads the
    // `proof` argument (there is no `SigningProof::Custodial` variant). This test
    // pins that contract: a custodial gate raised here continues successfully
    // when handed a non-WebAuthn proof type (`WalletConnectProof`), proving the
    // proof type is irrelevant on the custodial branch. If proof validation is
    // ever added to the custodial path, this test must change deliberately.
    let priv_bytes = [0x55u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();
    let input = json!({
        "provider_hint": "custodial",
        "signer_account": account,
        "decoded": decoded,
    });

    let gate = match hook
        .raise(AttestedRaiseRequest::new(
            capability_id.clone(),
            execution_context(owner_scope()),
            input,
        ))
        .await
    {
        RuntimeCapabilityOutcome::AttestedSigningRequired(gate) => gate,
        other => panic!("expected AttestedSigningRequired, got {other:?}"),
    };

    let signing_gate_ref = SigningGateRef::new(format!("gate:attested-{}", gate.gate_id.as_str()));

    // A proof type that is NOT the custodial-adjacent WebAuthn assertion. The
    // custodial branch must still continue: the proof is never read.
    let unrelated_proof = SigningProof::WalletConnectProof(vec![0xde, 0xad]);
    let continuation = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &unrelated_proof)
        .await
        .expect("custodial continuation succeeds regardless of proof type");
    assert_eq!(
        continuation.ledger_state,
        ironclaw_attestation::SigningLedgerState::Signed
    );
}

#[tokio::test]
async fn overlong_signer_account_fails_closed_without_raising() {
    // `signer_account` is raw agent-supplied JSON and enters the hash domain via
    // the signing context. An over-long string is rejected (InvalidInput) before
    // any hashing/binding work, closing the unbounded-allocation /
    // hash-domain-confusion path. Fail-closed: no gate is raised, no grant sealed.
    let priv_bytes = [0x66u8; 32];
    let (keystore, _account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    let composition = composition_with_keystore(Arc::clone(&keystore));
    let hook = RebornAttestedRaiseHook::new(Arc::clone(&composition));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();

    // 129 bytes: one over the 128-byte chain-agnostic cap.
    let overlong = "a".repeat(129);
    let input = json!({
        "provider_hint": "custodial",
        "signer_account": overlong,
        "decoded": decoded,
    });
    let outcome = hook
        .raise(AttestedRaiseRequest::new(
            capability_id.clone(),
            execution_context(owner_scope()),
            input,
        ))
        .await;
    match outcome {
        RuntimeCapabilityOutcome::Failed(failure) => {
            assert_eq!(failure.capability_id, capability_id);
            assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
        }
        other => panic!("expected Failed for overlong signer_account, got {other:?}"),
    }

    // No binding persisted for the rejected raise.
    let signing_gate_ref = SigningGateRef::new("gate:attested-overlong-none");
    let proof = SigningProof::WebAuthnAssertionProof(vec![]);
    let err = composition
        .driver()
        .continue_after_resolved(&signing_gate_ref, &proof)
        .await
        .expect_err("no binding persisted for an over-long signer_account");
    assert!(matches!(
        err,
        ironclaw_attested_runtime::ContinuationError::MissingBinding
    ));
}

#[tokio::test]
async fn concurrent_register_of_same_gate_serializes_to_one_winner() {
    // `register_attested_gate` is the serializer against a double-raise race for
    // the SAME gate: the one-shot grant seal is an atomic CAS (`AlreadySealed`
    // → `DuplicateBinding`) and the binding `put` is insert-only. Firing two
    // concurrent registrations for an IDENTICAL `(gate_ref, binding)` — the
    // grant key is derived from the binding's context + hash, so both share it —
    // must collapse to exactly one success and one `DuplicateBinding`, never two
    // accepted registrations.
    //
    // (Two independent `request_signature` invocations each mint a fresh gate_id
    // → distinct gate_ref/run_id → distinct grant key, so they are intentionally
    // NOT the same gate. The race this guards is re-registering one gate_ref,
    // which is exactly what this drives.)
    let priv_bytes = [0x77u8; 32];
    let (keystore, account) = keystore_with_evm_key(&priv_bytes).await;
    let (_tx, decoded) = sample_evm();

    // Raise once into a throwaway composition to obtain a valid authoritative
    // binding (real hash + context + decoded tx) without hand-constructing it.
    let seed = composition_with_keystore(Arc::clone(&keystore));
    let seed_hook = RebornAttestedRaiseHook::new(Arc::clone(&seed));
    let capability_id = CapabilityId::new("builtin.request_signature").unwrap();
    let gate = match seed_hook
        .raise(AttestedRaiseRequest::new(
            capability_id.clone(),
            execution_context(owner_scope()),
            json!({
                "provider_hint": "custodial",
                "signer_account": account,
                "decoded": decoded,
            }),
        ))
        .await
    {
        RuntimeCapabilityOutcome::AttestedSigningRequired(gate) => gate,
        other => panic!("expected AttestedSigningRequired, got {other:?}"),
    };
    let gate_ref = SigningGateRef::new(format!("gate:attested-{}", gate.gate_id.as_str()));
    let binding = seed
        .bindings()
        .get_sync(&gate_ref)
        .expect("seed binding persisted");

    // Fresh composition that has never seen this gate. Two concurrent
    // registrations contend on the grant CAS + insert-only binding store.
    let target = composition_with_keystore(Arc::clone(&keystore));
    let t_a = Arc::clone(&target);
    let t_b = Arc::clone(&target);
    let gr_a = gate_ref.clone();
    let gr_b = gate_ref.clone();
    let b_a = binding.clone();
    let b_b = binding.clone();
    let (res_a, res_b) = tokio::join!(
        async move { t_a.register_attested_gate(gr_a, b_a, 1, None).await },
        async move { t_b.register_attested_gate(gr_b, b_b, 1, None).await },
    );

    let ok = [&res_a, &res_b].iter().filter(|r| r.is_ok()).count();
    let dup = [&res_a, &res_b]
        .iter()
        .filter(|r| {
            matches!(
                r,
                Err(ironclaw_reborn_composition::RegisterAttestedGateError::DuplicateBinding)
            )
        })
        .count();
    assert_eq!(ok, 1, "exactly one concurrent register wins the grant CAS");
    assert_eq!(
        dup, 1,
        "the losing concurrent register must fail closed as DuplicateBinding"
    );
}
