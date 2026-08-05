//! CHECKLIST WS5, the `ironclaw_operator` row — the operator port inversion
//! (PROPOSAL §6.1.3, §6.9.2).
//!
//! `ironclaw_operator` is a **sibling** of `ironclaw_assistant`, not a consumer
//! of it: both sit in the `products` layer and the operator crate implements
//! product-side ports rather than calling product. Declaring those ports inside
//! `ironclaw_assistant` inverted the ownership — the implementor had to compile
//! the 57k-line crate it sits beside just to name the trait it satisfies. This
//! row moved the declarations to `ironclaw_product_contracts`; this file keeps
//! them there.
//!
//! **Why a purpose-built gate rather than the layer matrix.** The same reason
//! `reborn_extension_host_port_inversion.rs` exists for the extension host:
//! `ironclaw_operator` and `ironclaw_assistant` both declare `layer = "products"`
//! and `layer_allows_dependency("products", "products")` is `true`, so this edge
//! is *legal by the matrix and invisible to it*. Nothing in
//! `reborn_dependency_boundaries.rs` could ever have reported it.
//!
//! Four halves:
//!
//! 1. **The residue is frozen, exact-match and shrink-only.** Product-declared
//!    traits `ironclaw_operator` still implements are enumerated with a reason
//!    each. It is currently **empty** — the inversion is complete — so any new
//!    entry is a regression and the baseline can only fall.
//! 2. **The manifest edge is gone, proved through `cargo metadata`.** A
//!    trait-shaped rule cannot see a dependency carried by a *type*; the
//!    manifest can. Resolved through `cargo metadata` rather than by reading a
//!    literal path, so a crate directory rename (WS10 moves this crate to
//!    `product/`) fails loudly instead of silently scanning nothing.
//! 3. **The inverted ports are pinned where they landed** — declared in
//!    contracts, *not* re-declared in product, and still implemented by the
//!    crate that owns the implementation. A revert is loud.
//! 4. **The scanner is self-tested and cannot go vacuous.** The impl reader is
//!    exercised over every impl shape the workspace actually uses, every I/O
//!    error is fatal, and each walk asserts it found something.

// The shared walker is compiled per test binary; each binary uses a subset.
#[allow(dead_code)]
mod ratchet_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use ratchet_support::{
    TypeDefOccurrence, collect_type_defs, strip_comments_and_strings, workspace_root,
};

const PRODUCT: &str = "ironclaw_assistant";
const PRODUCT_CONTRACTS: &str = "ironclaw_product_contracts";
const OPERATOR: &str = "ironclaw_operator";
const COMPOSITION: &str = "ironclaw_composition";

/// Product-declared traits `ironclaw_operator` still implements, each with the
/// reason the inversion could not move it and the slice that will.
///
/// **Empty, and that is the point.** Every product-side port the operator
/// crate satisfies is declared in `ironclaw_product_contracts`. An entry here
/// is not a normal state to be filled in later — it is a re-inversion that has
/// to be argued for in writing, and the ceiling below stops it from becoming
/// two. The `stale entry` direction is enforced with the same strength as the
/// `new entry` direction, so this list can never outlive what it describes.
const PRODUCT_DEFINED_TRAITS_OPERATOR_STILL_IMPLEMENTS: &[(&str, &str)] = &[];

/// Ceiling on the residue. Only ever moves down.
const OPERATOR_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE: usize = 0;

/// The ports this row inverted: declared in `ironclaw_product_contracts`,
/// implemented by the crate named beside each. Enumerated so a rename or a
/// relocation has to come through this file.
///
/// Note the two implementors. §6.1.3's rule is "defined here, implemented by
/// exactly the crates the caller wires" — for the operator control plane that
/// is `ironclaw_operator` for the three it owns outright, and
/// `ironclaw_composition` for readiness status, which is the only layer
/// that can see every subsystem a readiness check reports on. Pinning the
/// *implementor* is what makes a silent move of an implementation back into
/// product visible, which the location scan cannot see.
const INVERTED_PORTS: &[(&str, &str)] = &[
    ("ActiveModelReader", OPERATOR),
    ("LlmConfigService", OPERATOR),
    ("OperatorLogsService", OPERATOR),
    // WS3's secrets tightening. Implemented by COMPOSITION for the same reason
    // `OperatorStatusService` is: assembly is the only layer that may name both
    // the products-tier port and the `ironclaw_secrets` substrate behind it,
    // which is the whole point of removing the operator's direct edge
    // (PROPOSAL §8.2's product row, §12.1b).
    ("OperatorSecretValueStore", COMPOSITION),
    ("OperatorServiceLifecycleService", OPERATOR),
    ("OperatorStatusService", COMPOSITION),
];

