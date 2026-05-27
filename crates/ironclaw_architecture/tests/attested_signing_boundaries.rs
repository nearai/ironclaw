//! Dependency-boundary tests for the attested-signing substrate.
//!
//! PR1 of the 10-PR attested-signing stack introduces
//! `ironclaw_signing_provider`, the provider-agnostic `SigningProvider` trait
//! crate. It pins the binding model every downstream crate (chain signing,
//! attestation, external wallets) depends on, so it MUST stay pure: zero chain,
//! crypto, or secrets dependencies. A regression that pulls any of those into
//! the trait crate would let chain-specific or key-handling code leak into the
//! shared abstraction and is caught here.
//!
//! See `docs/plans/2026-05-23-attested-signing-substrate.md`.

use std::process::Command;

use serde_json::Value;

/// The dependency names the signing-provider trait crate must never carry —
/// chain SDKs, crypto primitives, and key-custody crates. Matched as a prefix
/// against each dependency name so e.g. `alloy`, `alloy-primitives`, and
/// `solana-program` are all covered.
const FORBIDDEN_DEPENDENCY_PREFIXES: &[&str] = &[
    "solana-sdk",
    "solana-program",
    "solana",
    "near-",
    "alloy",
    "k256",
    "sha3",
    "webauthn-rs",
    "ironclaw_secrets",
    "ironclaw_chain_signing",
    "ironclaw_attestation",
];

#[test]
fn signing_provider_trait_crate_has_no_chain_crypto_or_secrets_dependency() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_signing_provider")
        .expect(
            "ironclaw_signing_provider must be a workspace member; add it to the root \
             Cargo.toml `workspace.members` (see attested-signing PR1)",
        );

    // Cover every dependency kind (normal, dev, build): the purity invariant
    // applies to the whole manifest. A chain/crypto crate sneaking in as a
    // dev-dependency would still mean the trait crate's tests reach for chain
    // code, defeating the "trait crate names types without chain deps" goal.
    let dependencies = package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array");

    let mut violations = Vec::new();
    for dependency in dependencies {
        let Some(name) = dependency["name"].as_str() else {
            continue;
        };
        for forbidden in FORBIDDEN_DEPENDENCY_PREFIXES {
            if name == *forbidden || name.starts_with(forbidden) {
                let kind = dependency["kind"].as_str().unwrap_or("normal").to_string();
                violations.push(format!("{name} (kind: {kind}) matched `{forbidden}`"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "ironclaw_signing_provider is the pure trait crate at the base of the attested-signing \
         stack and must carry no chain/crypto/secrets dependency. Forbidden dependencies found:\n{}\n\
         Concrete chain/crypto types belong in ironclaw_attestation (PR2) and the chain crates, \
         not the trait crate. See docs/plans/2026-05-23-attested-signing-substrate.md.",
        violations.join("\n")
    );
}

/// The dependency names the attestation crate must never carry. It is the
/// canonical / render / hash core plus (as of PR4) the WebAuthn registry +
/// verifier and the durable challenge store. It stays one layer above the chain
/// crates: no chain SDK and no key custody.
///
/// PR4 verifies passkey assertions with PURE-RUST crypto (`coset` for COSE_Key,
/// `p256` for ES256, `ed25519-dalek` for EdDSA). This tree is a `ring`/`rustls`
/// tree and does NOT accept `openssl`, so `webauthn-rs` / `webauthn-rs-core`
/// (which pull in `openssl`/`openssl-sys`) and `openssl*` itself are forbidden
/// here. Also forbidden: chain SDKs (solana/near/alloy), the EVM crypto
/// primitives that belong to the chain layer (`k256`/`sha3`), key custody
/// (`ironclaw_secrets`), and the chain-signing crate (`ironclaw_chain_signing`)
/// — the custodial keys and per-chain decode/sign/broadcast land in PR6, not
/// here. `sha2`/`serde`, and the pure-Rust crypto crates (`coset`, `ciborium`,
/// `p256`, `ed25519-dalek`) are allowed and deliberately absent from this list.
const ATTESTATION_FORBIDDEN_DEPENDENCY_PREFIXES: &[&str] = &[
    "solana-sdk",
    "solana-program",
    "solana",
    "near-",
    "alloy",
    "k256",
    "sha3",
    "webauthn-rs",
    "openssl",
    "ironclaw_secrets",
    "ironclaw_chain_signing",
];

#[test]
fn attestation_crate_has_no_chain_secrets_or_webauthn_dependency() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_attestation")
        .expect(
            "ironclaw_attestation must be a workspace member; add it to the root \
             Cargo.toml `workspace.members` (see attested-signing PR2)",
        );

    let dependencies = package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array");

    let mut violations = Vec::new();
    for dependency in dependencies {
        let Some(name) = dependency["name"].as_str() else {
            continue;
        };
        for forbidden in ATTESTATION_FORBIDDEN_DEPENDENCY_PREFIXES {
            if name == *forbidden || name.starts_with(forbidden) {
                let kind = dependency["kind"].as_str().unwrap_or("normal").to_string();
                violations.push(format!("{name} (kind: {kind}) matched `{forbidden}`"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "ironclaw_attestation is the canonical/render/hash + WebAuthn verifier core and must carry \
         no chain SDK, secrets, chain-signing, openssl, or webauthn-rs dependency — chain custody \
         lands in PR6 and openssl is banned tree-wide (pure-Rust crypto only). Forbidden \
         dependencies found:\n{}\nSee docs/plans/2026-05-23-attested-signing-substrate.md.",
        violations.join("\n")
    );
}

/// The whole tree is deliberately openssl-free (a `ring`/`rustls` tree). PR4's
/// WebAuthn verifier uses pure-Rust crypto (`coset`/`p256`/`ed25519-dalek`)
/// precisely so it adds NO `openssl`/`openssl-sys` native C dependency. This
/// test asserts the resolved dependency graph carries neither — catching a
/// regression anywhere in the workspace that would re-introduce openssl
/// (e.g. swapping the WebAuthn crypto back to `webauthn-rs-core`).
#[test]
fn workspace_graph_is_openssl_free() {
    let metadata = cargo_metadata_with_deps();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let offenders: Vec<String> = packages
        .iter()
        .filter_map(|p| p["name"].as_str())
        .filter(|name| *name == "openssl" || *name == "openssl-sys")
        .map(|name| name.to_string())
        .collect();

    assert!(
        offenders.is_empty(),
        "the workspace dependency graph must be openssl-free (this is a ring/rustls tree). \
         Found: {}. The attested-signing WebAuthn verifier must keep using pure-Rust crypto \
         (coset/p256/ed25519-dalek) and must not pull in webauthn-rs-core (which links openssl).",
        offenders.join(", ")
    );
}

fn cargo_metadata() -> Value {
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
}

/// Full resolved dependency graph (no `--no-deps`), used to assert the absence
/// of a transitive dependency such as `openssl`.
fn cargo_metadata_with_deps() -> Value {
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata: {error}"));

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata output must be JSON")
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}
