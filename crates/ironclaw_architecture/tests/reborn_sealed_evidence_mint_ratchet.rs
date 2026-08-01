//! PROPOSAL §11.2.5 — the sealed-evidence rule, as refute-tests.
//!
//! CHECKLIST WS1's evidence-mint row consolidated protocol-auth evidence
//! minting behind one witness-gated seam. This file is the *other half* of that
//! seal: every path the consolidation closed gets a test asserting it stays
//! closed. Each test below names the path it refutes.
//!
//! ## What the seal is, and why it needed replacing
//!
//! `ProtocolAuthEvidence::Verified` is the in-memory proof that host glue
//! authenticated an inbound request before it reached the product surface. It
//! used to be minted by eight `pub fn mark_*_verified*` free functions gated on
//! a `host-auth-mint` **cargo feature**.
//!
//! That gate was vacuous in every workspace build, and measurably so. Cargo
//! unifies features across the packages selected in one invocation, so
//! `ironclaw_webui`'s `ironclaw_product = { features = ["host-auth-mint"] }`
//! (→ `ironclaw_turns/host-auth-mint` → `ironclaw_host_api/host-auth-mint`)
//! compiled `ironclaw_host_api` **once, with the gate on**, for every other
//! crate in the same build. Measured on this row's base branch: a test in
//! `ironclaw_agent_loop` — a crate whose manifest names
//! `ironclaw_host_api` with *no* features at all — could call
//! `ironclaw_host_api::product_adapter::auth::mark_bearer_token_verified("attacker")`
//! and mint a verified bearer claim, and it compiled and passed as soon as
//! `ironclaw_webui` was named in the same `cargo test` (it correctly failed to
//! compile when `ironclaw_agent_loop` was built alone). A seal that any sibling
//! crate's manifest silently unlocks, workspace-wide, is not a seal.
//!
//! The replacement is the repo's existing witness-token idiom
//! (`ironclaw_host_api::authorized`, whose companion ratchet is
//! `reborn_authorized_seal_ratchet.rs`), which is feature-independent:
//!
//! - `HostAuthenticationGrant` / `VerifiedInboundGrant` are zero-sized witnesses
//!   whose fields are private to `ironclaw_host_api`. The only source of each is
//!   the provided body of `HostProtocolAuthenticator::host_authentication_grant`
//!   and `ChannelIngressVerifier::verified_inbound_grant` respectively.
//! - Every mint entry point consumes the matching grant, so minting requires
//!   *implementing* one of those traits — a greppable, reviewable line of code
//!   rather than a feature flag in someone else's manifest.
//! - Pure cross-crate type-sealing is not expressible in Rust (the grant is
//!   defined in `ironclaw_host_api`; the legitimate minters are two other
//!   crates), exactly as `authorized.rs` records for `CapabilityAuthorizer`. So
//!   these tests supply the other half: **only the sanctioned crate may
//!   implement each trait**, and no crate may re-open a second import path to
//!   the mint family.
//!
//! Two grants, not one, because the two minters are different trust roles: the
//! host authenticator (`ironclaw_webui`) attests bearer/session evidence, and
//! the generic channel ingress verifier (`ironclaw_extension_host`) attests
//! signature/shared-secret evidence. A single grant would let either forge the
//! other's claim shape.
//!
//! ## Residual, recorded rather than hidden
//!
//! `ProtocolAuthEvidence::seal_verified_inbound` takes an `AuthRequirement`, so
//! a holder of a `VerifiedInboundGrant` could attest a bearer-shaped
//! requirement. The grant holder is the generic ingress verifier — trusted host
//! code by charter — and narrowing the parameter would mean duplicating
//! `AuthRequirement`'s channel half as a second enum. Recorded as a bounded
//! residual, not silently passed: the crate-level seam that a *package* or a
//! *product handler* cannot mint at all is the property this row exists for.

// Each integration-test binary compiles the shared module independently; this
// binary uses only the comment/string stripper.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ratchet_support::{strip_comments_and_strings, workspace_root};

/// The retired cargo feature. It must not come back under any spelling that a
/// manifest, a CI script, or guidance could re-enable.
const RETIRED_FEATURE: &str = "host-auth-mint";

/// The sole crate permitted to implement `HostProtocolAuthenticator` — the
/// transport that performs bearer/session authentication (PROPOSAL §6.9.4,
/// trust stage T1).
const HOST_AUTHENTICATOR_CRATE: &str = "ironclaw_webui";

