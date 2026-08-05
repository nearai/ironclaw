//! CHECKLIST WS2, row 1 — the `extension_host` port inversion
//! (PROPOSAL §6.1.3, §6.8.2, ordering constraint §12.1c).
//!
//! `ironclaw_extension_host` sits *below* product in the target tree, so a
//! product-side port it satisfies must be **defined at the product boundary**
//! (`ironclaw_product_contracts`) and implemented here — never defined in
//! `ironclaw_assistant` and reached upward. Every trait the extension host
//! implements that is still declared inside `ironclaw_assistant` is a live
//! instance of the inverted edge, and the layer flip (`products` → `loops`)
//! cannot land while any remain: the crate would not compile.
//!
//! Two halves:
//!
//! - **The residue is frozen and shrink-only.** The traits still in the wrong
//!   place are enumerated with the reason each could not move and the WS2
//!   slice that removes it. A new one fails; a stale one fails too, so the
//!   entry has to be deleted in the same change that removes the edge. That is
//!   the same update-never-relax shape as the extension-specificity allowlist.
//! - **The inverted ports are pinned where they landed.** Each port this row
//!   moved is asserted to be defined in `ironclaw_product_contracts` and
//!   implemented in `ironclaw_extension_host`, so a revert is loud rather than
//!   a silent re-inversion. (`reborn_product_contract_location_scan.rs` already
//!   pins that no *other* crate defines or re-exports them; this pins that the
//!   implementation stayed below the contract, which that scan cannot see.)
//! - **The error vocabulary is pinned too** (WS2.2). A trait is not the only
//!   way to depend upward: `ProductSurfaceFailure` was product's *internal*
//!   workflow error and simultaneously the extension host's own lifecycle error
//!   in 19 production files, which no trait-shaped rule can see. The boundary
//!   half now lives in `ironclaw_product_contracts::error`, and the files still
//!   naming product's type are frozen exact-match and shrink-only, exactly like
//!   the trait residue.

// The shared walker is compiled per test binary; each binary uses a subset.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

use ratchet_support::{
    TypeDefOccurrence, cfg_test_only_files, collect_type_defs, names_crate, out_of_line_mod_decls,
    production_rust_files, strip_comments_and_strings, workspace_root,
};

const PRODUCT: &str = "ironclaw_assistant";
const PRODUCT_CONTRACTS: &str = "ironclaw_product_contracts";
const EXTENSION_HOST: &str = "ironclaw_extension_host";
const EXTENSION_MANAGER: &str = "ironclaw_extension_manager";

/// Product-declared traits `ironclaw_extension_host` still implements, each
/// with the reason the WS2 port-inversion row could not move it and the slice
/// that will. **Shrink-only**: adding an entry re-inverts the edge, and an
/// entry that no longer matches is deleted in the change that fixes it.
///
/// Every reason below is a *contract-purity* fact, not a preference:
/// `ironclaw_product_contracts` may depend on `ironclaw_host_api` and
/// `ironclaw_extension_contracts` and nothing else internal
/// (`reborn_dependency_boundaries.rs`), so a port whose signature names a type
/// from `ironclaw_auth`, `ironclaw_threads`, `ironclaw_turns`, or
/// `ironclaw_conversations` cannot be declared there until that type is
/// narrowed out of the signature.
///
/// **WS2.2 rewrote three of these reasons and deleted a fourth.** The row that
/// froze this list named `ProductSurfaceFailure` as the blocker on three ports;
/// that is no longer true of any of them. The boundary error moved to
/// `ironclaw_product_contracts::error::ProductOperationFailure`, so what
/// actually blocks the survivors is their *request/response* vocabulary, which
/// is what each reason now states. `ProductConversationSubjectRouteResolver`
/// had no other blocker and was inverted.
///
/// **WS2.5 then cleared every vocabulary-blocked row, 4 -> 1, and the reasons
/// above were the map that made it mechanical.** Two of the three did not need
/// their vocabulary narrowed at all: `AuthChallengeProvider` and
/// `ChannelConnectionService` were declared where their vocabulary already
/// lives (`ironclaw_auth`), which is what `.claude/rules/type-placement.md`
/// §2/§3 and `families/contracts.md:46` ask for and costs zero type
/// weakening — the residue clears when a trait stops being *product*-declared,
/// whichever legal home it lands in. The third,
/// `ProductActorUserResolver`, did move to `ironclaw_product_contracts`,
/// because the one type that blocked it (`ExternalActorBindingEpoch`) belonged
/// beside `ExternalActorRef` in `ironclaw_extension_contracts::external` all
/// along.
///
/// **The D-A factory port (§12.11) took the last one, 1 -> 0.**
/// `ProductBindingResolver` moved to `ironclaw_product_contracts::binding`
/// with `ResolveBindingRequest`, `ResolvedBinding`, and the route-kind grammar
/// that derives them — every one of those names nothing but `host_api` and
/// `extension_contracts` vocabulary, so the DTOs were never the real blocker,
/// only the fact that nobody had moved them. The error became
/// `ProductOperationFailure`, which grew the three discriminants the binding
/// path actually constructs (`BindingRequired`, `UnknownInstallation`,
/// `TurnSubmissionRejected`) rather than collapsing them — `BindingRequired`
/// in particular is what an unpaired external actor is told, and folding it
/// into `BindingResolutionFailed` would have changed that message.
///
/// **An empty list is the end state for this half, not a disabled gate.** The
/// exact two-way diff below still fails on a new inverted edge, and the
/// manifest biconditional now keys on this list *and* the reference ledger, so
/// emptying this one does not release the manifest edge on its own.
const PRODUCT_DEFINED_TRAITS_EXTENSION_HOST_STILL_IMPLEMENTS: &[(&str, &str)] = &[];