/// The wire vocabulary that moved with the ports. Types, not traits, so the
/// one-import-path half of `reborn_product_contract_location_scan.rs` does not
/// govern them — it deliberately scopes itself to ports. Their *definition*
/// site is governed there (one home); what this list adds is that they are not
/// re-declared in `ironclaw_assistant`, which is how a "temporary compatibility
/// alias" becomes permanent.
const INVERTED_PORT_DTOS: &[&str] = &[
    "CodexLoginStart",
    "OperatorSecretValueStoreError",
    "LlmActiveSelection",
    "LlmConfigServiceError",
    "LlmConfigSnapshot",
    "LlmModelsResult",
    "LlmProbeRequest",
    "LlmProbeResult",
    "LlmProviderView",
    "NearAiAuthProvider",
    "NearAiLoginRequest",
    "NearAiLoginStart",
    "NearAiWalletLoginRequest",
    "NearAiWalletLoginResult",
    "RebornLogEntry",
    "RebornLogLevel",
    "RebornLogQueryRequest",
    "RebornLogQueryResponse",
    "RebornOperatorStatusCheck",
    "RebornOperatorStatusResponse",
    "RebornOperatorStatusSeverity",
    "RebornOperatorStatusState",
    "RebornServiceLifecycleAction",
    "RebornServiceLifecycleRequest",
    "RebornServiceLifecycleResponse",
    "RebornServiceLifecycleState",
    "SetActiveLlmRequest",
    "UpsertLlmProviderRequest",
];

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

fn defined_in(root: &Path, crate_name: &str, keywords: &[&str]) -> BTreeSet<String> {
    let mut found: BTreeMap<String, Vec<TypeDefOccurrence>> = BTreeMap::new();
    collect_type_defs(
        &crate_src(root, crate_name),
        keywords,
        &is_rust_identifier,
        &[],
        &mut found,
    );
    assert!(
        !found.is_empty(),
        "no {keywords:?} definitions discovered in {crate_name} — the walk is broken, not the crate"
    );
    found.into_keys().collect()
}

/// Every production `.rs` file under `dir`.
///
/// **Every I/O error is fatal.** A scan that silently shrinks its input passes
/// while enforcing nothing — a missing directory, an unreadable entry, or a
/// permission error must red the gate rather than thin it. (Both earlier
/// port-inversion scanners shipped with a `let Ok(..) = read_dir else {
/// return }` and had to be fixed; this one starts fatal.)
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

/// Remove `#[cfg(test)]`-gated items. Only the *production* edge is an
/// architectural fact: a test double may implement a product trait through the
/// crate's dev-dependency without the shipped artifact depending on product.
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
/// it), so an `impl` behind it is a genuine normal-dependency edge. Only
/// `#[cfg(test)]` is invisible to a shipped artifact.
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

fn traits_implemented_by(root: &Path, crate_name: &str, min_files: usize) -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&crate_src(root, crate_name), &mut files);
    assert!(
        files.len() >= min_files,
        "expected to walk {crate_name}'s source tree; found {} files, wanted at least {min_files}",
        files.len()
    );
    let mut names = BTreeSet::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
        names.extend(implemented_trait_names(&source));
    }
    assert!(
        !names.is_empty(),
        "no trait implementations found in {crate_name} — the impl reader is broken, not the crate"
    );
    names
}

// ---------------------------------------------------------------------------
// 1. The residue
// ---------------------------------------------------------------------------

