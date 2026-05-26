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

/// The dependency names the attestation crate (PR2) must never carry. It is the
/// canonical / render / hash core and stays one layer above the chain crates:
/// no chain SDK, no key custody, no webauthn (those land in PR4/PR6). `sha2`
/// (its hashing primitive) and `serde` are allowed and so are deliberately
/// absent from this list.
const ATTESTATION_FORBIDDEN_DEPENDENCY_PREFIXES: &[&str] = &[
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
        "ironclaw_attestation is the canonical/render/hash core (PR2) and must carry no chain \
         SDK, secrets, or webauthn dependency — those belong in PR4/PR6. Forbidden dependencies \
         found:\n{}\nSee docs/plans/2026-05-23-attested-signing-substrate.md.",
        violations.join("\n")
    );
}

/// `ironclaw_chain_signing` (PR6) is the ONE crate in the substrate that is
/// *allowed* to carry chain SDKs and secrets — it is the custodial signing
/// layer. This test is the inverse of the purity tests above: it asserts the
/// chain crate actually depends on at least one chain SDK and on
/// `ironclaw_secrets`, so a regression that accidentally moved chain/secret
/// code OUT of this crate (e.g. up into the pure attestation core) would be
/// caught from both directions.
#[test]
fn chain_signing_crate_carries_chain_sdk_and_secrets() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_chain_signing")
        .expect(
            "ironclaw_chain_signing must be a workspace member; add it to the root \
             Cargo.toml `workspace.members` (see attested-signing PR6)",
        );

    let dependencies = package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array");
    let dep_names: Vec<&str> = dependencies
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();

    // It must depend on ironclaw_secrets (custodial keys are secrets).
    assert!(
        dep_names.contains(&"ironclaw_secrets"),
        "ironclaw_chain_signing must depend on ironclaw_secrets (custodial keys are secrets); \
         deps: {dep_names:?}"
    );

    // It must depend on at least one chain SDK (the whole point of the crate).
    let chain_sdk_prefixes = ["alloy", "k256", "solana", "near-", "ed25519-dalek"];
    assert!(
        dep_names.iter().any(|name| chain_sdk_prefixes
            .iter()
            .any(|p| *name == *p || name.starts_with(p))),
        "ironclaw_chain_signing must carry a chain SDK / signing primitive; deps: {dep_names:?}"
    );

    // And it must build on the lower substrate crates.
    for required in ["ironclaw_signing_provider", "ironclaw_attestation"] {
        assert!(
            dep_names.contains(&required),
            "ironclaw_chain_signing must depend on {required}; deps: {dep_names:?}"
        );
    }
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

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}