/// The ports this row inverted: defined in `ironclaw_product_contracts` and
/// implemented **below** product, paired with the crate that implements each.
/// Enumerated so a rename or a relocation has to come through this file.
///
/// **WS2.4 made the implementor explicit.** The list used to assert only that
/// `ironclaw_extension_host` implements every inverted port, which was true
/// while the extension host was one crate. The `ironclaw_extension_manager`
/// split moved four implementations (the lifecycle product service, the admin
/// configuration view provider, the channel-config product service, and — via
/// the residue list above — the credential setup service) into the manager.
/// Asserting "implemented in extension_host" would then have failed for a
/// *correct* change, and the fix-it message the gate already carried says the
/// right thing: "if the implementor moved, move this row with it rather than
/// deleting the pin". That is what this pairing does. Both crates sit below
/// the contract, which is the property the pin exists to protect.
const INVERTED_PORT_IMPLEMENTORS: &[(&str, &str)] = &[
    ("AccountConnectionStatusSource", EXTENSION_HOST),
    ("ApprovalPromptContextSource", EXTENSION_HOST),
    ("BlockedAuthPromptSource", EXTENSION_HOST),
    // WS2.4: `RebornChannelConfigProductService` is the product projection over
    // the host's `ChannelConfigService`; the service core stayed (§6.8.2), the
    // projection moved (§6.8.3).
    ("ChannelConfigProductService", EXTENSION_MANAGER),
    ("ChannelDeliveryResolver", EXTENSION_HOST),
    ("CommandActorRoleResolver", EXTENSION_HOST),
    ("DeliveryReplyContextSource", EXTENSION_HOST),
    // WS2.4: the lifecycle product service is the manager's headline surface.
    ("LifecycleProductService", EXTENSION_MANAGER),
    // WS2.5: inverted once `ExternalActorBindingEpoch` moved to
    // `ironclaw_extension_contracts::external`, which made
    // `ResolvedProductActorUser` contracts-legal. Its request and response
    // types moved with it and the error became `ProductOperationFailure`.
    ("ProductActorUserResolver", EXTENSION_HOST),
    // WS2.2: inverted once `ProductOperationFailure` gave it a contracts-legal
    // error. Its request type and route key moved with it.
    ("ProductConversationSubjectRouteResolver", EXTENSION_HOST),
    // WS2.4: the admin-configuration view provider is a credential/admin view.
    ("RebornViewProvider", EXTENSION_MANAGER),
];

/// Ceiling on the residue. Only ever moves down. (WS2.1 froze it at 6; WS2.2
/// inverted `ProductConversationSubjectRouteResolver`; WS2.4 moved
/// `ExtensionCredentialSetupService`'s implementation out of the crate; WS2.5
/// took the three vocabulary-blocked ports 4 -> 1 — `AuthChallengeProvider` and
/// `ChannelConnectionService` to `ironclaw_auth` beside the vocabulary that
/// blocked them, `ProductActorUserResolver` to `ironclaw_product_contracts`
/// once `ExternalActorBindingEpoch` moved to `ironclaw_extension_contracts`;
/// the §12.11 D-A factory port took `ProductBindingResolver` 1 -> 0.)
const WS2_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE: usize = 0;

/// The manager twin of the host list above. WS2.4 moved
/// `ExtensionCredentialSetupService`'s implementation out of the host, which
/// removed it from the host's frozen residue — but the trait is still declared
/// in `ironclaw_assistant`, so the inverted edge now runs from the *manager* and
/// needs its own freeze: without one, any product-declared trait could quietly
/// gain a manager implementation at file granularity the residue list in
/// `reborn_extension_manager_split.rs` cannot see.
const PRODUCT_DEFINED_TRAITS_EXTENSION_MANAGER_STILL_IMPLEMENTS: &[(&str, &str)] = &[(
    "ExtensionCredentialSetupService",
    "implemented in webui_extension_credentials.rs; the port stays declared in \
     ironclaw_assistant because its vocabulary is ironclaw_auth credential-account \
     projections. WS2.5 showed the cheaper answer for that class: declare the \
     port in ironclaw_auth beside the vocabulary, as AuthChallengeProvider and \
     ChannelConnectionService now are, rather than narrowing the vocabulary out \
     to reach ironclaw_product_contracts",
)];

/// **The full `ironclaw_assistant` reference ledger** — every production file in
/// `ironclaw_extension_host` whose code names `ironclaw_assistant` at all, with
/// the reason it still does. Exact-match in both directions and shrink-only.
///
/// Why this exists when the trait residue above already does: the trait list is
/// **trait-shaped** — it sees `impl <product trait> for …` headers and nothing
/// else. A dependency can also be a constant (`adapter_registry::
/// PRODUCT_ADAPTER_HOST_API_ID`), a free function (`auth_prompt_view_for_
/// blocked_auth`), or an inline construction of a concrete product type
/// (`ProductConversationBindingService::new`), and none of those register
/// there (`auth_prompt_view_for_blocked_auth` was one, until WS2.5 moved it to
/// `ironclaw_auth::product_prompt` with the rest of the challenge family). The
/// manifest biconditional below catches the *sum* loudly, but as a
/// boolean: it cannot say what remains. The `products → loops` re-layer was
/// sized five times from proxies of this set and was wrong five times
/// (PROPOSAL §12.11 D-A and its 2026-08-03 amendment; #7092; #7143; #7145) —
/// this ledger makes the remaining scope a file-list diff instead of an
/// estimate.
///
/// **Update rule:** removing a reference deletes its row in the same change
/// (the stale direction fails otherwise); a new production file naming product
/// fails the gate and is not to be allowlisted here — implement against
/// `ironclaw_product_contracts` instead (§6.1.3). The row's reason names the
/// blocker class so the flip's remaining work stays enumerable: `port` (the
/// trait residue above), `adapter-registry` (manifest projection, owned by
/// CHECKLIST WS5's `product` narrows row), `product-fn` (a free function that
/// moves with its vocabulary), or `assembly` (the D-A factory-port scope).
const EXTENSION_HOST_PRODUCTION_FILES_STILL_NAMING_PRODUCT: &[(&str, &str)] = &[];

/// Ceiling on the reference ledger. Only ever moves down — growing the frozen
/// list past it needs this constant raised in the same PR, which is the
/// deliberate two-edit speed bump against re-widening the edge.
///
/// **This batch took it 9 -> 2, in two moves.**
///
/// *WS2.5 took 9 -> 5.* Four rows fell together, all by the same move: the
/// port-facing vocabulary went to the crate that owns it, and product maps at
/// its boundary. `channel_connection.rs` and `product_lifecycle.rs` speak
/// `ironclaw_auth::{ChannelConnectionService, ChannelAuthAccountState}` and the
/// `ExtensionAccountSetupReader` port; `provider_identity.rs` speaks
/// `ironclaw_product_contracts::actor_identity`; `run_delivery_ports.rs` speaks
/// `ironclaw_auth::product_prompt` for the challenge family and
/// `ironclaw_product_contracts::approval_prompt` for the approval projection.
///
/// *WS5 then took 5 -> 2.* The whole `adapter-registry` class went with the
/// `[product_adapter.*]` manifest surface: `product_adapter_section` moved out
/// of `ironclaw_assistant::adapter_registry` into
/// `ironclaw_extension_contracts`, so `available_extensions.rs`,
/// `channel_lifecycle.rs`, and `host_api_contracts.rs` now name the contracts
/// crate rather than product.
///
/// **The §12.11 D-A factory port took the last two, 2 -> 0 (2026-08-05).** Both
/// `assembly` rows fell together and by the same move: the per-extension
/// product cone is now built by `ChannelWorkflowFactory` (declared in
/// `ironclaw_product_contracts`, implemented by
/// `ironclaw_assistant::RebornChannelWorkflowFactory`, injected through
/// `GenericChannelHostDeps`), and the proactive half by
/// `ironclaw_outbound::TriggeredRunDelivery` — declared beside the
/// triggered-delivery vocabulary it already carries, the same placement rule
/// WS2.5 applied to the auth ports. `channel_host.rs` states the shape and
/// consumes the result; `channel_triggered_delivery.rs` routes a fire to a
/// driver composition built. Neither constructs a product type. (The two
/// halves landed on different branches — the D-A port on the #7202 line, the
/// adapter-registry move via #7181 — and their union emptied the ledger, which
/// is what released the manifest edge and the layer flip.)
const EXTENSION_HOST_PRODUCT_REFERENCE_FILE_BASELINE: usize = 0;

