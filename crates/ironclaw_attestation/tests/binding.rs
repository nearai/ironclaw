//! Binding / anti-field-smuggling tests for the attested-signing core.
//!
//! These tests are the *point* of the crate: they prove that the
//! human-approved view and the signed bytes are bound to the same fields, that
//! the binding is deterministic, that every signing-relevant field is rendered,
//! that the EXPLICIT signer is bound (not a heuristic from the tx body), that
//! unknown/extra fields are rejected, that distinct Solana versioned messages
//! and distinct NEAR actions never collide, and that chain/network domain
//! separation prevents cross-chain collisions.

use ironclaw_attestation::{
    AttestationError, Bytes32, DecodedTransaction, EvmAccessListEntry, EvmAddress, EvmTransaction,
    NearAccessKey, NearAccessKeyPermission, NearAction, NearPublicKey, NearTransaction,
    RenderingSchemaVersion, SolanaAddressTableLookup, SolanaCompiledInstruction,
    SolanaMessageHeader, SolanaMessageVersion, SolanaTransaction, approved_tx_hash_for,
    canonical_signing_bytes, compute_approved_tx_hash, render,
};

const SV: RenderingSchemaVersion = RenderingSchemaVersion::CURRENT;
const SIGNER: &str = "0x1111111111111111111111111111111111111111";

/// Full hash via the SAFE public API (derives render + canonical from the same
/// tx and binds the explicit signer).
fn hash_of(tx: &DecodedTransaction, schema: RenderingSchemaVersion) -> [u8; 32] {
    *approved_tx_hash_for(tx, SIGNER, schema)
        .expect("hash")
        .as_bytes()
}

/// Canonical signing bytes for a well-formed sample (panics on overflow, which
/// the dedicated fail-closed tests cover explicitly).
fn canon(tx: &DecodedTransaction, schema: RenderingSchemaVersion) -> Vec<u8> {
    canonical_signing_bytes(tx, schema).expect("canonical bytes")
}

/// Render a well-formed sample (panics on overflow).
fn render_ok(
    tx: &DecodedTransaction,
    schema: RenderingSchemaVersion,
) -> ironclaw_attestation::RenderedTx {
    render(tx, schema).expect("render")
}

fn sample_evm() -> EvmTransaction {
    EvmTransaction {
        chain_id: 1,
        nonce: 7,
        tx_type: 2,
        to: Some(EvmAddress([0x11; 20])),
        value: vec![0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00],
        data: vec![0xab, 0xcd],
        gas_limit: 21000,
        gas_price: None,
        max_fee_per_gas: Some(vec![0x09, 0x18, 0x4e, 0x72, 0xa0, 0x00]),
        max_priority_fee_per_gas: Some(vec![0x3b, 0x9a, 0xca, 0x00]),
        access_list: vec![EvmAccessListEntry {
            address: EvmAddress([0x22; 20]),
            storage_keys: vec![Bytes32([0x33; 32])],
        }],
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: vec![],
    }
}

fn ed25519_pk(seed: u8) -> NearPublicKey {
    NearPublicKey {
        key_type: 0,
        data: vec![seed; 32],
    }
}

fn sample_solana() -> SolanaTransaction {
    SolanaTransaction {
        cluster: "mainnet-beta".to_string(),
        version: SolanaMessageVersion::V0,
        header: SolanaMessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        static_account_keys: vec![Bytes32([0x44; 32]), Bytes32([0x55; 32])],
        recent_blockhash: Bytes32([0x66; 32]),
        instructions: vec![SolanaCompiledInstruction {
            program_id_index: 1,
            account_indices: vec![0],
            data: vec![1, 2, 3],
        }],
        address_table_lookups: vec![SolanaAddressTableLookup {
            account_key: Bytes32([0x99; 32]),
            writable_indexes: vec![3],
            readonly_indexes: vec![7],
        }],
    }
}

fn sample_near() -> NearTransaction {
    NearTransaction {
        network: "mainnet".to_string(),
        signer_id: "alice.near".to_string(),
        public_key: ed25519_pk(0xaa),
        receiver_id: "bob.near".to_string(),
        nonce: 42,
        block_hash: Bytes32([0x88; 32]),
        actions: vec![NearAction::Transfer {
            deposit: vec![0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00],
        }],
    }
}

// ---- Determinism --------------------------------------------------------

#[test]
fn canonical_bytes_are_deterministic_across_calls() {
    let tx = DecodedTransaction::Evm(sample_evm());
    assert_eq!(canon(&tx, SV), canon(&tx, SV));
    assert_eq!(hash_of(&tx, SV), hash_of(&tx, SV));
}