#[test]
fn operator_implements_only_the_frozen_residue_of_product_defined_traits() {
    let root = workspace_root();
    let product_traits = defined_in(&root, PRODUCT, &["trait "]);
    let implemented = traits_implemented_by(&root, OPERATOR, 10);

    let found: BTreeSet<String> = implemented.intersection(&product_traits).cloned().collect();
    let frozen: BTreeSet<String> = PRODUCT_DEFINED_TRAITS_OPERATOR_STILL_IMPLEMENTS
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect();

    let mut violations = Vec::new();
    for name in found.difference(&frozen) {
        violations.push(format!(
            "{OPERATOR} implements {PRODUCT}::{name}, which re-inverts the operator -> product \
             edge. Declare the port in {PRODUCT_CONTRACTS} (PROPOSAL §6.1.3) instead of adding a \
             row here"
        ));
    }
    for name in frozen.difference(&found) {
        violations.push(format!(
            "{name} is listed as residue but {OPERATOR} no longer implements a {PRODUCT}-defined \
             trait by that name — delete its row in the same change"
        ));
    }

    assert!(
        violations.is_empty(),
        "operator port-inversion rule violated (CHECKLIST WS5, the ironclaw_operator row):\n{}",
        violations.join("\n")
    );
    // The ratchet is stated against the *declared list*, not against `found`.
    //
    // With the residue empty, `found.len() <= 0` would be a tautology on a
    // `usize` — clippy says so (`absurd_extreme_comparisons`), and a
    // decorative assertion is worse than none. What actually needs a ceiling
    // is the enumeration: the exact-match halves above already force `found`
    // to equal `frozen`, so the only way to permit a re-inverted edge is to
    // add a row here — and this makes that cost a second, deliberate edit with
    // a number in it, which is the whole update-never-relax shape.
    assert_eq!(
        PRODUCT_DEFINED_TRAITS_OPERATOR_STILL_IMPLEMENTS.len(),
        OPERATOR_PRODUCT_DEFINED_TRAIT_RESIDUE_BASELINE,
        "the product-defined trait residue is shrink-only. Adding a row means the operator -> \
         product edge came back; raising the baseline beside it is a reviewed decision that \
         belongs in the PR body, not a mechanical fix to make this test pass"
    );
}

// ---------------------------------------------------------------------------
// 2. The manifest edge
// ---------------------------------------------------------------------------

fn cargo_metadata(root: &Path) -> serde_json::Value {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON")
}