/// Workspace package metadata, resolved once per test binary.
///
/// The move-order proof (CHECKLIST WS2's last row) turns on this: every path
/// this gate reads is derived from `cargo metadata`'s `manifest_path`, never
/// assembled as `crates/<name>`. WS7 moves these crates into family
/// directories, and a literal path would then resolve to nothing — which for
/// a scan means walking an empty tree, and for a manifest guard means an
/// `exists()` check that quietly stops guarding. Resolved through cargo, a
/// crate that is not a workspace member is a **panic**, and one that moved is
/// simply followed.
fn workspace_metadata() -> &'static Value {
    static METADATA: OnceLock<Value> = OnceLock::new();
    METADATA.get_or_init(|| {
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
    })
}

/// The `cargo metadata` package entry for a workspace crate. Absence is fatal
/// on purpose — see [`workspace_metadata`].
fn package(name: &str) -> &'static Value {
    workspace_metadata()["packages"]
        .as_array()
        .expect("cargo metadata packages")
        .iter()
        .find(|package| package["name"].as_str() == Some(name))
        .unwrap_or_else(|| {
            panic!(
                "{name} is not a workspace member; this gate resolves every path through \
                 cargo metadata, so an unregistered crate must fail loudly rather than \
                 leave the scans walking a directory that is not there"
            )
        })
}

/// A workspace crate's directory, taken from its manifest's real location.
fn crate_dir(name: &str) -> PathBuf {
    let manifest = package(name)["manifest_path"]
        .as_str()
        .unwrap_or_else(|| panic!("{name} has no manifest_path in cargo metadata"));
    Path::new(manifest)
        .parent()
        .unwrap_or_else(|| panic!("{name}'s manifest_path has no parent directory"))
        .to_path_buf()
}

fn crate_src(name: &str) -> PathBuf {
    crate_dir(name).join("src")
}

fn is_rust_identifier(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn traits_defined_in(crate_name: &str) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_src(crate_name),
        &["trait "],
        &is_rust_identifier,
        &[],
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no traits discovered in {crate_name} — the walk is broken, not the crate"
    );
    found.into_keys().collect()
}