#[test]
fn canonical_bytes_survive_serde_round_trip() {
    for tx in [
        DecodedTransaction::Evm(sample_evm()),
        DecodedTransaction::Solana(sample_solana()),
        DecodedTransaction::Near(sample_near()),
    ] {
        let json = serde_json::to_string(&tx).expect("serialize");
        let back: DecodedTransaction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, tx);
        assert_eq!(canon(&tx, SV), canon(&back, SV));
        assert_eq!(hash_of(&tx, SV), hash_of(&back, SV));
    }
}

// ---- Explicit-signer binding (finding #1) -------------------------------

#[test]
fn changing_signer_account_changes_hash_with_fixed_to() {
    // The `to` recipient stays fixed; only the explicit signer/account changes.
    // The hash MUST change — the approval commits to *who signs*, not to a
    // heuristic recovered from the tx body.
    let tx = DecodedTransaction::Evm(sample_evm());
    let h_a = *approved_tx_hash_for(&tx, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", SV)
        .expect("hash a")
        .as_bytes();
    let h_b = *approved_tx_hash_for(&tx, "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", SV)
        .expect("hash b")
        .as_bytes();
    assert_ne!(h_a, h_b, "changing the bound signer must change the hash");

    // Sanity: `to` is identical across both hashes (same tx), proving the
    // signer is an independent component, not derived from `to`.
    if let DecodedTransaction::Evm(evm) = &tx {
        assert_eq!(evm.to, Some(EvmAddress([0x11; 20])));
    }
}

#[test]
fn safe_api_matches_manual_component_assembly() {
    let tx = DecodedTransaction::Evm(sample_evm());
    let rendered = render_ok(&tx, SV);
    let canonical = canon(&tx, SV);
    let manual = compute_approved_tx_hash(
        &rendered,
        &canonical,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    assert_eq!(approved_tx_hash_for(&tx, SIGNER, SV).expect("hash"), manual);
}

// ---- Anti-field-smuggling: every component changes the hash -------------

#[test]
fn changing_schema_version_changes_hash() {
    let tx = DecodedTransaction::Evm(sample_evm());
    assert_ne!(hash_of(&tx, SV), hash_of(&tx, RenderingSchemaVersion(2)));
}

#[test]
fn changing_any_evm_field_changes_canonical_bytes_and_hash() {
    let base = sample_evm();
    let baseline = DecodedTransaction::Evm(base.clone());
    let baseline_bytes = canon(&baseline, SV);
    let baseline_hash = hash_of(&baseline, SV);

    type Mutator = Box<dyn Fn(&mut EvmTransaction)>;
    let mutate: Vec<(&str, Mutator)> = vec![
        ("chain_id", Box::new(|t| t.chain_id = 137)),
        ("nonce", Box::new(|t| t.nonce += 1)),
        ("tx_type", Box::new(|t| t.tx_type = 0)),
        ("to", Box::new(|t| t.to = Some(EvmAddress([0x99; 20])))),
        ("value", Box::new(|t| t.value = vec![0xff])),
        ("data", Box::new(|t| t.data = vec![0x00])),
        ("gas_limit", Box::new(|t| t.gas_limit = 99999)),
        (
            "max_fee",
            Box::new(|t| t.max_fee_per_gas = Some(vec![0x01])),
        ),
        (
            "max_prio",
            Box::new(|t| t.max_priority_fee_per_gas = Some(vec![0x02])),
        ),
        (
            "access_list",
            Box::new(|t| {
                t.access_list.push(EvmAccessListEntry {
                    address: EvmAddress([0xaa; 20]),
                    storage_keys: vec![],
                })
            }),
        ),
        (
            "blob_hashes",
            Box::new(|t| t.blob_versioned_hashes.push(Bytes32([0xbb; 32]))),
        ),
    ];

    for (name, f) in mutate {
        let mut m = base.clone();
        f(&mut m);
        let tx = DecodedTransaction::Evm(m);
        assert_ne!(
            canon(&tx, SV),
            baseline_bytes,
            "mutating `{name}` must change canonical bytes"
        );
        assert_ne!(
            hash_of(&tx, SV),
            baseline_hash,
            "mutating `{name}` must change the hash"
        );
    }
}

#[test]
fn changing_chain_network_or_tx_type_component_changes_hash() {
    let tx = DecodedTransaction::Evm(sample_evm());
    let rendered = render_ok(&tx, SV);
    let canonical = canon(&tx, SV);
    let base = compute_approved_tx_hash(
        &rendered,
        &canonical,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    let diff_net = compute_approved_tx_hash(
        &rendered,
        &canonical,
        SIGNER,
        "eip155:137",
        &tx.tx_type_label(),
        SV,
    );
    let diff_type = compute_approved_tx_hash(
        &rendered,
        &canonical,
        SIGNER,
        &tx.chain_network(),
        "evm-type-0",
        SV,
    );
    assert_ne!(base, diff_net);
    assert_ne!(base, diff_type);
}

#[test]
fn same_render_different_canonical_bytes_changes_hash() {
    // The render and canonical bytes are independent components of the hash.
    // If an attacker could keep the displayed render fixed while swapping the
    // signed bytes (approve-A / sign-B), the hash MUST still change.
    let tx = DecodedTransaction::Evm(sample_evm());
    let rendered = render_ok(&tx, SV);
    let canonical_a = canon(&tx, SV);
    let mut canonical_b = canonical_a.clone();
    *canonical_b.last_mut().expect("non-empty") ^= 0xff;
    assert_ne!(canonical_a, canonical_b);

    let h_a = compute_approved_tx_hash(
        &rendered,
        &canonical_a,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    let h_b = compute_approved_tx_hash(
        &rendered,
        &canonical_b,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    assert_ne!(
        h_a, h_b,
        "same render with different canonical bytes must change the hash"
    );
}

#[test]
fn changing_render_component_changes_hash() {
    let tx = DecodedTransaction::Evm(sample_evm());
    let canonical = canon(&tx, SV);
    let r1 = render_ok(&tx, SV);
    let mut r2 = r1.clone();
    r2.fields[0].value = "tampered".to_string();
    let h1 = compute_approved_tx_hash(
        &r1,
        &canonical,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    let h2 = compute_approved_tx_hash(
        &r2,
        &canonical,
        SIGNER,
        &tx.chain_network(),
        &tx.tx_type_label(),
        SV,
    );
    assert_ne!(h1, h2);
}

// ---- Unknown / extra field rejection (findings #2, threats #8/#9) -------

#[test]
fn decoded_transaction_rejects_unknown_top_level_field() {
    let json = r#"{"chain":"evm","chain_id":1,"nonce":0,"tx_type":0,"to":null,
        "value":[],"data":[],"gas_limit":0,"gas_price":null,"max_fee_per_gas":null,
        "max_priority_fee_per_gas":null,"access_list":[],"max_fee_per_blob_gas":null,
        "blob_versioned_hashes":[],"smuggled":"evil"}"#;
    let result: Result<DecodedTransaction, _> = serde_json::from_str(json);
    assert!(result.is_err(), "extra top-level field must be rejected");
}

#[test]
fn evm_access_list_entry_rejects_unknown_field() {
    let json = r#"{"address":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "storage_keys":[],"extra":1}"#;
    let result: Result<ironclaw_attestation::EvmAccessListEntry, _> = serde_json::from_str(json);
    assert!(result.is_err(), "extra access-list field must be rejected");
}

#[test]
fn solana_nested_structs_reject_unknown_fields() {
    // Header.
    assert!(
        serde_json::from_str::<SolanaMessageHeader>(
            r#"{"num_required_signatures":1,"num_readonly_signed_accounts":0,
                "num_readonly_unsigned_accounts":0,"extra":1}"#
        )
        .is_err(),
        "header extra field must be rejected"
    );
    // Compiled instruction.
    assert!(
        serde_json::from_str::<SolanaCompiledInstruction>(
            r#"{"program_id_index":0,"account_indices":[],"data":[],"extra":1}"#
        )
        .is_err(),
        "instruction extra field must be rejected"
    );
    // Address-table lookup.
    assert!(
        serde_json::from_str::<SolanaAddressTableLookup>(
            r#"{"account_key":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
                "writable_indexes":[],"readonly_indexes":[],"extra":1}"#
        )
        .is_err(),
        "lookup extra field must be rejected"
    );
}

#[test]
fn near_nested_structs_reject_unknown_fields() {
    assert!(
        serde_json::from_str::<NearPublicKey>(r#"{"key_type":0,"data":[],"extra":1}"#).is_err(),
        "public-key extra field must be rejected"
    );
    assert!(
        serde_json::from_str::<NearAccessKey>(r#"{"nonce":0,"permission":"FullAccess","extra":1}"#)
            .is_err(),
        "access-key extra field must be rejected"
    );
    // Action variant extra field.
    assert!(
        serde_json::from_str::<NearAction>(r#"{"Transfer":{"deposit":[],"extra":1}}"#).is_err(),
        "action variant extra field must be rejected"
    );
    // Whole NEAR tx extra field.
    let json = r#"{"network":"mainnet","signer_id":"a.near","public_key":{"key_type":0,"data":[]},
        "receiver_id":"b.near","nonce":0,
        "block_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        "actions":[],"extra":1}"#;
    assert!(
        serde_json::from_str::<NearTransaction>(json).is_err(),
        "near tx extra field must be rejected"
    );
}

// ---- Solana full-message-model injectivity (finding #3) -----------------

#[test]
fn solana_distinct_messages_hash_differently() {
    let base = DecodedTransaction::Solana(sample_solana());
    let baseline = hash_of(&base, SV);
    let baseline_bytes = canon(&base, SV);

    type Mutator = Box<dyn Fn(&mut SolanaTransaction)>;
    let mutate: Vec<(&str, Mutator)> = vec![
        (
            "version",
            Box::new(|t| t.version = SolanaMessageVersion::Legacy),
        ),
        (
            "num_required_signatures",
            Box::new(|t| t.header.num_required_signatures = 2),
        ),
        (
            "num_readonly_signed",
            Box::new(|t| t.header.num_readonly_signed_accounts = 1),
        ),
        (
            "num_readonly_unsigned",
            Box::new(|t| t.header.num_readonly_unsigned_accounts = 0),
        ),
        (
            "static_keys",
            Box::new(|t| t.static_account_keys.push(Bytes32([0x01; 32]))),
        ),
        (
            "blockhash",
            Box::new(|t| t.recent_blockhash = Bytes32([0x00; 32])),
        ),
        (
            "program_id_index",
            Box::new(|t| t.instructions[0].program_id_index = 0),
        ),
        (
            "account_indices",
            Box::new(|t| t.instructions[0].account_indices = vec![1]),
        ),
        (
            "instruction_data",
            Box::new(|t| t.instructions[0].data = vec![9]),
        ),
        (
            "lookup_account_key",
            Box::new(|t| t.address_table_lookups[0].account_key = Bytes32([0x00; 32])),
        ),
        (
            "lookup_writable",
            Box::new(|t| t.address_table_lookups[0].writable_indexes = vec![4]),
        ),
        (
            "lookup_readonly",
            Box::new(|t| t.address_table_lookups[0].readonly_indexes = vec![8]),
        ),
    ];

    for (name, f) in mutate {
        let mut m = sample_solana();
        f(&mut m);
        let tx = DecodedTransaction::Solana(m);
        assert_ne!(
            canon(&tx, SV),
            baseline_bytes,
            "mutating Solana `{name}` must change canonical bytes"
        );
        assert_ne!(
            hash_of(&tx, SV),
            baseline,
            "mutating Solana `{name}` must change the hash"
        );
    }
}

#[test]
fn solana_legacy_and_v0_with_same_contents_differ() {
    // Same header/keys/instructions, no lookups: the version byte alone (and
    // tx-type label) must distinguish a legacy message from a v0 message.
    let mut legacy = sample_solana();
    legacy.version = SolanaMessageVersion::Legacy;
    legacy.address_table_lookups.clear();
    let mut v0 = legacy.clone();
    v0.version = SolanaMessageVersion::V0;

    assert_ne!(
        canon(&DecodedTransaction::Solana(legacy.clone()), SV),
        canon(&DecodedTransaction::Solana(v0.clone()), SV)
    );
    assert_ne!(
        hash_of(&DecodedTransaction::Solana(legacy), SV),
        hash_of(&DecodedTransaction::Solana(v0), SV)
    );
}

#[test]
fn solana_message_bytes_match_known_layout() {
    // A minimal legacy message with a known byte layout, asserting the
    // hand-rolled shortvec encoding matches the on-chain `Message` format.
    let tx = SolanaTransaction {
        cluster: "devnet".to_string(),
        version: SolanaMessageVersion::Legacy,
        header: SolanaMessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        static_account_keys: vec![Bytes32([0xa1; 32]), Bytes32([0xb2; 32])],
        recent_blockhash: Bytes32([0xc3; 32]),
        instructions: vec![SolanaCompiledInstruction {
            program_id_index: 1,
            account_indices: vec![0],
            data: vec![0xde, 0xad],
        }],
        address_table_lookups: vec![],
    };
    let bytes = canon(&DecodedTransaction::Solana(tx), SV);
    // The canonical bytes embed the message; assert the message slice appears.
    // Expected message = header(1,0,1) ∥ shortvec(2) ∥ key0 ∥ key1 ∥ blockhash
    //                    ∥ shortvec(1) ∥ [1, shortvec(1), 0, shortvec(2), de ad]
    let mut expected = vec![1u8, 0, 1, 2];
    expected.extend_from_slice(&[0xa1; 32]);
    expected.extend_from_slice(&[0xb2; 32]);
    expected.extend_from_slice(&[0xc3; 32]);
    // num_instructions(1) ∥ program_id_index(1) ∥ shortvec(1 acct) ∥ acct(0)
    //   ∥ shortvec(2 data) ∥ data(de ad)
    expected.extend_from_slice(&[1, 1, 1, 0, 2, 0xde, 0xad]);
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "canonical bytes must embed the exact legacy message layout"
    );
}

// ---- NEAR full-action-model injectivity (finding #4) --------------------

fn near_with_action(action: NearAction) -> DecodedTransaction {
    let mut tx = sample_near();
    tx.actions = vec![action];
    DecodedTransaction::Near(tx)
}

#[test]
fn near_distinct_actions_hash_differently() {
    let variants = vec![
        NearAction::CreateAccount,
        NearAction::DeployContract {
            code: vec![1, 2, 3],
        },
        NearAction::FunctionCall {
            method_name: "do_it".to_string(),
            args: vec![9],
            gas: 1000,
            deposit: vec![0x01],
        },
        NearAction::Transfer {
            deposit: vec![0x05],
        },
        NearAction::Stake {
            stake: vec![0x07],
            public_key: ed25519_pk(0x11),
        },
        NearAction::AddKey {
            public_key: ed25519_pk(0x22),
            access_key: NearAccessKey {
                nonce: 1,
                permission: NearAccessKeyPermission::FullAccess,
            },
        },
        NearAction::DeleteKey {
            public_key: ed25519_pk(0x33),
        },
        NearAction::DeleteAccount {
            beneficiary_id: "ben.near".to_string(),
        },
        NearAction::Delegate {
            sender_id: "s.near".to_string(),
            receiver_id: "r.near".to_string(),
            inner_actions: vec![],
            nonce: 2,
            max_block_height: 100,
            public_key: ed25519_pk(0x44),
        },
    ];
    let mut hashes = std::collections::HashSet::new();
    let mut byte_sets = std::collections::HashSet::new();
    for action in variants {
        let tx = near_with_action(action);
        assert!(
            byte_sets.insert(canon(&tx, SV)),
            "each NEAR action variant must produce unique canonical bytes"
        );
        assert!(
            hashes.insert(hash_of(&tx, SV)),
            "each NEAR action variant must produce a unique hash"
        );
    }
}

#[test]
fn near_addkey_permission_fields_all_bind() {
    let base = NearAction::AddKey {
        public_key: ed25519_pk(0x22),
        access_key: NearAccessKey {
            nonce: 1,
            permission: NearAccessKeyPermission::FunctionCall {
                allowance: Some(vec![0x10]),
                receiver_id: "contract.near".to_string(),
                method_names: vec!["m1".to_string()],
            },
        },
    };
    let baseline = hash_of(&near_with_action(base.clone()), SV);

    let mutated = [
        NearAction::AddKey {
            public_key: ed25519_pk(0x22),
            access_key: NearAccessKey {
                nonce: 1,
                permission: NearAccessKeyPermission::FunctionCall {
                    allowance: None, // changed
                    receiver_id: "contract.near".to_string(),
                    method_names: vec!["m1".to_string()],
                },
            },
        },
        NearAction::AddKey {
            public_key: ed25519_pk(0x22),
            access_key: NearAccessKey {
                nonce: 1,
                permission: NearAccessKeyPermission::FunctionCall {
                    allowance: Some(vec![0x10]),
                    receiver_id: "evil.near".to_string(), // changed
                    method_names: vec!["m1".to_string()],
                },
            },
        },
        NearAction::AddKey {
            public_key: ed25519_pk(0x22),
            access_key: NearAccessKey {
                nonce: 1,
                permission: NearAccessKeyPermission::FunctionCall {
                    allowance: Some(vec![0x10]),
                    receiver_id: "contract.near".to_string(),
                    method_names: vec!["m1".to_string(), "m2".to_string()], // changed
                },
            },
        },
    ];
    for m in mutated {
        assert_ne!(
            hash_of(&near_with_action(m), SV),
            baseline,
            "every AddKey permission field must bind into the hash"
        );
    }
}

#[test]
fn near_public_key_binds() {
    // The transaction public_key participates in the hash (previously omitted).
    let mut a = sample_near();
    a.public_key = ed25519_pk(0x01);
    let mut b = sample_near();
    b.public_key = ed25519_pk(0x02);
    assert_ne!(
        hash_of(&DecodedTransaction::Near(a), SV),
        hash_of(&DecodedTransaction::Near(b), SV)
    );
}

// ---- Render coverage: render touches every consumed field ---------------

fn assert_render_covers(tx: &DecodedTransaction, expected_labels: &[&str]) {
    let rendered = render_ok(tx, SV);
    for label in expected_labels {
        assert!(
            rendered.has_label(label),
            "render for {} must surface `{label}`; got labels {:?}",
            tx.chain_tag(),
            rendered.fields.iter().map(|f| &f.label).collect::<Vec<_>>()
        );
    }
    assert!(!canon(tx, SV).is_empty());
    assert!(rendered.fields.len() >= expected_labels.len());
}

#[test]
fn evm_render_surfaces_every_signing_field() {
    assert_render_covers(
        &DecodedTransaction::Evm(sample_evm()),
        &[
            "Chain ID",
            "Nonce",
            "Tx Type",
            "To",
            "Value (wei)",
            "Data",
            "Gas Limit",
            "Max Fee/Gas",
            "Max Priority Fee/Gas",
            "Access List Address",
        ],
    );
}

#[test]
fn solana_render_surfaces_every_signing_field() {
    assert_render_covers(
        &DecodedTransaction::Solana(sample_solana()),
        &[
            "Cluster",
            "Message Version",
            "Required Signatures",
            "Readonly Signed Accounts",
            "Readonly Unsigned Accounts",
            "Recent Blockhash",
            "Static Account Key",
            "Instruction Program Index",
            "Instruction Account Indices",
            "Instruction Data",
            "Lookup Table Account",
            "Signed Message Bytes",
        ],
    );
}

#[test]
fn near_render_surfaces_every_signing_field() {
    assert_render_covers(
        &DecodedTransaction::Near(sample_near()),
        &[
            "Network",
            "Signer",
            "Public Key",
            "Receiver",
            "Access-Key Nonce",
            "Block Hash",
            "Action Kind",
            "Deposit (yocto)",
            "Signed Transaction Bytes",
        ],
    );
}

#[test]
fn render_field_count_matches_canonical_field_count() {
    // Both views derive from the same projection. The canonical bytes encode
    // the field count as a u64 immediately after the domain tag and the four
    // length-prefixed headers (chain_tag, chain_network, tx_type, schema). We
    // parse it out and assert it EQUALS the rendered field count — guarding
    // against a field added to one path but not the other (fixes the previously
    // mislabeled "field count" test that only checked non-emptiness).
    for tx in [
        DecodedTransaction::Evm(sample_evm()),
        DecodedTransaction::Solana(sample_solana()),
        DecodedTransaction::Near(sample_near()),
    ] {
        let rendered = render_ok(&tx, SV);
        let bytes = canon(&tx, SV);
        let count = parse_canonical_field_count(&bytes);
        assert_eq!(
            count,
            rendered.fields.len() as u64,
            "canonical field count must equal rendered field count for {}",
            tx.chain_tag()
        );
    }
}

/// Parse the `u64` field count out of the canonical signing bytes.
///
/// Layout: `DOMAIN ∥ lp(chain_tag) ∥ lp(chain_network) ∥ lp(tx_type) ∥
/// lp(schema u16) ∥ u64_be(field_count) ∥ ...`, where `lp(x) = u64_be(len) ∥ x`.
fn parse_canonical_field_count(bytes: &[u8]) -> u64 {
    const DOMAIN_LEN: usize = b"ironclaw.attestation.canonical.v1".len();
    let mut pos = DOMAIN_LEN;
    let skip_lp = |bytes: &[u8], pos: &mut usize| {
        let len = u64::from_be_bytes(bytes[*pos..*pos + 8].try_into().expect("len")) as usize;
        *pos += 8 + len;
    };
    skip_lp(bytes, &mut pos); // chain_tag
    skip_lp(bytes, &mut pos); // chain_network
    skip_lp(bytes, &mut pos); // tx_type
    skip_lp(bytes, &mut pos); // schema version
    u64::from_be_bytes(bytes[pos..pos + 8].try_into().expect("count"))
}

// ---- Cross-chain domain separation --------------------------------------

#[test]
fn evm_and_solana_with_similar_bytes_hash_differently() {
    let shared = [0x42u8; 32];
    let evm = DecodedTransaction::Evm(EvmTransaction {
        chain_id: 1,
        nonce: 0,
        tx_type: 0,
        to: Some(EvmAddress([0x42; 20])),
        value: shared.to_vec(),
        data: vec![],
        gas_limit: 0,
        gas_price: Some(shared.to_vec()),
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
        access_list: vec![],
        max_fee_per_blob_gas: None,
        blob_versioned_hashes: vec![],
    });
    let sol = DecodedTransaction::Solana(SolanaTransaction {
        cluster: "mainnet-beta".to_string(),
        version: SolanaMessageVersion::Legacy,
        header: SolanaMessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 0,
        },
        static_account_keys: vec![Bytes32(shared)],
        recent_blockhash: Bytes32(shared),
        instructions: vec![],
        address_table_lookups: vec![],
    });
    assert_ne!(canon(&evm, SV), canon(&sol, SV));
    assert_ne!(hash_of(&evm, SV), hash_of(&sol, SV));
}

#[test]
fn same_tx_on_different_evm_networks_hash_differently() {
    let mut a = sample_evm();
    a.chain_id = 1;
    let mut b = sample_evm();
    b.chain_id = 137;
    assert_ne!(
        hash_of(&DecodedTransaction::Evm(a), SV),
        hash_of(&DecodedTransaction::Evm(b), SV)
    );
}

// ---- Fail-closed wire encoding (henrypark133 Critical/High findings) -----
//
// These drive the SAFE public API (`approved_tx_hash_for` /
// `canonical_signing_bytes`) — not the private wire helpers — so they regress
// the actual call site a signing flow uses. An input that cannot be reproduced
// byte-for-byte as the signed payload must be REJECTED, never truncated: a
// truncated length prefix would let trailing bytes be reinterpreted as extra
// instructions/accounts (Solana) or desync the borsh stream (NEAR), smuggling
// fields past the what-you-see-is-what-you-sign binding.

#[test]
fn solana_oversized_instruction_data_is_rejected() {
    // Instruction data longer than the compact-u16 maximum cannot be
    // length-prefixed faithfully. The canonical/hash path must fail closed
    // rather than truncate the shortvec prefix.
    let mut sol = sample_solana();
    sol.instructions[0].data = vec![0u8; u16::MAX as usize + 1];
    let tx = DecodedTransaction::Solana(sol);
    assert_eq!(
        canonical_signing_bytes(&tx, SV),
        Err(AttestationError::SolanaShortVecOverflow {
            what: "instruction.data",
            len: u16::MAX as usize + 1,
            max: u16::MAX as usize,
        })
    );
    assert!(approved_tx_hash_for(&tx, SIGNER, SV).is_err());
    assert!(render(&tx, SV).is_err());
}

#[test]
fn solana_max_instruction_data_still_encodes() {
    // Exactly u16::MAX is representable and must succeed (boundary).
    let mut sol = sample_solana();
    sol.instructions[0].data = vec![0u8; u16::MAX as usize];
    let tx = DecodedTransaction::Solana(sol);
    assert!(canonical_signing_bytes(&tx, SV).is_ok());
    assert!(approved_tx_hash_for(&tx, SIGNER, SV).is_ok());
}

#[test]
fn near_oversized_deposit_is_rejected() {
    // A deposit carrying more than 16 big-endian bytes does not fit a borsh
    // u128. Wrapping it (as the old `wrapping_shl` did) would make the SIGNED
    // amount differ from the human-RENDERED amount — an approve-vs-sign
    // divergence — so canonicalization must fail closed.
    let mut near = sample_near();
    near.actions = vec![NearAction::Transfer {
        deposit: vec![0x01; 17],
    }];
    let tx = DecodedTransaction::Near(near);
    assert_eq!(
        canonical_signing_bytes(&tx, SV),
        Err(AttestationError::NearU128Overflow { len: 17 })
    );
    assert!(approved_tx_hash_for(&tx, SIGNER, SV).is_err());
}

#[test]
fn near_max_width_deposit_still_encodes() {
    // A full 16-byte (u128::MAX) deposit is representable and must succeed.
    let mut near = sample_near();
    near.actions = vec![NearAction::Transfer {
        deposit: vec![0xff; 16],
    }];
    let tx = DecodedTransaction::Near(near);
    assert!(canonical_signing_bytes(&tx, SV).is_ok());
    assert!(approved_tx_hash_for(&tx, SIGNER, SV).is_ok());
}

#[test]
fn near_delegate_action_serializes_empty_inner_actions_vector() {
    // NEP-366 DelegateAction places a `Vec<Action>` between `receiver_id` and
    // `nonce`. Omitting it (the previous bug) makes a borsh deserializer read
    // the 8-byte `nonce` as the actions-vector length, corrupting the parse.
    // We serialize an explicit empty actions vector (borsh `0u32`) in the
    // correct position; assert that exact slice appears in the canonical bytes.
    let mut near = sample_near();
    near.actions = vec![NearAction::Delegate {
        sender_id: "s.near".to_string(),
        receiver_id: "r.near".to_string(),
        inner_actions: vec![],
        nonce: 0x0102_0304_0506_0708,
        max_block_height: 100,
        public_key: ed25519_pk(0x44),
    }];
    let tx = DecodedTransaction::Near(near);
    let bytes = canon(&tx, SV);

    // Expected wire fragment for the Delegate action body:
    //   discriminant(8) ∥ borsh("s.near") ∥ borsh("r.near") ∥
    //   actions_len(0u32 le) ∥ nonce(u64 le) ∥ ...
    let mut expected = vec![8u8]; // Delegate discriminant
    expected.extend_from_slice(&6u32.to_le_bytes());
    expected.extend_from_slice(b"s.near");
    expected.extend_from_slice(&6u32.to_le_bytes());
    expected.extend_from_slice(b"r.near");
    expected.extend_from_slice(&0u32.to_le_bytes()); // empty inner actions vec
    expected.extend_from_slice(&0x0102_0304_0506_0708u64.to_le_bytes()); // nonce
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "Delegate action must serialize an explicit empty actions vector before the nonce"
    );

    // Distinct delegate transactions must still hash distinctly.
    let mut other = sample_near();
    other.actions = vec![NearAction::Delegate {
        sender_id: "s.near".to_string(),
        receiver_id: "r.near".to_string(),
        inner_actions: vec![],
        nonce: 0x0102_0304_0506_0709, // differs by one
        max_block_height: 100,
        public_key: ed25519_pk(0x44),
    }];
    assert_ne!(
        hash_of(&tx, SV),
        hash_of(&DecodedTransaction::Near(other), SV)
    );
}

#[test]
fn near_delegate_inner_actions_are_bound_into_canonical_bytes() {
    // WYSIWYS regression: a NEP-366 DelegateAction with non-empty inner actions
    // must commit those inner actions into the canonical bytes (and therefore
    // the ApprovedTxHash). A previous implementation hardcoded an empty
    // `Vec<Action>` regardless of the signed payload, so a real delegate of
    // `Transfer { 1 NEAR }` would produce the SAME hash as an empty delegate —
    // an approve-empty / sign-transfer mismatch. These two must hash distinctly.
    let inner = NearAction::Transfer {
        deposit: vec![0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00], // 1e18 yocto
    };
    let delegate_with_inner = |actions: Vec<NearAction>| {
        let mut near = sample_near();
        near.actions = vec![NearAction::Delegate {
            sender_id: "s.near".to_string(),
            receiver_id: "r.near".to_string(),
            inner_actions: actions,
            nonce: 7,
            max_block_height: 100,
            public_key: ed25519_pk(0x44),
        }];
        DecodedTransaction::Near(near)
    };

    let empty = delegate_with_inner(vec![]);
    let with_transfer = delegate_with_inner(vec![inner.clone()]);

    // The inner action must change the canonical bytes and the bound hash.
    assert_ne!(
        canon(&empty, SV),
        canon(&with_transfer, SV),
        "non-empty inner actions must change the canonical bytes"
    );
    assert_ne!(
        hash_of(&empty, SV),
        hash_of(&with_transfer, SV),
        "non-empty inner actions must change the ApprovedTxHash (WYSIWYS)"
    );

    // The inner action's bytes must actually appear inside the canonical bytes:
    // discriminant(3 = Transfer) ∥ deposit(u128 le) for 1e18 yocto.
    let bytes = canon(&with_transfer, SV);
    let mut expected = vec![3u8];
    expected.extend_from_slice(&1_000_000_000_000_000_000u128.to_le_bytes());
    assert!(
        bytes.windows(expected.len()).any(|w| w == expected),
        "the inner Transfer action must be serialized into the delegate's canonical bytes"
    );

    // And the inner action must surface in the human render too.
    let render = render_ok(&with_transfer, SV);
    assert!(
        render
            .fields
            .iter()
            .any(|f| f.label == "Delegate Inner Action"),
        "the human render must surface the delegated inner action"
    );
}
