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

/// The dependency names the external-wallet crate (PR7 + PR9) must never carry.
/// It implements the browser injected provider (PR7) and the WalletConnect v2
/// provider (PR9) and holds NO key material, so it must not pull the heavy chain
/// SDKs (`solana-sdk` / `near-primitives`) nor the key-custody crate
/// (`ironclaw_secrets`) nor the chain-signing crate (`ironclaw_chain_signing`).
/// It IS allowed `k256` / `ed25519-dalek` / `sha3` / `sha2` for signer recovery
/// and ed25519 verification, the openssl-free WalletConnect relay fork
/// (`relay_client` / `relay_rpc`, PR9), plus `ironclaw_signing_provider` and
/// `ironclaw_attestation` — so those are deliberately absent from this list.
const WALLET_EXTERNAL_FORBIDDEN_DEPENDENCY_PREFIXES: &[&str] = &[
    "solana-sdk",
    "solana-program",
    "solana",
    "near-",
    "ironclaw_secrets",
    "ironclaw_chain_signing",
];

#[test]
fn wallet_external_crate_has_no_chain_sdk_secrets_or_chain_signing_dependency() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_wallet_external")
        .expect(
            "ironclaw_wallet_external must be a workspace member; add it to the root \
             Cargo.toml `workspace.members` (see attested-signing PR7)",
        );

    let dependencies = package["dependencies"]
        .as_array()
        .expect("package dependencies must be an array");

    let mut violations = Vec::new();
    for dependency in dependencies {
        let Some(name) = dependency["name"].as_str() else {
            continue;
        };
        for forbidden in WALLET_EXTERNAL_FORBIDDEN_DEPENDENCY_PREFIXES {
            if name == *forbidden || name.starts_with(forbidden) {
                let kind = dependency["kind"].as_str().unwrap_or("normal").to_string();
                violations.push(format!("{name} (kind: {kind}) matched `{forbidden}`"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "ironclaw_wallet_external is the external-wallet provider (PR7 + PR9) and holds no key \
         material; it must carry no chain SDK, secrets, or chain-signing dependency. Broadcasting \
         the wallet-signed tx (ironclaw_chain_signing) is deferred to PR10. Forbidden dependencies \
         found:\n{}\nSee docs/plans/2026-05-23-attested-signing-substrate.md.",
        violations.join("\n")
    );

    // Positive assertion (PR9): the openssl-free WalletConnect relay fork IS a
    // dependency of the external-wallet crate. This guards against the WC deps
    // being silently dropped (which would make the provider unbuildable) and
    // documents that `relay_client` / `relay_rpc` are the allowed transport.
    let names: Vec<&str> = dependencies
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();
    for expected in ["relay_client", "relay_rpc"] {
        assert!(
            names.contains(&expected),
            "ironclaw_wallet_external (PR9) must depend on `{expected}` from the openssl-free \
             WalletConnect fork (tracecommons/walletconnect-rs). Present dependencies: {names:?}"
        );
    }
}

/// The whole attested-signing substrate is deliberately openssl-free: every TLS
/// path uses rustls/ring so the workspace carries no OpenSSL C dependency (no
/// system-openssl build/runtime coupling, smaller attack surface, reproducible
/// cross-compilation). PR9 adds the WalletConnect relay client via the
/// openssl-free fork (`tracecommons/walletconnect-rs`, rustls default,
/// `relay_rpc`'s `cacao` feature DISABLED — `cacao` pulls `alloy 0.3.6 → reqwest
/// default-tls → openssl`). This test fails if anything in the workspace graph
/// (re)introduces `openssl-sys`, against the Linux target where native-tls would
/// otherwise resolve to openssl.
///
/// If this regresses after a dependency change, the fix is to disable the
/// offending crate's openssl/native-tls feature and select rustls — NOT to
/// silence this test.
#[test]
fn workspace_graph_is_openssl_free() {
    // `cargo tree -i <pkg>` exits non-zero with "did not match any packages"
    // when the package is absent from the graph — exactly the success case here.
    let manifest_path = workspace_root().join("Cargo.toml");
    let output = Command::new("cargo")
        .args([
            "tree",
            "--workspace",
            "-i",
            "openssl-sys",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--manifest-path",
        ])
        .arg(&manifest_path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo tree: {error}"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if output.status.success() {
        // A zero exit with non-empty output means openssl-sys IS in the graph.
        let listed = stdout.trim();
        assert!(
            listed.is_empty(),
            "the attested-signing workspace must stay openssl-free, but `openssl-sys` is in the \
             dependency graph (Linux target):\n{listed}\n\nThe likely culprit is a native-tls / \
             default-tls feature — most often `relay_rpc`'s `cacao` (alloy → reqwest default-tls). \
             Disable it and select rustls; do NOT silence this test. See \
             docs/plans/2026-05-23-attested-signing-substrate.md."
        );
    } else {
        // Non-zero exit: confirm it is the "no such package" case (openssl-sys
        // absent = success), not a cargo invocation failure.
        assert!(
            stderr.contains("did not match any packages"),
            "cargo tree failed unexpectedly while checking for openssl-sys:\nstdout: {stdout}\n\
             stderr: {stderr}"
        );
    }
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
