//! CHECKLIST WS2, row 1 — the `extension_host` port inversion
//! (PROPOSAL §6.1.3, §6.8.2, ordering constraint §12.1c).
//!
//! `ironclaw_extension_host` sits *below* product in the target tree, so a
//! product-side port it satisfies must be **defined at the product boundary**
//! (`ironclaw_product_contracts`) and implemented here — never defined in
//! `ironclaw_product` and reached upward. Every trait the extension host
//! implements that is still declared inside `ironclaw_product` is a live
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

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, strip_comments_and_strings, workspace_root,
};

const PRODUCT: &str = "ironclaw_product";
const PRODUCT_CONTRACTS: &str = "ironclaw_product_contracts";
const EXTENSION_HOST: &str = "ironclaw_extension_host";

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
const PRODUCT_DEFINED_TRAITS_EXTENSION_HOST_STILL_IMPLEMENTS: &[(&str, &str)] = &[
    (
        "AuthChallengeProvider",
        "signature returns Result<_, ironclaw_auth::AuthProductError> and carries \
         ironclaw_auth::{AuthProviderId, CredentialAccountLabel, OAuthAuthorizationUrl}; \
         moving it needs the auth vocabulary narrowed out of the port first",
    ),
    (
        "ChannelConnectionService",
        "returns ChannelAuthAccountState, whose fields are \
         ironclaw_auth::{AuthFlowStatus, CredentialAccountStatus}",
    ),
    (
        "ConversationBindingService",
        "takes ironclaw_product::ResolveBindingRequest and returns \
         ironclaw_product::ResolvedBinding; both are declared in product beside \
         the route-kind grammar that derives them. The error no longer blocks it \
         (WS2.2) — the DTOs do, and they move with the channel_host row",
    ),
    (
        "ExtensionCredentialSetupService",
        "request/response types are ironclaw_auth credential-account projections",
    ),
    (
        "ProductActorUserResolver",
        "resolves to ResolvedProductActorUser, which carries \
         ironclaw_conversations::ExternalActorBindingEpoch. The error no longer \
         blocks it (WS2.2); the conversations dep is the whole blocker and needs \
         that epoch narrowed out of the response first",
    ),
];

/// The ports this row inverted: defined in `ironclaw_product_contracts`,
/// implemented in `ironclaw_extension_host`. Enumerated so a rename or a
/// relocation has to come through this file.
const INVERTED_PORTS: &[&str] = &[
    "AccountConnectionStatusSource",
    "ApprovalPromptContextSource",
    "BlockedAuthPromptSource",
    "ChannelConfigProductService",
    "ChannelDeliveryResolver",
    "CommandActorRoleResolver",
    "DeliveryReplyContextSource",
    "LifecycleProductService",
    // WS2.2: inverted once `ProductOperationFailure` gave it a contracts-legal
    // error. Its request type and route key moved with it.
    "ProductConversationSubjectRouteResolver",
    "RebornViewProvider",
];

/// Ceiling on the residue. Only ever moves down. (WS2.1 froze it at 6; WS2.2
/// inverted `ProductConversationSubjectRouteResolver`.)
const WS2_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE: usize = 5;

fn crate_src(root: &Path, name: &str) -> PathBuf {
    root.join("crates").join(name).join("src")
}

fn is_rust_identifier(ident: &str) -> bool {
    let mut chars = ident.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn traits_defined_in(root: &Path, crate_name: &str) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_src(root, crate_name),
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
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("cannot read an entry under {}: {error}", dir.display())
        });
        let path = entry.path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target") | Some("node_modules") | Some("tests")
            ) {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if name == "tests.rs" || name.ends_with("_tests.rs") {
                continue;
            }
            out.push(path);
        }
    }
}

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
/// (`impl ironclaw_product::Foo<T> for Bar` → `Foo`). Inherent impls
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

fn traits_implemented_by(root: &Path, crate_name: &str) -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_src(root, crate_name), &mut files);
    assert!(
        files.len() > 20,
        "expected to walk {crate_name}'s source tree; found {} files",
        files.len()
    );
    let mut names = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        names.extend(implemented_trait_names(&source));
    }
    names
}