/// The sole crate permitted to implement `ChannelIngressVerifier` — the
/// vendor-blind ingress router/verifier (PROPOSAL §6.8.2, trust stage T2).
const CHANNEL_VERIFIER_CRATE: &str = "ironclaw_extension_host";

/// The crate that owns the evidence type and the bearer/session mint family
/// (PROPOSAL §6.1.1).
const EVIDENCE_TYPE_OWNER: &str = "ironclaw_host_api";

/// The crate that owns the channel/webhook mint family (PROPOSAL §6.1.2).
const CHANNEL_MINT_OWNER: &str = "ironclaw_extension_contracts";

/// The channel/webhook half of the mint family. Frozen by name: these are the
/// functions PROPOSAL §6.1.2 assigns to `ironclaw_extension_contracts`, so a
/// rename or deletion has to come here in the same change.
const CHANNEL_MINT_FNS: &[&str] = &[
    "mark_request_signature_verified",
    "mark_request_signature_verified_for_tenant",
    "mark_shared_secret_header_verified",
    "mark_shared_secret_header_verified_for_tenant",
];

/// The bearer/session half. PROPOSAL §6.1.1 keeps these in `ironclaw_host_api`.
const HOST_MINT_FNS: &[&str] = &[
    "mark_bearer_token_verified",
    "mark_bearer_token_verified_for_tenant",
    "mark_session_verified",
    "mark_session_verified_for_tenant",
];

/// This ratchet's own file, skipped so its own frozen-name tables and doc
/// examples do not read as offending call sites.
const SELF_FILE: &str = "reborn_sealed_evidence_mint_ratchet.rs";

fn all_mint_fns() -> Vec<&'static str> {
    CHANNEL_MINT_FNS
        .iter()
        .chain(HOST_MINT_FNS.iter())
        .copied()
        .collect()
}

/// Walk production Rust sources under `crates/`. Test trees are skipped for the
/// *call-site* rules (a test standing in for the host is the sanctioned
/// `ProtocolAuthEvidence::test_verified` seam, and test doubles are not
/// inventoried — the same convention `reborn_authorized_seal_ratchet` relies
/// on). The implementor rule below re-uses the same walk for the same reason.
fn collect_production_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "tests" | "examples" | "benches" | "target") {
                continue;
            }
            collect_production_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs")
            && path.file_name().and_then(|n| n.to_str()) != Some(SELF_FILE)
        {
            out.push(path);
        }
    }
}