/// Every production `.rs` file under `dir`. **Every I/O error is fatal**: a
/// scan that silently shrinks its input passes while enforcing nothing, which
/// is the failure mode this whole file exists to prevent. A missing directory,
/// an unreadable entry, or a permission error must red the gate, not thin it.
/// Remove `#[cfg(test)]`-gated items. Only the *production* edge blocks the
/// layer flip: a test double may implement a product trait through the crate's
/// dev-dependency without the shipped artifact depending on product. Same
/// stripping shape as `reborn_registration_pipeline_boundary.rs`.
///
/// **Callers must strip comments and string literals first.** This walk finds
/// the block by counting raw `{`/`}` bytes, so a brace inside a doc comment or
/// a string literal in a gated block desynchronizes the depth and either leaks
/// a test-only `impl` into the production set or swallows production code that
/// follows. `implemented_trait_names` composes them in that order and
/// `cfg_test_stripping_survives_braces_in_comments_and_strings` pins it.
///
/// `#[cfg(feature = "test-support")]` items are deliberately **not** stripped:
/// that feature compiles into a real build (CI's `--all-features` lanes enable
/// it), so an `impl` behind it is a genuine normal-dependency edge that would
/// block the layer flip. Only `#[cfg(test)]` is invisible to a shipped
/// artifact.
fn strip_cfg_test_blocks(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(at) = rest.find(MARKER) {
        out.push_str(&rest[..at]);
        let after = &rest[at + MARKER.len()..];
        let Some(open) = after.find('{') else {
            // A `#[cfg(test)] use …;` line: drop through the statement end.
            match after.find(';') {
                Some(semi) => {
                    rest = &after[semi + 1..];
                    continue;
                }
                None => return out,
            }
        };
        // An attribute followed by a `;` before any `{` is a gated statement.
        if let Some(semi) = after.find(';')
            && semi < open
        {
            rest = &after[semi + 1..];
            continue;
        }
        let bytes = after.as_bytes();
        let mut depth = 0usize;
        let mut idx = open;
        while idx < bytes.len() {
            match bytes[idx] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        if idx >= bytes.len() {
            return out;
        }
        rest = &after[idx + 1..];
    }
    out.push_str(rest);
    out
}

/// Byte index of the `>` that closes the `<` at index 0, counting nesting.
/// `None` when the brackets never balance (a truncated slice), which the
/// caller treats as "not an impl header I can read" rather than guessing.
fn balanced_angle_close(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '<' => depth += 1,
            // A `->` inside a bound (`impl<F: Fn(&str) -> bool>`) is a return
            // arrow, not a closing bracket. `-` never opens one, so a `>`
            // preceded by `-` is skipped.
            '>' if index > 0 && bytes[index - 1] == b'-' => {}
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Trait names appearing as the *implemented* trait of an `impl … for …` item,
/// with any leading path qualifier and generic arguments dropped
/// (`impl ironclaw_assistant::Foo<T> for Bar` → `Foo`). Inherent impls
/// (`impl Bar {`) have no `for` and never match.
fn implemented_trait_names(source: &str) -> BTreeSet<String> {
    let cleaned = strip_cfg_test_blocks(&strip_comments_and_strings(source));
    let mut names = BTreeSet::new();
    for segment in cleaned.split("impl").skip(1) {
        let Some(head) = segment.split_once(" for ") else {
            continue;
        };
        let mut candidate = head.0.trim();
        // Drop an `<'a, T>` generic-parameter list that binds the impl itself.
        // The close must be found by *balancing*, not by the first `>`: a bound
        // may itself be generic (`impl<T: Iterator<Item = X>> Port for Host<T>`),
        // and taking the first `>` would leave `> Port` — not an identifier, so
        // the impl would be skipped and the gate would enforce nothing for it.
        if candidate.starts_with('<') {
            let Some(close) = balanced_angle_close(candidate) else {
                continue;
            };
            candidate = candidate[close + 1..].trim();
        }
        // Drop generic arguments on the trait itself, then the path qualifier.
        let candidate = candidate.split('<').next().unwrap_or(candidate).trim();
        let Some(last) = candidate.rsplit("::").next() else {
            continue;
        };
        let last = last.trim();
        if is_rust_identifier(last) {
            names.insert(last.to_string());
        }
    }
    names
}

/// Traits implemented by `crate_name`'s production code, counted only in files
/// whose (comment/string/`#[cfg(test)]`-stripped) code names `referenced_crate`
/// as a whole token.
///
/// The qualification exists because `implemented_trait_names` records
/// *unqualified* final path segments: without it, a crate-local trait that
/// merely shares an inverted port's name would satisfy the implementor pin
/// while the contracts trait went unimplemented — the pin would fail open. A
/// real implementation must have the trait in scope, which spells the owning
/// crate's name in the file.
///
/// Files reachable only through `#[cfg(test)] mod …;` chains are excluded up
/// front: they are test code wearing a production filename, and a test double
/// in one of them must satisfy nothing (see `cfg_test_only_files`).
fn traits_implemented_by(
    crate_name: &str,
    referenced_crate: &str,
    minimum_files: usize,
) -> BTreeSet<String> {
    let src = crate_src(crate_name);
    let files = production_rust_files(&src);
    assert!(
        files.len() >= minimum_files,
        "expected to walk {crate_name}'s source tree; found {} files (floor {minimum_files}) — \
         a broken path must fail loudly rather than yield an empty, vacuously passing set",
        files.len()
    );
    let mut names = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        let cleaned = strip_cfg_test_blocks(&strip_comments_and_strings(&source));
        if !names_crate(&cleaned, referenced_crate) {
            continue;
        }
        names.extend(implemented_trait_names(&source));
    }
    names
}

/// Shared body of the host/manager residue checks: the crate implements
/// exactly the frozen set of product-declared traits, no more (a new inverted
/// edge) and no fewer (a stale row).
fn assert_product_trait_residue_is_exact(
    crate_name: &str,
    minimum_files: usize,
    residue: &[(&str, &str)],
    baseline: usize,
) {
    let product_traits = traits_defined_in(PRODUCT);
    let implemented = traits_implemented_by(crate_name, PRODUCT, minimum_files);

    let found: BTreeSet<String> = implemented.intersection(&product_traits).cloned().collect();
    let frozen: BTreeSet<String> = residue
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let mut violations = Vec::new();
    for name in found.difference(&frozen) {
        violations.push(format!(
            "{crate_name} implements {PRODUCT}::{name}, which re-inverts the \
             {crate_name} -> product edge. Define the port in {PRODUCT_CONTRACTS} \
             (PROPOSAL §6.1.3) instead of adding a row here"
        ));
    }
    for name in frozen.difference(&found) {
        violations.push(format!(
            "{name} is listed as residue but {crate_name} no longer implements a \
             {PRODUCT}-defined trait by that name — delete its row in the same change"
        ));
    }

    assert!(
        violations.is_empty(),
        "WS2 port-inversion rule violated (CHECKLIST WS2 row 1):\n{}",
        violations.join("\n")
    );
    assert!(
        found.len() <= baseline,
        "the product-defined trait residue is shrink-only: {} > baseline {}",
        found.len(),
        baseline
    );
}

#[test]
fn extension_host_implements_only_the_frozen_residue_of_product_defined_traits() {
    assert_product_trait_residue_is_exact(
        EXTENSION_HOST,
        21,
        PRODUCT_DEFINED_TRAITS_EXTENSION_HOST_STILL_IMPLEMENTS,
        WS2_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE,
    );
}

#[test]
fn extension_manager_implements_only_the_frozen_residue_of_product_defined_traits() {
    assert_product_trait_residue_is_exact(
        EXTENSION_MANAGER,
        10,
        PRODUCT_DEFINED_TRAITS_EXTENSION_MANAGER_STILL_IMPLEMENTS,
        PRODUCT_DEFINED_TRAITS_EXTENSION_MANAGER_STILL_IMPLEMENTS.len(),
    );
}

#[test]
fn inverted_ports_are_declared_in_contracts_and_implemented_below_product() {
    let contract_traits = traits_defined_in(PRODUCT_CONTRACTS);
    let product_traits = traits_defined_in(PRODUCT);
    let host_impls = traits_implemented_by(EXTENSION_HOST, PRODUCT_CONTRACTS, 21);
    let manager_impls = traits_implemented_by(EXTENSION_MANAGER, PRODUCT_CONTRACTS, 10);

    let mut violations = Vec::new();
    for (port, implementor) in INVERTED_PORT_IMPLEMENTORS {
        if !contract_traits.contains(*port) {
            violations.push(format!(
                "{port} must be declared in {PRODUCT_CONTRACTS}; the WS2 inversion moved it there"
            ));
        }
        if product_traits.contains(*port) {
            violations.push(format!(
                "{port} is declared again in {PRODUCT} — the inversion gives it exactly one home"
            ));
        }
        let implemented = match *implementor {
            EXTENSION_HOST => &host_impls,
            EXTENSION_MANAGER => &manager_impls,
            other => panic!("{port} names an unknown implementor crate {other}"),
        };
        if !implemented.contains(*port) {
            violations.push(format!(
                "{port} has no implementation in {implementor}; if the implementor moved, \
                 repoint this row rather than deleting the pin"
            ));
        }
        // The pin is only meaningful if the port has exactly one home below the
        // contract: a second implementation in the other crate would mean the
        // WS2.4 split duplicated a surface rather than moving it.
        let other = match *implementor {
            EXTENSION_HOST => (&manager_impls, EXTENSION_MANAGER),
            _ => (&host_impls, EXTENSION_HOST),
        };
        // Scope note: "exactly one" is asserted between the two split crates
        // only. Impls elsewhere (product's own Unsupported*/Unavailable*
        // fallbacks, composition doubles) are legal downward edges this gate
        // deliberately does not police.
        if other.0.contains(*port) {
            violations.push(format!(
                "{port} is implemented in both {implementor} and {} — within the split \
                 pair a port has exactly one home; the split moves a surface, it does \
                 not fork it",
                other.1
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "WS2 inverted-port placement violated (PROPOSAL §6.1.3):\n{}",
        violations.join("\n")
    );
}

/// The only `ironclaw_extension_host` production files still allowed to name
/// product's workflow error, each with the residue port that forces it.
///
/// **Shrink-only, exact-match, and now empty.** Each entry was a file
/// implementing a port whose *signature* still named `ProductSurfaceFailure`
/// because the port itself had not been inverted (see the trait residue above).
/// All 19 production files the WS2.1 finding counted now speak
/// `ironclaw_product_contracts::error::ProductOperationFailure`: the last one,
/// `channel_host.rs`, lost its binding-port implementation to the §12.11 D-A
/// factory port, and the port itself took the boundary error with it when it
/// moved to `ironclaw_product_contracts::binding`. A file appearing here means
/// the boundary error was bypassed.
const EXTENSION_HOST_FILES_STILL_NAMING_THE_WORKFLOW_ERROR: &[(&str, &str)] = &[];

/// Production files in `crate_name` whose *code* names `type_name`, as paths
/// relative to the crate's `src/`.
///
/// Comments and string literals are stripped **first**, so prose about the
/// migration — including this file's own vocabulary — never registers as a
/// dependency, and so a brace inside a comment or literal cannot desynchronise
/// the `#[cfg(test)]` brace matching that runs next. (Same ordering as
/// `implemented_trait_names`; getting it backwards is a silent miscount.)
/// `#[cfg(test)]` blocks then go for the same reason the impl scan drops them:
/// a test double may reach product through a dev-dependency without the shipped
/// artifact doing so.
///
/// An unreadable file is fatal, not skipped — a silent skip is how this scan
/// would go quietly vacuous.
fn production_files_naming(
    crate_name: &str,
    type_name: &str,
    minimum_files: usize,
) -> BTreeSet<String> {
    let src = crate_src(crate_name);
    let files = production_rust_files(&src);
    assert!(
        files.len() >= minimum_files,
        "expected to walk {crate_name}'s source tree; found {} files (floor {minimum_files}) — \
         a broken path must fail loudly rather than report an empty, vacuously passing set",
        files.len()
    );
    let mut named = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        if strip_cfg_test_blocks(&strip_comments_and_strings(&source)).contains(type_name) {
            let relative = file.strip_prefix(&src).unwrap_or_else(|error| {
                panic!("{} is not under {}: {error}", file.display(), src.display())
            });
            named.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    named
}

/// Production files in `crate_name` whose *code* names `referenced_crate` as a
/// whole token, as paths relative to the crate's `src/`.
///
/// The sibling of [`production_files_naming`] for crate names rather than type
/// names. The difference is load-bearing, not stylistic: that helper matches by
/// substring, and `ironclaw_assistant` is a substring of
/// `ironclaw_product_contracts` — a raw `contains` here would count every file
/// that (correctly) imports the contracts crate and the ledger would freeze
/// files that never touch product. `names_crate` is the same whole-token
/// matcher the impl scan uses, so the two scans cannot disagree about what
/// "names the crate" means. Stripping order and the fatal-I/O rule are
/// identical to the sibling, for the reasons documented there.
fn production_files_naming_crate(
    crate_name: &str,
    referenced_crate: &str,
    minimum_files: usize,
) -> BTreeSet<String> {
    let src = crate_src(crate_name);
    let files = production_rust_files(&src);
    assert!(
        files.len() >= minimum_files,
        "expected to walk {crate_name}'s source tree; found {} files (floor {minimum_files}) — \
         a broken path must fail loudly rather than report an empty, vacuously passing set",
        files.len()
    );
    let mut named = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        if names_crate(
            &strip_cfg_test_blocks(&strip_comments_and_strings(&source)),
            referenced_crate,
        ) {
            let relative = file.strip_prefix(&src).unwrap_or_else(|error| {
                panic!("{} is not under {}: {error}", file.display(), src.display())
            });
            named.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    named
}

#[test]
fn extension_host_speaks_the_contract_error_everywhere_but_the_frozen_residue_files() {
    let found = production_files_naming(EXTENSION_HOST, "ProductSurfaceFailure", 21);
    let frozen: BTreeSet<String> = EXTENSION_HOST_FILES_STILL_NAMING_THE_WORKFLOW_ERROR
        .iter()
        .map(|(file, _)| (*file).to_string())
        .collect();

    let mut violations = Vec::new();
    for file in found.difference(&frozen) {
        violations.push(format!(
            "{EXTENSION_HOST}/src/{file} names {PRODUCT}::ProductSurfaceFailure. Use \
             {PRODUCT_CONTRACTS}::error::ProductOperationFailure — product absorbs it \
             with a total From, so nothing is lost at a product call site"
        ));
    }
    for file in frozen.difference(&found) {
        violations.push(format!(
            "{file} is listed as still naming the workflow error but no longer does — \
             delete its row in the same change"
        ));
    }
    assert!(
        violations.is_empty(),
        "WS2.2 error-vocabulary rule violated:\n{}",
        violations.join("\n")
    );

    // The other half of the claim: the contract error is actually the one in
    // use, so the scan above cannot pass by the crate simply not having errors.
    //
    // **Counted across both halves of the split (WS2.4).** The lifecycle
    // surface that spoke `ProductOperationFailure` in 17 files was cut in two;
    // holding `extension_host` alone to the old floor would have made a
    // correct move look like a regression, and lowering the floor to fit
    // would have relaxed the guard. Summing the halves keeps the original
    // claim exactly: the boundary error is still the vocabulary of the whole
    // lifecycle surface, wherever that surface now lives. Each half is also
    // held above zero so the sum cannot be satisfied by one crate alone.
    let host_users = production_files_naming(EXTENSION_HOST, "ProductOperationFailure", 21);
    let manager_users = production_files_naming(EXTENSION_MANAGER, "ProductOperationFailure", 10);
    assert!(
        !host_users.is_empty() && !manager_users.is_empty(),
        "both halves of the lifecycle surface must speak the contract error; \
         host={host_users:?} manager={manager_users:?}"
    );
    assert!(
        host_users.len() + manager_users.len() >= 15,
        "expected the contract error across the extension lifecycle surface; found only \
         {} host + {} manager files: {host_users:?} {manager_users:?}",
        host_users.len(),
        manager_users.len()
    );
}

/// The reference ledger's enforcement: `ironclaw_extension_host`'s production
/// tree names `ironclaw_assistant` in exactly the frozen files, no more (a new
/// reference, or a proxy-sized estimate about to be wrong again) and no fewer
/// (a stale row that must fall with the change that removed the reference).
///
/// Vacuity guards, in order: the walk floor inside
/// [`production_files_naming_crate`] (an empty tree cannot read as success);
/// the exact two-way diff (an empty found-set fails against a non-empty frozen
/// list); and the ledger⇄manifest consistency assert at the bottom (the ledger
/// cannot read empty while the manifest still carries the dependency — the
/// itemization and the biconditional in
/// [`the_extension_host_manifest_names_product_only_while_a_residue_needs_it`]
/// must agree about whether the edge exists).
#[test]
fn extension_host_production_files_naming_product_are_exactly_the_frozen_ledger() {
    let found = production_files_naming_crate(EXTENSION_HOST, PRODUCT, 21);
    let frozen: BTreeSet<String> = EXTENSION_HOST_PRODUCTION_FILES_STILL_NAMING_PRODUCT
        .iter()
        .map(|(file, _)| (*file).to_string())
        .collect();
    assert_eq!(
        frozen.len(),
        EXTENSION_HOST_PRODUCTION_FILES_STILL_NAMING_PRODUCT.len(),
        "the reference ledger lists a file twice — every row must be a distinct file"
    );

    let mut violations = Vec::new();
    for file in found.difference(&frozen) {
        violations.push(format!(
            "{EXTENSION_HOST}/src/{file} names {PRODUCT} but has no ledger row. Do not add \
             one: implement against {PRODUCT_CONTRACTS} (PROPOSAL §6.1.3) — the ledger only \
             shrinks on the way to the products -> loops re-layer (#7145)"
        ));
    }
    for file in frozen.difference(&found) {
        violations.push(format!(
            "{file} is in the reference ledger but no longer names {PRODUCT} — delete its \
             row in the same change so the ledger stays the exact remaining scope"
        ));
    }
    assert!(
        violations.is_empty(),
        "WS2 product-reference ledger violated (the residue itemization, #7145):\n{}",
        violations.join("\n")
    );
    assert!(
        found.len() == EXTENSION_HOST_PRODUCT_REFERENCE_FILE_BASELINE,
        "the product-reference ledger is shrink-only: {} files > baseline {}",
        found.len(),
        EXTENSION_HOST_PRODUCT_REFERENCE_FILE_BASELINE
    );

    // The itemization and the manifest biconditional must tell one story: a
    // ledger that reads empty while the manifest still lists the dependency
    // means this scan went blind (or a reference hides somewhere no file-level
    // scan sees), and an occupied ledger without the manifest edge means the
    // rows are stale. Either way the fix is in this file, loudly.
    let host = package(EXTENSION_HOST);
    let has_normal_dep = host["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|dependency| {
            dependency["name"].as_str() == Some(PRODUCT)
                && dependency["kind"].as_str().unwrap_or("normal") == "normal"
        });
    assert_eq!(
        !found.is_empty(),
        has_normal_dep,
        "ledger occupancy ({} files) disagrees with the manifest edge (present: \
         {has_normal_dep}) — the itemization has gone vacuous or stale",
        found.len()
    );
}

/// CHECKLIST WS2's closing row — the **move-order proof**, stated as the
/// property it is rather than as a one-off audit: `ironclaw_extension_host`'s
/// manifest lists no `ironclaw_assistant` dependency *under any name* once the
/// residue is gone, and the manifest is found through `cargo metadata` rather
/// than at `crates/ironclaw_extension_host/Cargo.toml`.
///
/// Two failure modes it closes, both of which are silent today:
///
/// - **A rename blinds the source scans.** Every residue list in this file and
///   in `reborn_extension_manager_split.rs` matches the literal crate name, so
///   `p = { package = "ironclaw_assistant" }` would let the edge grow with every
///   list still passing. (Same rule the manager half already carries; the host
///   half had none.)
/// - **A directory move blinds the manifest guard.** The literal-path idiom
///   used elsewhere in this suite is `if manifest.exists() { assert!(…) }`,
///   which after WS7 moves the crate stops asserting instead of failing.
///   Resolved through cargo, an unregistered crate panics in [`package`] and a
///   relocated one is simply followed.
///
/// The dependency is asserted to exist **exactly while** a residue needs it, so
/// the last residue row and the manifest edge have to go in the same change —
/// in either direction.
///
/// ✎ **"a residue" is the trait residue OR the reference ledger (2026-08-04,
/// §12.11 D-A).** This assertion used to key on the trait list alone, which was
/// a faithful proxy only while a trait was the last thing holding the edge. It
/// stopped being one the moment #7145 itemized the *full* reference ledger:
/// `adapter-registry` rows are constants and free functions, invisible to any
/// trait-shaped rule, and they hold the manifest edge just as hard. Keying on
/// the trait list alone would now demand the edge be **deleted** while three
/// files still name the crate — i.e. it would fail a correct tree and pass an
/// impossible one. Both directions stay enforced against the union: the edge
/// must exist while either list is occupied, and must be gone when both are.
/// (`extension_host_production_files_naming_product_are_exactly_the_frozen_ledger`
/// carries the ledger half of the same biconditional, so the two agree by
/// construction.)
#[test]
fn the_extension_host_manifest_names_product_only_while_a_residue_needs_it() {
    let host = package(EXTENSION_HOST);

    // The resolution is live, not vacuous: whatever cargo reported is the
    // directory the scans in this file actually walk.
    let lib = crate_src(EXTENSION_HOST).join("lib.rs");
    assert!(
        lib.is_file(),
        "cargo metadata put {EXTENSION_HOST} at {}, which holds no src/lib.rs — the \
         manifest_path resolution is wrong, and every scan in this file reads from it",
        crate_dir(EXTENSION_HOST).display()
    );

    let product_deps: Vec<&Value> = host["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| dependency["name"].as_str() == Some(PRODUCT))
        .collect();
    for dependency in &product_deps {
        assert!(
            dependency["rename"].is_null(),
            "{EXTENSION_HOST} renames its {PRODUCT} dependency to {:?}. Every residue list \
             in this suite matches the real crate name, so a rename lets the edge grow with \
             all of them still green",
            dependency["rename"]
        );
    }

    let has_normal_dep = product_deps
        .iter()
        .any(|dependency| dependency["kind"].as_str().unwrap_or("normal") == "normal");
    let trait_residue_is_open = !PRODUCT_DEFINED_TRAITS_EXTENSION_HOST_STILL_IMPLEMENTS.is_empty();
    let ledger_is_open = !EXTENSION_HOST_PRODUCTION_FILES_STILL_NAMING_PRODUCT.is_empty();
    let residue_is_open = trait_residue_is_open || ledger_is_open;
    assert_eq!(
        has_normal_dep, residue_is_open,
        "{EXTENSION_HOST}'s normal dependency on {PRODUCT} must exist exactly while a residue \
         still needs it. Trait residue open: {trait_residue_is_open}; reference ledger open: \
         {ledger_is_open}; manifest edge present: {has_normal_dep}. When the last row of BOTH \
         goes, delete the manifest edge in the same change (and vice versa) — that pairing IS \
         the move-order proof"
    );
}

/// The other half of the move-order proof: the resolution itself is loud.
///
/// A name cargo does not know must **panic**, not hand back a path that
/// happens not to exist — because a non-existent path is exactly what a
/// literal `crates/<name>` produces after WS7 moves a crate, and every scan
/// in this file would then walk nothing and pass.
#[test]
#[should_panic(expected = "is not a workspace member")]
fn a_crate_cargo_does_not_know_fails_loudly_rather_than_resolving_to_a_dead_path() {
    let _ = crate_dir("ironclaw_extension_host_relocated_by_ws7");
}

#[test]
fn the_boundary_error_names_no_type_the_contracts_crate_may_not_depend_on() {
    let path = crate_src(PRODUCT_CONTRACTS).join("error.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let code = strip_comments_and_strings(&source);
    assert!(
        code.contains("ProductOperationFailure"),
        "the boundary error must live in {PRODUCT_CONTRACTS}/src/error.rs"
    );
    // The whole reason the type exists: it is declarable in a crate whose
    // dependency ceiling is host_api + extension_contracts. `TurnError` is the
    // exact payload that kept `ProductSurfaceFailure` out.
    for forbidden in [
        "TurnError",
        "ironclaw_turns",
        "ironclaw_auth",
        "ironclaw_threads",
        "ironclaw_conversations",
        "ironclaw_assistant",
    ] {
        assert!(
            !code.contains(forbidden),
            "{PRODUCT_CONTRACTS}/src/error.rs names {forbidden}, which re-creates the \
             blocker WS2.2 removed"
        );
    }
}

#[test]
fn cfg_test_stripping_survives_braces_in_comments_and_strings() {
    // A brace inside a comment or a string literal in a gated block would
    // desynchronize a raw-byte depth count. Composed in the wrong order, the
    // stray `{` here swallows `Production` (or leaks `TestOnly`); composed as
    // `implemented_trait_names` does, neither happens.
    let source = r#"
        #[cfg(test)]
        mod tests {
            // an unbalanced brace in a comment: {
            const PATTERN: &str = "unbalanced { in a string";
            impl TestOnly for Double {}
        }
        impl Production for Real {}
    "#;
    let found = implemented_trait_names(source);
    assert!(
        found.contains("Production"),
        "production impl after a brace-carrying gated block must survive: {found:?}"
    );
    assert!(
        !found.contains("TestOnly"),
        "gated impl must still be stripped: {found:?}"
    );
}

#[test]
fn impl_scanner_reads_the_trait_out_of_real_impl_shapes() {
    let source = r#"
        impl Plain for Thing {}
        impl<'a, T> Generic<T> for Other<'a, T> {}
        impl ironclaw_assistant::Qualified for Third {}
        impl Inherent { fn f() {} }
        // impl Commented for Ignored {}
        impl async_trait::Marker for Fourth {}
        impl<T: Iterator<Item = X>> NestedBound for Host<T> {}
        impl<F: Fn(&str) -> bool> ArrowBound for Guard<F> {}
        #[cfg(test)]
        mod tests {
            impl TestOnly for Double {}
        }
    "#;
    let found = implemented_trait_names(source);
    assert!(found.contains("Plain"), "plain impl: {found:?}");
    assert!(found.contains("Generic"), "generic impl: {found:?}");
    assert!(
        found.contains("Qualified"),
        "path-qualified impl: {found:?}"
    );
    assert!(found.contains("Marker"), "crate-qualified impl: {found:?}");
    assert!(
        !found.contains("Inherent"),
        "inherent impl must not match: {found:?}"
    );
    assert!(
        !found.contains("Commented"),
        "commented-out impl must not match: {found:?}"
    );
    assert!(
        !found.contains("TestOnly"),
        "a #[cfg(test)] impl is not a production edge: {found:?}"
    );
    // The generic-parameter list must be closed by balancing, not by the first
    // `>`: a nested bound or an `Fn(..) -> T` return arrow both put a `>`
    // inside the list, and taking the first one silently drops the impl — a
    // hole through which a new product-defined port could enter unenforced.
    assert!(
        found.contains("NestedBound"),
        "a nested generic bound must not hide the trait: {found:?}"
    );
    assert!(
        found.contains("ArrowBound"),
        "an `Fn(..) -> T` bound must not hide the trait: {found:?}"
    );
}

/// The gate reads `cfg(test)` by walking *backwards* from the `mod` keyword
/// over the contiguous `#[…]` run, so every token the language allows between
/// the attribute and `mod` — i.e. the whole visibility grammar — has to be
/// pinned here. A visibility qualifier the walk cannot step over ends the run
/// early and the module reads as **ungated**, which is the fail-open direction:
/// `cfg_test_only_files` then leaves a `#[cfg(test)]`-only file classified as
/// production, and a test double in a production-named file can satisfy a
/// residue row or an implementor pin. Raised by @serrrfirat on #7000.
#[test]
fn mod_decl_scanner_reads_gating_and_declaration_shapes() {
    let source = r#"
        #[cfg(test)]
        mod gated;
        #[allow(dead_code)]
        #[cfg(test)]
        mod stacked_attrs;
        #[cfg(test)]
        pub mod gated_public;
        #[cfg(test)]
        pub(crate) mod gated_crate;
        #[cfg(test)]
        pub(super) mod gated_super;
        #[cfg(test)]
        pub(in crate::a::b) mod gated_in_path;
        #[allow(dead_code)]
        #[cfg(test)]
        pub(crate) mod gated_public_stacked;
        mod plain;
        pub mod public_plain;
        pub(crate) mod crate_plain;
        #[allow(dead_code)]
        pub mod attributed_but_ungated;
        mod inline_module { fn f() {} }
        // mod commented_out;
    "#;
    let decls: BTreeMap<String, bool> = out_of_line_mod_decls(&strip_comments_and_strings(source))
        .into_iter()
        .collect();
    assert_eq!(decls.get("gated"), Some(&true), "{decls:?}");
    assert_eq!(
        decls.get("stacked_attrs"),
        Some(&true),
        "cfg(test) anywhere in the contiguous attribute run gates the module: {decls:?}"
    );
    // Positive: every visibility form the language allows, gated. Reading any
    // of these as ungated is the fail-open the doc comment above describes.
    for gated_public in [
        "gated_public",
        "gated_crate",
        "gated_super",
        "gated_in_path",
        "gated_public_stacked",
    ] {
        assert_eq!(
            decls.get(gated_public),
            Some(&true),
            "a visibility qualifier must not hide the #[cfg(test)] gate \
             ({gated_public}): {decls:?}"
        );
    }
    // Negative: nothing above may be achieved by treating *any* preceding
    // attribute — or an unrelated earlier gate — as gating.
    assert_eq!(decls.get("plain"), Some(&false), "{decls:?}");
    assert_eq!(decls.get("public_plain"), Some(&false), "{decls:?}");
    assert_eq!(decls.get("crate_plain"), Some(&false), "{decls:?}");
    assert_eq!(
        decls.get("attributed_but_ungated"),
        Some(&false),
        "a non-cfg(test) attribute run must leave a pub module production: {decls:?}"
    );
    assert!(
        !decls.contains_key("inline_module"),
        "inline modules stay with their file: {decls:?}"
    );
    assert!(
        !decls.contains_key("commented_out"),
        "comments are stripped before scanning: {decls:?}"
    );
}

/// The concrete in-tree shape that motivated the test-only exclusion:
/// `channel_host.rs` declares `#[cfg(test)] mod e2e_tests;`, and
/// `e2e_tests.rs` declares `mod e2e_auth_challenge;` — a file whose
/// production-looking name evades every name-based rule while holding a fake
/// `impl` of a product-declared port. Counting it would let the fake keep a
/// residue row or an implementor pin satisfied after the real impl is gone.
/// (If the fixture is ever renamed to `*_tests.rs` or moved under `tests/`,
/// update or delete this pin in the same change.)
#[test]
fn files_reachable_only_through_cfg_test_modules_are_not_production() {
    let src = crate_src(EXTENSION_HOST);
    let test_only = cfg_test_only_files(&src);
    let evader = src.join("channel_host").join("e2e_auth_challenge.rs");
    assert!(
        evader.is_file(),
        "{} is gone — point this pin at another cfg(test)-declared, \
         production-named module file, or delete it if none remain",
        evader.display()
    );
    assert!(
        test_only.contains(&evader),
        "channel_host/e2e_auth_challenge.rs is reachable only through a #[cfg(test)] \
         module chain and must be classified test-only"
    );
}

/// The two halves of `cfg_test_only_files` the in-tree pin above cannot reach,
/// on a synthetic tree so nothing depends on a fixture staying put:
///
/// 1. **Direct `#[cfg(test)] mod …;` seeding.** The in-tree chain starts at
///    `e2e_tests.rs`, whose *name* already seeds it via the `*_tests.rs` rule —
///    so the seeding loop that reads the `cfg(test)` gate is never the reason
///    that chain is classified. Verified by deleting that loop: the pin above
///    stayed green. A production-named `fixture.rs` declared directly as
///    `#[cfg(test)] mod fixture;` would therefore have become countable with no
///    test failing.
/// 2. **Default (non-`#[path]`) transitive resolution.** The in-tree child is
///    reached through an explicit `#[path = "…"]`, so `resolve`'s ordinary
///    `<dir>/<name>.rs` and `<dir>/<name>/mod.rs` lookups are unexercised.
///
/// 3. **A gated module carrying a visibility qualifier**
///    (`#[cfg(test)] pub(crate) mod …;`) — the shape that actually ships in
///    `ironclaw_cli/src/runtime/mod.rs`. The backwards attribute walk
///    used to stop at `pub`, so the module read as ungated and its file stayed
///    countable as production. This half is the caller-level half: it asserts
///    through `production_rust_files`, the function the residue rows and
///    implementor pins actually consume, not only the classifier.
///
/// 4. **Walk scope.** The shared walk prunes the non-source directories
///    (`target/`, `node_modules/`) rather than only filtering them out of the
///    production list afterwards, so vendored packages can neither be counted
///    as production nor pollute the test-only set. `tests/` is deliberately
///    *not* pruned — it is excluded from counting, while the walk still reads
///    it so test-only-ness keeps propagating through it.
///
/// The first three are the shapes that let a test double keep a residue row or
/// an implementor pin satisfied after the real impl is gone. Raised by
/// @serrrfirat on #7003 and #7000.
#[test]
fn direct_cfg_test_module_and_default_child_are_test_only() {
    let tree = tempfile::tempdir().expect("temp tree");
    let src = tree.path().join("src");
    std::fs::create_dir_all(src.join("fixture")).expect("fixture dir");

    // Production-named, and reachable only through a cfg(test) gate. `double`
    // adds the visibility qualifier between the gate and the `mod` keyword.
    std::fs::write(
        src.join("lib.rs"),
        "pub mod real;\n#[cfg(test)]\nmod fixture;\n#[cfg(test)]\npub(crate) mod double;\n",
    )
    .expect("lib.rs");
    std::fs::write(src.join("double.rs"), "// a test double\n").expect("double.rs");
    // Declares BOTH default resolution shapes: `<dir>/<name>.rs` and
    // `<dir>/<name>/mod.rs`. No `#[path]` anywhere.
    std::fs::write(src.join("fixture.rs"), "mod child;\nmod nested;\n").expect("fixture.rs");
    std::fs::write(src.join("fixture").join("child.rs"), "// child\n").expect("child.rs");
    std::fs::create_dir_all(src.join("fixture").join("nested")).expect("nested dir");
    std::fs::write(
        src.join("fixture").join("nested").join("mod.rs"),
        "// nested\n",
    )
    .expect("nested/mod.rs");
    // Ungated sibling: must NOT be swept up, or the function would be
    // classifying the whole tree rather than the cfg(test) reachable set.
    std::fs::write(src.join("real.rs"), "// real production code\n").expect("real.rs");
    // Vendored package: no first-party Rust lives here, so the walk must prune
    // it outright — neither list may mention it, gated or not.
    let vendored = src.join("node_modules").join("pkg");
    std::fs::create_dir_all(&vendored).expect("node_modules dir");
    std::fs::write(vendored.join("lib.rs"), "#[cfg(test)]\nmod inner;\n").expect("vendored lib.rs");
    std::fs::write(vendored.join("inner.rs"), "// vendored\n").expect("vendored inner.rs");

    let test_only = cfg_test_only_files(&src);

    for expected in [
        src.join("fixture.rs"),
        src.join("fixture").join("child.rs"),
        src.join("fixture").join("nested").join("mod.rs"),
        src.join("double.rs"),
    ] {
        assert!(
            test_only.contains(&expected),
            "{} is reachable only through a `#[cfg(test)] mod …;` gate and must be \
             test-only, got {test_only:?}",
            expected.display()
        );
    }
    assert!(
        !test_only.contains(&src.join("real.rs")),
        "an ungated production module must stay production, got {test_only:?}"
    );
    assert!(
        !test_only.contains(&src.join("lib.rs")),
        "the declaring file is production; only the gated target is not"
    );
    assert!(
        !test_only.iter().any(|file| file.starts_with(&vendored)),
        "the walk must prune node_modules/ outright, got {test_only:?}"
    );

    // Through the caller: `production_rust_files` is what the residue rows and
    // implementor pins count. A `#[cfg(test)] pub(crate) mod …;` file reaching
    // this list is the fail-open — a test double satisfying a production pin.
    let production = production_rust_files(&src);
    assert!(
        !production.contains(&src.join("double.rs")),
        "a `#[cfg(test)] pub(crate) mod …;` target must not count as production, \
         got {production:?}"
    );
    assert!(
        !production.iter().any(|file| file.starts_with(&vendored)),
        "vendored packages are never production, got {production:?}"
    );
    assert!(
        production.contains(&src.join("real.rs")) && production.contains(&src.join("lib.rs")),
        "ungated production files must still be counted, got {production:?}"
    );
}