#[test]
fn extension_host_implements_only_the_frozen_residue_of_product_defined_traits() {
    let root = workspace_root();
    let product_traits = traits_defined_in(&root, PRODUCT);
    let implemented = traits_implemented_by(&root, EXTENSION_HOST);

    let found: BTreeSet<String> = implemented.intersection(&product_traits).cloned().collect();
    let frozen: BTreeSet<String> = PRODUCT_DEFINED_TRAITS_EXTENSION_HOST_STILL_IMPLEMENTS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let mut violations = Vec::new();
    for name in found.difference(&frozen) {
        violations.push(format!(
            "{EXTENSION_HOST} implements {PRODUCT}::{name}, which re-inverts the \
             extension_host -> product edge. Define the port in {PRODUCT_CONTRACTS} \
             (PROPOSAL §6.1.3) instead of adding a row here"
        ));
    }
    for name in frozen.difference(&found) {
        violations.push(format!(
            "{name} is listed as residue but {EXTENSION_HOST} no longer implements a \
             {PRODUCT}-defined trait by that name — delete its row in the same change"
        ));
    }

    assert!(
        violations.is_empty(),
        "WS2 port-inversion rule violated (CHECKLIST WS2 row 1):\n{}",
        violations.join("\n")
    );
    assert!(
        found.len() <= WS2_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE,
        "the product-defined trait residue is shrink-only: {} > baseline {}",
        found.len(),
        WS2_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE
    );
}

#[test]
fn inverted_ports_are_declared_in_contracts_and_implemented_below_product() {
    let root = workspace_root();
    let contract_traits = traits_defined_in(&root, PRODUCT_CONTRACTS);
    let product_traits = traits_defined_in(&root, PRODUCT);
    let implemented = traits_implemented_by(&root, EXTENSION_HOST);

    let mut violations = Vec::new();
    for port in INVERTED_PORTS {
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
        if !implemented.contains(*port) {
            violations.push(format!(
                "{port} has no implementation in {EXTENSION_HOST}; if the implementor moved, \
                 move this row with it rather than deleting the pin"
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
/// **Shrink-only, exact-match.** These two are exactly the files implementing a
/// port whose *signature* still names `ProductSurfaceFailure` because the port
/// itself has not been inverted (see the trait residue above). Every other
/// production file — 17 of the 19 the WS2.1 finding counted — now speaks
/// `ironclaw_product_contracts::error::ProductOperationFailure`. A third file
/// appearing here means the boundary error was bypassed; a stale entry means a
/// port was inverted without deleting its row.
const EXTENSION_HOST_FILES_STILL_NAMING_THE_WORKFLOW_ERROR: &[(&str, &str)] = &[
    (
        "channel_host.rs",
        "implements ConversationBindingService and ProductActorUserResolver, both \
         still declared in ironclaw_product",
    ),
    (
        "provider_identity.rs",
        "implements ProductActorUserResolver, still declared in ironclaw_product",
    ),
];

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
fn production_files_naming(root: &Path, crate_name: &str, type_name: &str) -> BTreeSet<String> {
    let src = crate_src(root, crate_name);
    let mut files = Vec::new();
    rust_files(&src, &mut files);
    assert!(
        files.len() > 20,
        "expected to walk {crate_name}'s source tree; found {} files",
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

#[test]
fn extension_host_speaks_the_contract_error_everywhere_but_the_frozen_residue_files() {
    let root = workspace_root();
    let found = production_files_naming(&root, EXTENSION_HOST, "ProductSurfaceFailure");
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
    let contract_users = production_files_naming(&root, EXTENSION_HOST, "ProductOperationFailure");
    assert!(
        contract_users.len() >= 15,
        "expected the contract error across the extension host's lifecycle surface; \
         found only {} files: {contract_users:?}",
        contract_users.len()
    );
}

#[test]
fn the_boundary_error_names_no_type_the_contracts_crate_may_not_depend_on() {
    let root = workspace_root();
    let path = crate_src(&root, PRODUCT_CONTRACTS).join("error.rs");
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
        "ironclaw_product",
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
        impl ironclaw_product::Qualified for Third {}
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