fn owning_crate(root: &Path, path: &Path) -> String {
    path.strip_prefix(root.join("crates"))
        .ok()
        .and_then(|relative| relative.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn render(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every `Cargo.toml` in the workspace (crates, tools, and the root).
fn collect_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(name, "target" | "node_modules" | ".git") {
                continue;
            }
            collect_manifests(&path, out);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
            out.push(path);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Refutes: the `host-auth-mint` cargo feature (the vacuous seal it replaced).
// ─────────────────────────────────────────────────────────────────────────────

/// Closed path #1 — the feature itself, declared or enabled in any manifest.
///
/// This is the authoritative half: it reads every `Cargo.toml` in the tree, so
/// a re-declaration (`host-auth-mint = []`), a pass-through
/// (`host-auth-mint = ["ironclaw_host_api/host-auth-mint"]`), and a consumer
/// opt-in (`features = ["host-auth-mint"]`) all fail here.
#[test]
fn retired_host_auth_mint_feature_is_absent_from_every_manifest() {
    let root = workspace_root();
    let mut manifests = Vec::new();
    collect_manifests(&root, &mut manifests);
    assert!(
        manifests.len() > 50,
        "manifest walk found only {} Cargo.toml files — the walk is broken, not the workspace",
        manifests.len()
    );

    let mut offenders = Vec::new();
    for manifest in &manifests {
        let source = fs::read_to_string(manifest).unwrap_or_default();
        for (index, line) in source.lines().enumerate() {
            // Comments in a manifest are prose about the change, not a gate.
            if line.trim_start().starts_with('#') {
                continue;
            }
            if line.contains(RETIRED_FEATURE) {
                offenders.push(format!(
                    "{}:{}: {}",
                    render(&root, manifest),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "the `{RETIRED_FEATURE}` cargo feature is retired (PROPOSAL §11.2.5) and must not return. \
         It was never a seal: cargo unifies features across the packages in one build, so any \
         crate enabling it compiled the mint family open for the whole workspace. Minting is now \
         witness-gated — implement `HostProtocolAuthenticator` or `ChannelIngressVerifier` in the \
         one crate sanctioned for it. Offending manifest lines: {offenders:?}"
    );
}

/// Closed path #2 — the feature name surviving in CI scripts or workflow files,
/// where it would silently select a build configuration that no longer exists
/// (a `--features host-auth-mint` invocation now fails, but a *conditional* on
/// the name would go quietly dead instead).
#[test]
fn retired_host_auth_mint_feature_is_absent_from_ci_scripts() {
    let root = workspace_root();
    let mut scanned = 0usize;
    let mut offenders = Vec::new();

    for dir in ["scripts", ".github/workflows"] {
        let mut stack = vec![root.join(dir)];
        while let Some(current) = stack.pop() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Ok(source) = fs::read_to_string(&path) else {
                    continue;
                };
                scanned += 1;
                for (index, line) in source.lines().enumerate() {
                    if line.contains(RETIRED_FEATURE) {
                        offenders.push(format!(
                            "{}:{}: {}",
                            render(&root, &path),
                            index + 1,
                            line.trim()
                        ));
                    }
                }
            }
        }
    }

    assert!(
        scanned > 20,
        "scanned only {scanned} script/workflow files — the walk is broken, not the tree"
    );
    assert!(
        offenders.is_empty(),
        "the retired `{RETIRED_FEATURE}` feature still appears in CI tooling; a stale feature \
         selector goes dead silently rather than loudly. Offenders: {offenders:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Refutes: rogue grant sources (the witness half of the seal).
// ─────────────────────────────────────────────────────────────────────────────

/// Closed path #3 — any crate other than the host transport minting
/// bearer/session evidence by implementing the grant trait.
///
/// Mirrors `reborn_authorized_seal_ratchet::capability_authorizer_is_implemented_only_by_the_kernel`:
/// the grant's field is private to `ironclaw_host_api`, so the *only* way to
/// obtain one is this trait, and this test is what keeps the implementor set at
/// one.
#[test]
fn host_protocol_authenticator_is_implemented_only_by_the_host_transport() {
    assert_sole_implementor("HostProtocolAuthenticator", HOST_AUTHENTICATOR_CRATE);
}

/// Closed path #4 — any crate other than the generic ingress verifier minting
/// channel/webhook evidence. This is the property PROPOSAL §12.1a names: a
/// channel package may misreport parsed content but must never be able to forge
/// verification or scope.
#[test]
fn channel_ingress_verifier_is_implemented_only_by_the_generic_verifier() {
    assert_sole_implementor("ChannelIngressVerifier", CHANNEL_VERIFIER_CRATE);
}

fn assert_sole_implementor(trait_name: &str, permitted_crate: &str) {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_production_rs(&root.join("crates"), &mut files);
    assert!(
        files.len() > 500,
        "production walk found only {} files — the walk is broken, not the workspace",
        files.len()
    );

    let needle = format!("{trait_name} for");
    let mut offenders = Vec::new();
    let mut permitted_impls = 0usize;
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_default();
        // Comments and string literals are stripped, so doc mentions and error
        // text cannot false-positive, and a qualified or generic impl header
        // (`impl foo::Trait for`, `impl<T> Trait for`) still matches.
        for raw in strip_comments_and_strings(&source).lines() {
            let line = raw.trim();
            if !line.contains(&needle) {
                continue;
            }
            if owning_crate(&root, file) == permitted_crate {
                permitted_impls += 1;
            } else {
                offenders.push(format!("{}: {line}", render(&root, file)));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "`{trait_name}` may be implemented ONLY in `{permitted_crate}` — it is the sole source of \
         the grant that mints protocol-auth evidence (PROPOSAL §11.2.5). An implementation \
         anywhere else can forge a verified claim for a request nothing authenticated. Tests that \
         need verified evidence use `ProtocolAuthEvidence::test_verified` (the `test-support` \
         seam) instead. Offenders: {offenders:?}"
    );
    assert_eq!(
        permitted_impls, 1,
        "expected exactly one `{trait_name}` implementation, in `{permitted_crate}`; found \
         {permitted_impls}. Zero means the production minter lost its grant source (and the \
         scan now measures nothing); more than one means the trust role forked."
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Refutes: second import paths and off-seam call sites.
// ─────────────────────────────────────────────────────────────────────────────

/// Closed paths #5–#8 — the four re-export chains that gave the mint family
/// extra import paths before this row:
///
/// 1. `ironclaw_host_api::product_adapter` re-exported all eight from `auth`.
/// 2. `ironclaw_product` re-exported all eight at the crate root.
/// 3. `ironclaw_product::auth` re-exported all eight *again* — a second path out
///    of the same crate, which is how `ironclaw_extension_host` reached them.
/// 4. `ironclaw_reborn_composition` re-exported `mark_bearer_token_verified_for_tenant`
///    (with zero consumers) — an app-layer crate widening a security seam it did
///    not use.
///
/// §11.2.4's rule applied to the evidence family: one import path per mint
/// function, its owner's.
#[test]
fn no_crate_re_exports_a_mint_function_it_does_not_own() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_production_rs(&root.join("crates"), &mut files);

    let mint_fns = all_mint_fns();
    let mut offenders = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_default();
        for raw in strip_comments_and_strings(&source).lines() {
            let line = raw.trim();
            if !line.contains("pub use") {
                continue;
            }
            for name in &mint_fns {
                if !mentions_symbol(line, name) {
                    continue;
                }
                offenders.push(format!("{}: {line}", render(&root, file)));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "no crate may `pub use` a protocol-auth mint function (PROPOSAL §11.2.4/§11.2.5). A \
         second import path is how the mint family reached `ironclaw_extension_host` and \
         `ironclaw_webui` through `ironclaw_product` before this row, and it hides the seam from \
         the boundary rules. Import from the owner: bearer/session from \
         `{EVIDENCE_TYPE_OWNER}::product_adapter::auth`, channel/webhook from \
         `{CHANNEL_MINT_OWNER}::verified_inbound`. Offenders: {offenders:?}"
    );
}

/// Closed path #9 — production code outside the two sanctioned minters (and the
/// two owning contracts crates that define the family) naming a mint function
/// at all.
///
/// The witness makes an off-seam call *fail to compile* — this test makes it
/// fail in review, with an error message that says where the seam is, and
/// catches the case where someone adds a grant source and a call site in one
/// change.
#[test]
fn mint_functions_are_named_only_by_their_owners_and_sanctioned_minters() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_production_rs(&root.join("crates"), &mut files);

    let permitted: BTreeSet<&str> = [
        EVIDENCE_TYPE_OWNER,
        CHANNEL_MINT_OWNER,
        HOST_AUTHENTICATOR_CRATE,
        CHANNEL_VERIFIER_CRATE,
    ]
    .into_iter()
    .collect();

    let mint_fns = all_mint_fns();
    let mut offenders = Vec::new();
    let mut sighted = 0usize;
    for file in &files {
        let source = fs::read_to_string(file).unwrap_or_default();
        let owner = owning_crate(&root, file);
        for raw in strip_comments_and_strings(&source).lines() {
            let line = raw.trim();
            for name in &mint_fns {
                if !mentions_symbol(line, name) {
                    continue;
                }
                sighted += 1;
                if !permitted.contains(owner.as_str()) {
                    offenders.push(format!("{}: {line}", render(&root, file)));
                }
            }
        }
    }

    assert!(
        sighted > 0,
        "the mint-function scan matched nothing — the family was renamed without updating \
         CHANNEL_MINT_FNS/HOST_MINT_FNS, and this ratchet is now measuring an empty set"
    );
    assert!(
        offenders.is_empty(),
        "protocol-auth evidence may be minted only by the two sanctioned minters — \
         `{HOST_AUTHENTICATOR_CRATE}` (bearer/session, trust stage T1) and \
         `{CHANNEL_VERIFIER_CRATE}` (channel/webhook, trust stage T2) — plus the contracts crates \
         that define the family (`{EVIDENCE_TYPE_OWNER}`, `{CHANNEL_MINT_OWNER}`). A channel \
         package, a product handler, or composition minting its own evidence is the forgery \
         PROPOSAL §12.1a exists to prevent. Offenders: {offenders:?}"
    );
}

/// Closed path #10 — the mint family drifting out of the crate PROPOSAL §6
/// assigns it to. `#5`–`#9` police who *calls*; this polices where the
/// definitions live, so "consolidated" stays true.
#[test]
fn each_mint_half_is_defined_only_in_the_crate_that_owns_it() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_production_rs(&root.join("crates"), &mut files);

    let mut missing: Vec<&str> = Vec::new();
    let mut misplaced = Vec::new();

    for (family, owner) in [
        (CHANNEL_MINT_FNS, CHANNEL_MINT_OWNER),
        (HOST_MINT_FNS, EVIDENCE_TYPE_OWNER),
    ] {
        for name in family {
            let definition = format!("pub fn {name}(");
            let mut homes = Vec::new();
            for file in &files {
                let source = fs::read_to_string(file).unwrap_or_default();
                if strip_comments_and_strings(&source).contains(&definition) {
                    homes.push(file.clone());
                }
            }
            if homes.is_empty() {
                missing.push(name);
                continue;
            }
            for home in homes {
                if owning_crate(&root, &home) != owner {
                    misplaced.push(format!(
                        "{name} is defined in {} but PROPOSAL §6 assigns it to {owner}",
                        render(&root, &home)
                    ));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "these mint functions no longer exist anywhere: {missing:?}. If the family was \
         deliberately narrowed, update CHANNEL_MINT_FNS/HOST_MINT_FNS in the same change — \
         otherwise this ratchet silently stops governing them."
    );
    assert!(
        misplaced.is_empty(),
        "the mint family must stay split by trust role: channel/webhook in {CHANNEL_MINT_OWNER} \
         (§6.1.2), bearer/session in {EVIDENCE_TYPE_OWNER} (§6.1.1).\n{}",
        misplaced.join("\n")
    );
}

/// Word-boundary containment: `mark_session_verified` must not match inside
/// `mark_session_verified_for_tenant` when the caller asked for the base name,
/// and a substring of an unrelated identifier must not match at all.
fn mentions_symbol(line: &str, symbol: &str) -> bool {
    let mut start = 0usize;
    while let Some(offset) = line[start..].find(symbol) {
        let at = start + offset;
        let before_ok = at == 0 || !is_ident_char(line.as_bytes()[at - 1] as char);
        let after = at + symbol.len();
        let after_ok = after >= line.len() || !is_ident_char(line.as_bytes()[after] as char);
        if before_ok && after_ok {
            return true;
        }
        start = at + symbol.len();
    }
    false
}

fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

// ─────────────────────────────────────────────────────────────────────────────
// Self-tests: every scan above must be able to fail (zero-match principle).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn symbol_matcher_respects_word_boundaries() {
    assert!(mentions_symbol(
        "use x::mark_session_verified;",
        "mark_session_verified"
    ));
    assert!(mentions_symbol(
        "    mark_session_verified(a, b)",
        "mark_session_verified"
    ));
    // The `_for_tenant` variant is a different function; asking for the base
    // name must not match it, or every long-name offender is double-reported
    // and a base-name deletion looks governed when it is not.
    assert!(!mentions_symbol(
        "mark_session_verified_for_tenant(a)",
        "mark_session_verified"
    ));
    assert!(!mentions_symbol(
        "let x = premark_session_verifiedly;",
        "mark_session_verified"
    ));
}

#[test]
fn implementor_matcher_detects_a_rogue_impl_and_ignores_prose() {
    let rogue = strip_comments_and_strings("impl ChannelIngressVerifier for RogueAdapter {}");
    assert!(rogue.contains("ChannelIngressVerifier for"));

    // Generic and qualified impl headers must still match ...
    for head in [
        "impl<T> ChannelIngressVerifier for Wrapper<T> {}",
        "impl ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier for X {}",
    ] {
        assert!(
            strip_comments_and_strings(head).contains("ChannelIngressVerifier for"),
            "matcher must detect: {head}"
        );
    }
    // ... while prose and the trait declaration must not.
    for benign in [
        "// impl ChannelIngressVerifier for Foo (in docs)",
        "/// See ChannelIngressVerifier for details",
        "let msg = \"impl ChannelIngressVerifier for X\";",
    ] {
        assert!(
            !strip_comments_and_strings(benign).contains("ChannelIngressVerifier for"),
            "matcher must not fire on: {benign}"
        );
    }
    assert!(
        !strip_comments_and_strings("pub trait ChannelIngressVerifier {")
            .contains("ChannelIngressVerifier for")
    );
}

#[test]
fn manifest_and_script_walks_reach_the_files_they_claim_to() {
    let root = workspace_root();
    let mut manifests = Vec::new();
    collect_manifests(&root, &mut manifests);
    assert!(
        manifests
            .iter()
            .any(|path| path.ends_with("crates/ironclaw_host_api/Cargo.toml")),
        "the manifest walk must reach the crate that owns the evidence type"
    );
    assert!(
        manifests
            .iter()
            .any(|path| path == &root.join("Cargo.toml")),
        "the manifest walk must reach the workspace root manifest"
    );
    assert!(
        root.join("scripts/ci/package-feature-flags.sh").is_file(),
        "the CI feature-flag recipe must exist for the script scan to be meaningful"
    );
}
