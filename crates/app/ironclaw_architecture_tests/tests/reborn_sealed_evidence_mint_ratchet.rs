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
//! `ironclaw_webui`'s `ironclaw_assistant = { features = ["host-auth-mint"] }`
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

use ratchet_support::{crate_path, strip_comments_and_strings, workspace_root};

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

/// Read a file the walk already found, failing loudly on an I/O error.
///
/// `read_to_string(..).unwrap_or_default()` was the shape here, and it is
/// fail-open by construction: a file the census cannot read contributes no impl
/// headers and no offenders, so an unreadable source scans exactly like a clean
/// one and the gate reports success on an incomplete census. These traits are
/// unsealed with *provided* mint methods, so this census IS the enforcement
/// (CHECKLIST WS0, #6963).
fn read_source(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| {
        panic!(
            "failed to read {} — a source this census cannot read must fail the gate, \
             not scan as empty: {error}",
            path.display()
        )
    })
}

/// Walk production Rust sources under a member root. Test trees are skipped for
/// the *call-site* rules (a test standing in for the host is the sanctioned
/// `ProtocolAuthEvidence::test_verified` seam, and test doubles are not
/// inventoried — the same convention `reborn_authorized_seal_ratchet` relies
/// on). The implementor rule below re-uses the same walk for the same reason.
///
/// An unreadable directory or entry fails: a subtree that silently vanishes
/// from the walk is the same fail-open as an unreadable file.
fn collect_production_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| {
        panic!(
            "failed to read {} — a directory this census cannot read must fail the gate, \
             not vanish from the walk: {error}",
            dir.display()
        )
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read an entry in {}: {error}", dir.display())
        });
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // `node_modules` holds vendored frontend packages after a pnpm
            // install; nothing there is workspace source.
            if matches!(
                name,
                "tests" | "examples" | "benches" | "target" | "node_modules"
            ) {
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

/// Every top-level directory the root manifest's `[workspace] members` list
/// names — `crates/` plus `tools/` today.
///
/// The census used to walk `crates/` alone. `tools/ironclaw_stress` is a
/// workspace member that depends on `ironclaw_host_api`, so it can implement a
/// witness trait and mint `ProtocolAuthEvidence::Verified` exactly as a crate
/// can — and it sat outside every scan while the `> 500` file floor stayed
/// comfortably cleared. Derived from the manifest rather than hardcoded so a new
/// member root joins the census automatically.
fn member_roots(root: &Path) -> Vec<PathBuf> {
    let manifest = read_source(&root.join("Cargo.toml"));
    let parsed: toml::Value = toml::from_str(&manifest)
        .unwrap_or_else(|error| panic!("root Cargo.toml does not parse: {error}"));
    let members = parsed
        .get("workspace")
        .and_then(|workspace| workspace.get("members"))
        .and_then(|members| members.as_array())
        .unwrap_or_else(|| {
            panic!(
                "root Cargo.toml declares no `[workspace] members` — this census resolves \
                   its scan roots from that list, and cannot guess them"
            )
        });

    let mut names = BTreeSet::new();
    for member in members {
        let Some(text) = member.as_str() else {
            continue;
        };
        let Some(top) = text.split('/').next() else {
            continue;
        };
        // `"."` is the workspace root package, which owns no `src/` tree.
        if top.is_empty() || top == "." {
            continue;
        }
        names.insert(top.to_string());
    }

    assert!(
        names.contains("crates"),
        "the member list no longer names `crates/`, so this census would scan the wrong \
         tree entirely: {names:?}"
    );
    let mut roots = Vec::new();
    for name in names {
        let path = root.join(&name);
        assert!(
            path.is_dir(),
            "workspace member root {} does not exist — repoint the members list rather than \
             letting the census scan nothing",
            path.display()
        );
        roots.push(path);
    }
    roots
}

/// Every production Rust source across every workspace member root.
fn collect_workspace_production_rs(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for member_root in member_roots(root) {
        collect_production_rs(&member_root, &mut files);
    }
    files
}

/// The crate that owns `path`, by crate-directory basename.
///
/// Resolved through the crate inventory rather than by taking the first
/// component under `crates/`. That older idiom answers the FAMILY name once a
/// crate moves (`substrates`, not `ironclaw_webui`), and already answers
/// `extensions` for the two crates nested one level down today — so this
/// security-critical census would attribute a mint site to the wrong owner
/// (CHECKLIST WS10).
fn owning_crate(root: &Path, path: &Path) -> String {
    ratchet_support::owning_crate_name(root, path)
}

fn render(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Every `Cargo.toml` in the workspace (crates, tools, and the root).
fn collect_manifests(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| {
        panic!(
            "failed to read {} — an unreadable directory must fail this walk, not silently \
             shrink it: {error}",
            dir.display()
        )
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("failed to read an entry in {}: {error}", dir.display())
        });
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
        let source = read_source(manifest);
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
            let entries = fs::read_dir(&current).unwrap_or_else(|error| {
                panic!(
                    "failed to read {} — an unreadable directory must fail this scan, not \
                     silently shrink it: {error}",
                    current.display()
                )
            });
            for entry in entries {
                let entry = entry.unwrap_or_else(|error| {
                    panic!("failed to read an entry in {}: {error}", current.display())
                });
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                // Non-UTF-8 files (compiled fixtures, images) are not CI recipes
                // and are the one legitimate skip here; an I/O *failure* is not.
                let source = match fs::read_to_string(&path) {
                    Ok(source) => source,
                    Err(error) if error.kind() == std::io::ErrorKind::InvalidData => continue,
                    Err(error) => panic!(
                        "failed to read {} — an unreadable CI recipe must fail this scan, not \
                         scan as empty: {error}",
                        path.display()
                    ),
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
    let files = collect_workspace_production_rs(&root);
    assert!(
        files.len() > 500,
        "production walk found only {} files — the walk is broken, not the workspace",
        files.len()
    );

    let mut offenders = Vec::new();
    let mut permitted_impls = 0usize;
    let mut headers_seen = 0usize;
    for file in &files {
        let source = read_source(file);
        // Comments and string literals are stripped, so doc mentions and error
        // text cannot false-positive. Headers are then collapsed onto one line
        // and in-file `use … as …` aliases resolved, so neither a line break
        // nor a rename at the import can hide an implementation.
        let stripped = strip_comments_and_strings(&source);
        let names = implementor_names(&stripped, trait_name);
        for header in impl_headers(&stripped) {
            headers_seen += 1;
            if !header_implements(&header.text, &names) {
                continue;
            }
            if owning_crate(&root, file) == permitted_crate {
                permitted_impls += 1;
            } else {
                offenders.push(format!(
                    "{}:{}: {}",
                    render(&root, file),
                    header.line,
                    header.text
                ));
            }
        }
    }

    // The scan is only worth its verdict if the header extractor still parses
    // the workspace. A normalizer that silently stopped producing headers would
    // make every census read "no offenders" — the exact fail-open shape this
    // gate exists to refute (#6963's zero-match principle).
    assert!(
        headers_seen > 1000,
        "the impl-header scan found only {headers_seen} headers across {} production files — the \
         extractor is broken, not the workspace. A census that parses nothing reports every trait \
         as unimplemented, so this must fail loudly rather than pass.",
        files.len()
    );
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
// Implementor census: header extraction, alias resolution, matching.
//
// These traits are deliberately NOT sealed to `ironclaw_host_api` (sealing
// would lock out the one crate that legitimately implements each), and their
// mint methods are *provided*, so `impl Trait for X {}` alone confers minting.
// This census is therefore the enforcement, not a convenience check — every
// shape it cannot see is a way to mint forged evidence.
// ─────────────────────────────────────────────────────────────────────────────

/// One `impl … for …` header, collapsed onto a single line.
struct ImplHeader {
    /// 1-based line of the `impl` keyword, for the offender report.
    line: usize,
    text: String,
}

/// Every `impl` header in already-stripped `source`, whitespace-normalized.
///
/// Line-oriented matching is what let a rogue impl hide behind a line break:
/// `impl ChannelIngressVerifier\n    for RogueAdapter` never puts the trait
/// name and `for` on one line, so a `contains("<Trait> for")` scan over
/// `.lines()` read it as absent. Collapsing the header first removes that
/// formatting degree of freedom entirely.
///
/// The header region runs from the `impl` keyword to the first `{`, `;`, or
/// `where`, or — at angle-bracket depth zero — `,` or `)`. The last two stop an
/// `impl Trait` in argument position from swallowing a function body; the depth
/// check keeps `impl T for HashMap<K, V>` intact. `impl` in return position
/// (`-> impl Iterator<…>`) yields a header with no `for`, which matches nothing.
fn impl_headers(source: &str) -> Vec<ImplHeader> {
    let chars: Vec<char> = source.chars().collect();
    let mut headers = Vec::new();
    let mut line = 1usize;
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if !starts_word(&chars, index, "impl") {
            index += 1;
            continue;
        }
        let start_line = line;
        let mut cursor = index + "impl".len();
        let mut depth = 0usize;
        let mut text = String::from("impl");
        while cursor < chars.len() {
            let ch = chars[cursor];
            // `->` is an arrow, not a closing angle bracket. Without this a
            // return type inside generics (`Box<dyn Fn() -> u8>`) would close
            // the bracket early and truncate the header.
            if ch == '-' && chars.get(cursor + 1) == Some(&'>') {
                text.push_str("->");
                cursor += 2;
                continue;
            }
            match ch {
                '{' | ';' => break,
                '<' => depth += 1,
                '>' => depth = depth.saturating_sub(1),
                ',' | ')' if depth == 0 => break,
                '\n' => line += 1,
                _ => {}
            }
            if depth == 0 && starts_word(&chars, cursor, "where") {
                break;
            }
            text.push(ch);
            cursor += 1;
        }
        headers.push(ImplHeader {
            line: start_line,
            text: text.split_whitespace().collect::<Vec<_>>().join(" "),
        });
        index = cursor;
    }
    headers
}

/// The canonical trait name plus every name bound to it by a `use … as …` in
/// this file.
///
/// `use path::ChannelIngressVerifier as Verifier; impl Verifier for Rogue {}`
/// never names the trait on the impl line, so a census keyed only to the
/// canonical spelling cannot see it. Braced groups
/// (`use path::{Trait as V, Other}`) and raw-identifier aliases (`as r#type`)
/// bind the same way and are resolved the same way. Cross-file re-export
/// chains (`pub use … as …` in crate A, imported by crate B) stay out of reach
/// of a per-file census — recorded as residual, not closed.
fn implementor_names(source: &str, trait_name: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    names.insert(trait_name.to_string());
    let chars: Vec<char> = source.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if !starts_word(&chars, index, trait_name) {
            index += 1;
            continue;
        }
        let mut cursor = index + trait_name.chars().count();
        // ` as <ident>` — the only construct that can rename a trait in scope.
        // A cast (`x as u64`) cannot apply to a trait name, so no disambiguation
        // beyond the word boundary is needed.
        while chars.get(cursor).is_some_and(|c| c.is_whitespace()) {
            cursor += 1;
        }
        if starts_word(&chars, cursor, "as") {
            cursor += 2;
            while chars.get(cursor).is_some_and(|c| c.is_whitespace()) {
                cursor += 1;
            }
            if chars.get(cursor) == Some(&'r') && chars.get(cursor + 1) == Some(&'#') {
                cursor += 2;
            }
            let alias: String = chars[cursor..]
                .iter()
                .take_while(|c| is_ident_char(**c))
                .collect();
            if !alias.is_empty() {
                names.insert(alias);
            }
        }
        index += trait_name.chars().count();
    }
    names
}

/// Whether a collapsed header implements one of `names`.
///
/// Requires the name to appear on an identifier boundary — so a longer trait
/// that merely ends in the same text (`MyChannelIngressVerifier`) is not a hit
/// — followed, after any generic arguments, by the `for` keyword. Qualified
/// paths (`a::b::Trait for`) and generic impls (`impl<T> Trait<T> for`) still
/// match, which is what the pre-hardening scan did.
fn header_implements(header: &str, names: &BTreeSet<String>) -> bool {
    let chars: Vec<char> = header.chars().collect();
    for name in names {
        let mut index = 0usize;
        while index < chars.len() {
            if !starts_word(&chars, index, name) {
                index += 1;
                continue;
            }
            let mut cursor = index + name.chars().count();
            // Whitespace may separate the name from its generic arguments —
            // `impl Trait <> for X {}` compiles, and the header collapse *makes*
            // that space whenever a line break falls between the two. Skipping
            // it before looking for `<` is what keeps the normalization total;
            // without it the generic-argument branch is never entered, the
            // whitespace loop below lands on `<`, `for` is not found, and the
            // impl mints evidence undetected.
            while chars.get(cursor).is_some_and(|c| c.is_whitespace()) {
                cursor += 1;
            }
            // Skip the trait's own generic arguments: `Trait<T> for X`.
            if chars.get(cursor) == Some(&'<') {
                let mut depth = 0usize;
                while cursor < chars.len() {
                    // `->` inside the arguments is an arrow, not a closer.
                    if chars[cursor] == '-' && chars.get(cursor + 1) == Some(&'>') {
                        cursor += 2;
                        continue;
                    }
                    match chars[cursor] {
                        '<' => depth += 1,
                        '>' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                cursor += 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
            }
            while chars.get(cursor).is_some_and(|c| c.is_whitespace()) {
                cursor += 1;
            }
            if starts_word(&chars, cursor, "for") {
                return true;
            }
            index += name.chars().count();
        }
    }
    false
}

/// Whether `word` starts at `index` on both-side identifier boundaries.
fn starts_word(chars: &[char], index: usize, word: &str) -> bool {
    if index > 0 && is_ident_char(chars[index - 1]) {
        return false;
    }
    let mut cursor = index;
    for expected in word.chars() {
        if chars.get(cursor) != Some(&expected) {
            return false;
        }
        cursor += 1;
    }
    !chars.get(cursor).is_some_and(|c| is_ident_char(*c))
}

/// Every `pub`-visible `use` **item** in already-stripped `source` that names one
/// of the two witness traits, as `(1-based line of the visibility, collapsed
/// item text)`.
///
/// Item-based, not line-based, for the same reason `impl_headers` is: rustfmt
/// splits a long braced import, and the split puts `pub` and the trait name on
/// different physical lines —
///
/// ```text
/// pub(crate) use ironclaw_host_api::product_adapter::auth::{
///     ChannelIngressVerifier as V,
/// };
/// ```
///
/// — so a `.lines()` scan sees a first line with no trait name and a second line
/// that does not start with `pub`, returns `false` for both, and the two-file
/// alias blind spot this guard exists to close reopens. Items run to their
/// terminating `;` (a `use` tree contains none), so brace depth needs no
/// tracking.
///
/// `pub`-visible covers `pub use`, `pub(crate) use`, `pub(super) use`, and
/// `pub(in path) use` — every visibility that reaches beyond the declaring file,
/// which is exactly what would put an alias binding out of the census's reach.
fn grant_trait_reexports(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if !starts_word(&chars, index, "use") {
            index += 1;
            continue;
        }
        let Some(visibility) = preceding_pub_visibility(&chars, index) else {
            index += "use".len();
            continue;
        };
        let item_line = line
            - chars[visibility..index]
                .iter()
                .filter(|c| **c == '\n')
                .count();
        let mut cursor = index;
        while cursor < chars.len() && chars[cursor] != ';' {
            if chars[cursor] == '\n' {
                line += 1;
            }
            cursor += 1;
        }
        let item: String = chars[visibility..cursor].iter().collect();
        let collapsed = item.split_whitespace().collect::<Vec<_>>().join(" ");
        if ["HostProtocolAuthenticator", "ChannelIngressVerifier"]
            .iter()
            .any(|trait_name| mentions_symbol(&collapsed, trait_name))
        {
            out.push((item_line, collapsed));
        }
        index = cursor;
    }
    out
}

/// Where the `pub` visibility preceding the `use` keyword at `index` starts, if
/// there is one. Handles the restricted forms by matching the parenthesized
/// qualifier backwards: `pub(crate)`, `pub(super)`, `pub(in crate::m)`.
fn preceding_pub_visibility(chars: &[char], index: usize) -> Option<usize> {
    let mut cursor = index;
    while cursor > 0 && chars[cursor - 1].is_whitespace() {
        cursor -= 1;
    }
    if cursor > 0 && chars[cursor - 1] == ')' {
        let mut depth = 0usize;
        while cursor > 0 {
            cursor -= 1;
            match chars[cursor] {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
        while cursor > 0 && chars[cursor - 1].is_whitespace() {
            cursor -= 1;
        }
    }
    let start = cursor.checked_sub("pub".len())?;
    (starts_word(chars, start, "pub")).then_some(start)
}

/// The re-export census for one file, as the production scan runs it.
fn file_reexports(source: &str) -> Vec<(usize, String)> {
    grant_trait_reexports(&strip_comments_and_strings(source))
}

/// The full census for one file, as the production scan runs it. Self-tests
/// drive this rather than reimplementing the pipeline, so a test cannot pass
/// against logic the gate does not use.
fn file_implements(source: &str, trait_name: &str) -> bool {
    let stripped = strip_comments_and_strings(source);
    let names = implementor_names(&stripped, trait_name);
    impl_headers(&stripped)
        .iter()
        .any(|header| header_implements(&header.text, &names))
}

// ─────────────────────────────────────────────────────────────────────────────
// Refutes: second import paths and off-seam call sites.
// ─────────────────────────────────────────────────────────────────────────────

/// Closed paths #5–#8 — the four re-export chains that gave the mint family
/// extra import paths before this row:
///
/// 1. `ironclaw_host_api::product_adapter` re-exported all eight from `auth`.
/// 2. `ironclaw_assistant` re-exported all eight at the crate root.
/// 3. `ironclaw_assistant::auth` re-exported all eight *again* — a second path out
///    of the same crate, which is how `ironclaw_extension_host` reached them.
/// 4. `ironclaw_composition` re-exported `mark_bearer_token_verified_for_tenant`
///    (with zero consumers) — an app-layer crate widening a security seam it did
///    not use.
///
/// §11.2.4's rule applied to the evidence family: one import path per mint
/// function, its owner's.
#[test]
fn no_crate_re_exports_a_mint_function_it_does_not_own() {
    let root = workspace_root();
    let files = collect_workspace_production_rs(&root);

    let mint_fns = all_mint_fns();
    let mut offenders = Vec::new();
    for file in &files {
        let source = read_source(file);
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
         `ironclaw_webui` through `ironclaw_assistant` before this row, and it hides the seam from \
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
    let files = collect_workspace_production_rs(&root);

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
        let source = read_source(file);
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

/// Closes the implementor census's one structural blind spot: an alias
/// introduced in a *different* file from the `impl` that uses it.
///
/// `implementor_names` resolves `use … as …` within the file it scans, which
/// covers every shape a single file can express. What it cannot see is a
/// two-file split — `pub(crate) use …::ChannelIngressVerifier as V;` in one
/// module, `use crate::m::V; impl V for Rogue {}` in another — because neither
/// file alone names both the trait and the impl. Forbidding any `pub`-visible
/// re-export of the two traits removes that second file: outside the owning
/// crate the only way to bring either trait into scope is a private `use` in
/// the same file as the impl, which the census does see.
///
/// Same rule §11.2.4 already applies to the mint functions
/// (`no_crate_re_exports_a_mint_function_it_does_not_own`), applied to the
/// witness traits that gate them.
#[test]
fn no_crate_re_exports_the_grant_traits() {
    let root = workspace_root();
    let files = collect_workspace_production_rs(&root);
    assert!(
        files.len() > 500,
        "production walk found only {} files — the walk is broken, not the workspace",
        files.len()
    );

    let mut offenders = Vec::new();
    for file in &files {
        if owning_crate(&root, file) == EVIDENCE_TYPE_OWNER {
            continue;
        }
        for (line, item) in file_reexports(&read_source(file)) {
            offenders.push(format!("{}:{line}: {item}", render(&root, file)));
        }
    }

    assert!(
        offenders.is_empty(),
        "`HostProtocolAuthenticator` and `ChannelIngressVerifier` are witness traits: implementing \
         either mints protocol-auth evidence, so they have exactly one import path — \
         `{EVIDENCE_TYPE_OWNER}`'s. A `pub`-visible re-export creates a second name for the trait \
         in another crate, which also puts an alias binding in a different file from the `impl` \
         that uses it — outside what the implementor census can attribute. Import the trait \
         directly where it is implemented. Offenders: {offenders:?}"
    );
}

/// Closed path #10 — the mint family drifting out of the crate PROPOSAL §6
/// assigns it to. `#5`–`#9` police who *calls*; this polices where the
/// definitions live, so "consolidated" stays true.
#[test]
fn each_mint_half_is_defined_only_in_the_crate_that_owns_it() {
    let root = workspace_root();
    let files = collect_workspace_production_rs(&root);

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
                let source = read_source(file);
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

/// Both traits are unsealed with *provided* mint methods, so `impl Trait for X
/// {}` alone confers minting and this census IS the enforcement. Every shape it
/// cannot see is a way to forge verified evidence, so each is pinned here.
/// Driven through `file_implements` — the same pipeline the gate runs — so a
/// case cannot pass against logic production does not use.
#[test]
fn implementor_matcher_detects_a_rogue_impl_and_ignores_prose() {
    for trait_name in ["ChannelIngressVerifier", "HostProtocolAuthenticator"] {
        for rogue in [
            // Baseline: the shape the pre-hardening scan already caught.
            format!("impl {trait_name} for RogueAdapter {{}}"),
            format!("impl<T> {trait_name} for Wrapper<T> {{}}"),
            format!("impl ironclaw_host_api::product_adapter::auth::{trait_name} for X {{}}"),
            format!("unsafe impl {trait_name} for RogueAdapter {{}}"),
            // Fail-open #1: the header split across lines, so the trait name
            // and `for` never share a line.
            format!("impl {trait_name}\n    for RogueAdapter\n{{\n}}"),
            format!("impl<T>\n    {trait_name}\n    for Wrapper<T>\n{{\n}}"),
            format!("impl\n    {trait_name}\n    for RogueAdapter {{}}"),
            // Fail-open #2: renamed at the import, so the canonical spelling
            // never appears on the impl at all.
            format!("use a::b::{trait_name} as Verifier;\nimpl Verifier for RogueAdapter {{}}"),
            format!("use a::b::{{{trait_name} as V, Other}};\nimpl V for RogueAdapter {{}}"),
            format!("use a::{{b::{{{trait_name} as V}}}};\nimpl V for RogueAdapter {{}}"),
            format!("use a::b::{trait_name} as r#type;\nimpl r#type for RogueAdapter {{}}"),
            // Fail-open #3: whitespace between the trait name and its generic
            // argument list. `impl Trait <> for X {}` compiles today (verified
            // against rustc: empty angle brackets with a leading space are
            // accepted on a non-generic trait), and the header collapse *makes*
            // that space whenever a line break falls there — so a matcher that
            // only looks for `<` immediately after the name is evadable by
            // reformatting alone.
            format!("impl {trait_name} <> for RogueAdapter {{}}"),
            format!("impl {trait_name}\n    <>\n    for RogueAdapter {{}}"),
            format!("impl<T> {trait_name} <T> for Wrapper<T> {{}}"),
            // Both at once.
            format!("use a::b::{trait_name} as V;\nimpl V\n    for RogueAdapter {{}}"),
            // A macro body still spells the header out, so it is visible.
            format!(
                "macro_rules! mint {{\n    ($t:ty) => {{\n        impl {trait_name} for $t {{}}\n    }};\n}}"
            ),
        ] {
            assert!(
                file_implements(&rogue, trait_name),
                "census missed a minting implementation — it would forge verified evidence \
                 undetected:\n{rogue}"
            );
        }

        // Prose, string literals, the declaration, and near-miss names must not
        // fire: a census that flags everything is as useless as one that flags
        // nothing, and a false positive here blocks the seam's real owner.
        for benign in [
            format!("// impl {trait_name} for Foo (in docs)"),
            format!("/// See {trait_name} for details"),
            format!("let msg = \"impl {trait_name} for X\";"),
            format!("pub trait {trait_name} {{"),
            format!("fn f(x: &dyn {trait_name}) {{}}"),
            // Longer name that merely ends in the governed one.
            format!("impl My{trait_name} for X {{}}"),
            // Inherent impl on a type whose name contains the trait's.
            format!("impl {trait_name}Registry {{}}"),
            // The alias binds a *different* trait, so `V` is not governed here.
            format!("use a::b::Other as V;\nimpl V for X {{}}\n// {trait_name}"),
        ] {
            assert!(
                !file_implements(&benign, trait_name),
                "census fired on benign source, which would block the seam's own owner:\n{benign}"
            );
        }
    }
}

/// The normalizer is now load-bearing: if `impl_headers` stopped producing
/// headers the census would report every trait as unimplemented and pass. This
/// pins that it parses real, multi-shape source rather than degrading quietly —
/// the same zero-match principle as the `headers_seen` floor in the gate.
#[test]
fn impl_header_extraction_cannot_silently_degrade() {
    let source = "\
impl Alpha for One {}
unsafe impl Beta
    for Two
{
}
impl<T> Gamma<T> for Three<T>
where
    T: Clone,
{
}
impl Delta for HashMap<K, V> {}
fn produces() -> impl Iterator<Item = u8> { core::iter::empty() }
fn consumes(x: impl Into<String>, y: u8) {}
impl Epsilon for Box<dyn Fn() -> u8> {}
";
    let headers = impl_headers(&strip_comments_and_strings(source));
    let texts: Vec<&str> = headers.iter().map(|h| h.text.as_str()).collect();

    // Every real header is collapsed onto one line, generics and all.
    for expected in [
        "impl Alpha for One",
        "impl Beta for Two",
        "impl<T> Gamma<T> for Three<T>",
        "impl Delta for HashMap<K, V>",
        "impl Epsilon for Box<dyn Fn() -> u8>",
    ] {
        assert!(
            texts.contains(&expected),
            "header extractor lost {expected:?}; got {texts:?}"
        );
    }
    // `impl Trait` in return and argument position yields no `… for …`, so it
    // cannot be mistaken for an implementation.
    assert!(
        !texts
            .iter()
            .any(|text| text.contains("Iterator<Item = u8> for"))
    );
    assert!(!texts.iter().any(|text| text.contains("Into<String> for")));

    // Line numbers point at the `impl` keyword, so offenders are actionable.
    let beta = headers
        .iter()
        .find(|header| header.text == "impl Beta for Two")
        .expect("multi-line header extracted");
    assert_eq!(
        beta.line, 2,
        "offender report must name the impl's own line"
    );
}

/// The re-export guard is what removes the census's two-file blind spot, so it
/// has to fire on every visibility that can create a second file. Today's tree
/// contains no re-export at all, so the guard's decision is pinned directly.
#[test]
fn reexport_guard_catches_every_escaping_visibility() {
    for offending in [
        "pub use ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier;",
        "pub use ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier as V;",
        "pub(crate) use a::b::ChannelIngressVerifier as V;",
        "pub(super) use a::b::HostProtocolAuthenticator as A;",
        "pub(in crate::m) use a::b::HostProtocolAuthenticator;",
        "pub use a::b::{ChannelIngressVerifier as V, Other};",
        // rustfmt's own output for a long braced import: `pub` and the trait
        // name never share a physical line, which is what made the line-based
        // guard fail open.
        "pub(crate) use ironclaw_host_api::product_adapter::auth::{\n    ChannelIngressVerifier as V,\n};",
        "pub use a::b::{\n    HostProtocolAuthenticator,\n    Other,\n};",
        "pub\n    use a::b::ChannelIngressVerifier;",
        "pub(in crate::m)\nuse a::b::{\n    HostProtocolAuthenticator as A,\n};",
    ] {
        assert!(
            !file_reexports(offending).is_empty(),
            "re-export guard missed a second import path: {offending}"
        );
    }

    for benign in [
        // A private import in the implementing file is the sanctioned shape —
        // the census resolves its aliases, so it must not be flagged.
        "use ironclaw_host_api::product_adapter::auth::ChannelIngressVerifier;",
        "use a::b::ChannelIngressVerifier as V;",
        "use a::b::{\n    ChannelIngressVerifier as V,\n};",
        // Re-exporting something else entirely.
        "pub use a::b::AuthRequirement;",
        // A near-miss name is a different type.
        "pub use a::b::ChannelIngressVerifierRegistry;",
        // Not a use statement at all.
        "pub struct ChannelIngressVerifierRegistry;",
        // Prose and string literals are stripped before the scan.
        "// pub use a::b::ChannelIngressVerifier;",
        "let msg = \"pub use a::b::ChannelIngressVerifier;\";",
        // A `use` inside a function body is private no matter what precedes it.
        "pub fn f() {\n    use a::b::ChannelIngressVerifier;\n}",
    ] {
        assert!(
            file_reexports(benign).is_empty(),
            "re-export guard fired on a sanctioned shape: {benign}"
        );
    }

    // The offender report must name the item's own first line, not the line the
    // trait happens to land on.
    let split = "fn a() {}\n\npub(crate) use a::b::{\n    ChannelIngressVerifier as V,\n};";
    assert_eq!(
        file_reexports(split),
        vec![(
            3,
            "pub(crate) use a::b::{ ChannelIngressVerifier as V, }".to_string()
        )],
        "the offender must be reported at the visibility's line, collapsed onto one"
    );
}

/// Alias resolution is scoped to the file that declares the binding, and only
/// binds the trait actually renamed.
#[test]
fn alias_resolution_binds_only_the_renamed_trait() {
    let names = implementor_names(
        &strip_comments_and_strings(
            "use a::b::ChannelIngressVerifier as V;\nuse a::b::Unrelated as W;\n",
        ),
        "ChannelIngressVerifier",
    );

    assert!(names.contains("ChannelIngressVerifier"));
    assert!(names.contains("V"), "the renamed binding must be governed");
    assert!(
        !names.contains("W"),
        "an unrelated trait's alias must not be"
    );
}

/// The census reads every file the walk hands it, so an I/O failure must fail
/// the gate rather than contribute an empty source. Both probes fail for a
/// deterministic, platform-independent reason (`read_to_string` on a directory;
/// `read_dir` on a regular file) rather than depending on a chmod that root
/// ignores in a container.
#[test]
#[should_panic(expected = "must fail the gate, not scan as empty")]
fn an_unreadable_source_fails_the_census() {
    let temp = tempfile::tempdir().expect("tempdir");
    let _ = read_source(temp.path());
}

#[test]
#[should_panic(expected = "not vanish from the walk")]
fn an_unreadable_directory_fails_the_walk() {
    let temp = tempfile::tempdir().expect("tempdir");
    let not_a_directory = temp.path().join("regular-file");
    fs::write(&not_a_directory, "// fixture\n").expect("fixture file");
    let mut out = Vec::new();
    collect_production_rs(&not_a_directory, &mut out);
}

/// The census resolves its scan roots from the root manifest's member list, so
/// a workspace member outside `crates/` is covered. `tools/ironclaw_stress`
/// depends on `ironclaw_host_api` and can therefore implement a witness trait;
/// before this it sat outside every scan while the file floor stayed cleared.
#[test]
fn the_census_covers_every_workspace_member_root() {
    let root = workspace_root();
    let roots = member_roots(&root);
    assert!(
        roots.contains(&root.join("crates")),
        "the crates/ member root must be scanned: {roots:?}"
    );
    assert!(
        roots.len() > 1,
        "the member list names roots outside crates/ ({roots:?}) — if that ever stops being \
         true this assertion is the place to record it, not a silently narrowed scan"
    );

    let files = collect_workspace_production_rs(&root);
    for member_root in &roots {
        assert!(
            files.iter().any(|file| file.starts_with(member_root)),
            "the census walked no production source under {} — a member root that \
             contributes nothing is a scan that stopped covering it",
            member_root.display()
        );
    }
}

#[test]
fn manifest_and_script_walks_reach_the_files_they_claim_to() {
    let root = workspace_root();
    let mut manifests = Vec::new();
    collect_manifests(&root, &mut manifests);
    assert!(
        manifests
            .iter()
            .any(|path| path == &crate_path(&root, "crates/ironclaw_host_api/Cargo.toml")),
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