/// The operator crate's dependency on `ironclaw_assistant` is **gone, not
/// waived** — the whole point of the row.
///
/// Resolved through `cargo metadata` rather than by reading
/// `crates/ironclaw_operator/Cargo.toml` directly, for the reason CHECKLIST
/// WS2's verification row states in its own wording ("resolve the manifest via
/// `cargo metadata` rather than a literal path"): WS10 moves this crate into a
/// `product/` family directory, and a path-keyed read would then scan nothing
/// and pass. A missing package here is fatal.
///
/// It covers `dev-dependencies` and `build-dependencies` too. A dev-dependency
/// on product would not break the layer flip, but it would re-create exactly
/// the coupling this row removed — the operator crate's tests would again need
/// the 57k-line crate to compile — and it is the obvious place for the edge to
/// creep back.
#[test]
fn operator_manifest_names_no_product_dependency_under_any_kind() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let operator = packages
        .iter()
        .find(|package| package["name"].as_str() == Some(OPERATOR))
        .unwrap_or_else(|| {
            panic!(
                "{OPERATOR} is not in cargo metadata; if the crate was renamed or moved, \
                 repoint this gate in the same change"
            )
        });

    let offenders: Vec<String> = operator["dependencies"]
        .as_array()
        .expect("a package must list dependencies")
        .iter()
        .filter(|dependency| dependency["name"].as_str() == Some(PRODUCT))
        .map(|dependency| {
            format!(
                "    {OPERATOR} -> {PRODUCT} ({})",
                dependency["kind"].as_str().unwrap_or("normal")
            )
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "{OPERATOR} must not depend on {PRODUCT}: it implements product-side ports declared in \
         {PRODUCT_CONTRACTS}, and the whole point of the inversion (PROPOSAL §6.9.2) is that the \
         implementor compiles against the boundary, not against the crate it sits beside:\n{}",
        offenders.join("\n")
    );

    // Non-vacuity: prove the dependency list this test filters is real, by
    // showing the edge the crate *does* hold. Without this a metadata shape
    // change (an empty array, a renamed field) would make the assertion above
    // trivially true.
    let names: BTreeSet<&str> = operator["dependencies"]
        .as_array()
        .expect("a package must list dependencies")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str())
        .collect();
    assert!(
        names.contains(PRODUCT_CONTRACTS),
        "{OPERATOR} must depend on {PRODUCT_CONTRACTS} — it implements ports declared there. \
         Seeing neither crate in the dependency list means this scan is reading the wrong \
         package, not that the edge is clean. Found: {names:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Where the inverted ports landed
// ---------------------------------------------------------------------------

#[test]
fn inverted_ports_are_declared_in_contracts_and_implemented_by_their_owner() {
    let root = workspace_root();
    let contract_traits = defined_in(&root, PRODUCT_CONTRACTS, &["trait "]);
    let product_traits = defined_in(&root, PRODUCT, &["trait "]);
    let implemented_by: BTreeMap<&str, BTreeSet<String>> = [
        (OPERATOR, traits_implemented_by(&root, OPERATOR, 10)),
        (COMPOSITION, traits_implemented_by(&root, COMPOSITION, 20)),
    ]
    .into_iter()
    .collect();

    let mut violations = Vec::new();
    for (port, owner) in INVERTED_PORTS {
        if !contract_traits.contains(*port) {
            violations.push(format!(
                "{port} must be declared in {PRODUCT_CONTRACTS}; this row moved it there"
            ));
        }
        if product_traits.contains(*port) {
            violations.push(format!(
                "{port} is declared again in {PRODUCT} — the inversion gives it exactly one home"
            ));
        }
        let implemented = implemented_by
            .get(owner)
            .unwrap_or_else(|| panic!("{owner} has no scanned impl set; add one above"));
        if !implemented.contains(*port) {
            violations.push(format!(
                "{port} has no implementation in {owner}; if the implementor moved, move this \
                 row with it rather than deleting the pin"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "operator inverted-port placement violated (PROPOSAL §6.1.3, §6.9.2):\n{}",
        violations.join("\n")
    );
}

#[test]
fn inverted_port_dtos_are_not_re_declared_in_product() {
    let root = workspace_root();
    let contract_types = defined_in(&root, PRODUCT_CONTRACTS, &["trait ", "struct ", "enum "]);
    let product_types = defined_in(&root, PRODUCT, &["trait ", "struct ", "enum "]);

    let mut violations = Vec::new();
    for name in INVERTED_PORT_DTOS {
        if !contract_types.contains(*name) {
            violations.push(format!(
                "{name} must be defined in {PRODUCT_CONTRACTS}; the operator port vocabulary \
                 moved there with its ports"
            ));
        }
        if product_types.contains(*name) {
            violations.push(format!(
                "{name} is defined again in {PRODUCT} — a compatibility alias beside a moved DTO \
                 is how the second import path comes back"
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "operator port DTO placement violated (PROPOSAL §6.1.3):\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// 4. Scanner self-tests — the gate cannot go quietly vacuous
// ---------------------------------------------------------------------------

/// The impl reader over every shape the workspace actually contains. Without
/// this, a reader that silently matched nothing would make all three halves
/// above pass while enforcing nothing.
#[test]
fn impl_scanner_reads_the_trait_out_of_real_impl_shapes() {
    let source = r#"
        impl OperatorLogsService for OperatorLogBuffer {}
        impl ironclaw_assistant::LlmConfigService for RebornLlmConfigService {}
        impl<T> ActiveModelReader for Wrapper<T> {}
        impl<T: Iterator<Item = String>> OperatorStatusService for Nested<T> {}
        impl<F: Fn(&str) -> bool> ReturnArrowInBound for Callback<F> {}
        impl crate::llm_admin::Local<'_> for Thing {}
        impl<'a> ironclaw_product_contracts::wrapped::WrappedHeaderPort<SomeArgument>
            for SomeVeryLongConcreteTypeName<'a>
        {
        }
        impl OperatorLogBuffer { fn inherent(&self) {} }
        // impl CommentedOut for Thing {}
        #[cfg(test)]
        mod tests {
            impl TestOnlyPort for Double {}
        }
    "#;

    let found = implemented_trait_names(source);

    for expected in [
        "OperatorLogsService",
        "LlmConfigService",
        "ActiveModelReader",
        "OperatorStatusService",
        "ReturnArrowInBound",
        "Local",
        // rustfmt wraps a long header before `for`, indenting the
        // continuation. The indent is what keeps `" for "` intact as a
        // substring, so this shape reads exactly like a one-line header —
        // pinned here because "the wrap hides the impl" is the plausible
        // silent-vacuity story for the frozen-empty residue half, and it is
        // not true.
        "WrappedHeaderPort",
    ] {
        assert!(
            found.contains(expected),
            "{expected} was not read: {found:?}"
        );
    }
    assert!(
        !found.contains("TestOnlyPort"),
        "a #[cfg(test)] impl is not a production edge: {found:?}"
    );
    assert!(
        !found.contains("CommentedOut"),
        "a commented-out impl is not an edge: {found:?}"
    );
    assert!(
        !found.contains("OperatorLogBuffer"),
        "an inherent impl has no implemented trait: {found:?}"
    );
}

/// Order matters: comments and string literals are stripped **before**
/// `#[cfg(test)]` blocks, because the block walk counts raw braces. A `{` in a
/// doc comment or a literal inside a gated block would otherwise desynchronize
/// the depth counter and leak the rest of the file — including real production
/// impls — out of the scanned set, or swallow it.
#[test]
fn cfg_test_stripping_survives_braces_in_comments_and_strings() {
    let source = r#"
        #[cfg(test)]
        mod tests {
            /// A doc comment with an unbalanced { brace.
            const SHAPE: &str = "{{{";
            impl TestOnlyPort for Double {}
        }
        impl ProductionPort for RealThing {}
    "#;

    let found = implemented_trait_names(source);
    assert!(
        found.contains("ProductionPort"),
        "production code after a gated block with unbalanced braces was swallowed: {found:?}"
    );
    assert!(
        !found.contains("TestOnlyPort"),
        "the gated block leaked into the production set: {found:?}"
    );

    // The reverse composition (strip `#[cfg(test)]` first) is what this
    // ordering exists to avoid. Prove it really would have gone wrong rather
    // than asserting it — and note *which way*: the unbalanced `{` in the doc
    // comment and the `"{{{"` literal drive the depth counter up and it never
    // returns to zero, so the walk runs off the end of the input and returns
    // everything *before* the gated block. The naive order therefore
    // **swallows production code** — the silent-vacuity direction, worse than
    // leaking a test impl, because every assertion downstream still passes.
    let naive = strip_cfg_test_blocks(source);
    assert!(
        !naive.contains("ProductionPort"),
        "the ordering guard is pointless if the naive order happens to work — if this fires, \
         the brace hazard changed shape and the doc above needs rewriting"
    );
    assert!(
        !naive.contains("TestOnlyPort"),
        "the naive order is expected to swallow, not to leak; a leak here means the failure \
         mode changed and both directions need re-deriving"
    );
}

/// Every walk this file performs must find something. A path typo, a crate
/// rename, or a directory move would otherwise turn every assertion above into
/// a tautology over an empty set — the exact way a gate dies silently.
#[test]
fn every_scanned_input_is_non_empty() {
    let root = workspace_root();

    let contract_traits = defined_in(&root, PRODUCT_CONTRACTS, &["trait "]);
    let product_traits = defined_in(&root, PRODUCT, &["trait "]);
    let operator_impls = traits_implemented_by(&root, OPERATOR, 10);
    let composition_impls = traits_implemented_by(&root, COMPOSITION, 20);

    assert!(
        contract_traits.len() > 10,
        "expected {PRODUCT_CONTRACTS} to declare a real port set, found {}",
        contract_traits.len()
    );
    assert!(
        product_traits.len() > 10,
        "expected {PRODUCT} to declare a real trait set, found {}",
        product_traits.len()
    );
    assert!(
        operator_impls.len() > 3,
        "expected {OPERATOR} to implement several traits, found {}",
        operator_impls.len()
    );
    assert!(
        composition_impls.len() > 10,
        "expected {COMPOSITION} to implement many traits, found {}",
        composition_impls.len()
    );

    // The two sets this file intersects must actually overlap *somewhere* in
    // the workspace, or "the intersection is empty" would prove nothing about
    // the operator crate. Composition legitimately implements product-declared
    // traits (it is the assembly root and sits above product), so it is the
    // control that shows the intersection machinery works at all.
    let control: BTreeSet<&String> = composition_impls.intersection(&product_traits).collect();
    assert!(
        !control.is_empty(),
        "no crate in the workspace implements a {PRODUCT}-declared trait — the intersection \
         used by the residue test cannot distinguish 'clean' from 'broken'"
    );
}
