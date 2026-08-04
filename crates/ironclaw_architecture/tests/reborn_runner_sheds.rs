//! WS3 runner sheds — the model gateway and progressive tool disclosure moved
//! out of `ironclaw_runner` into `ironclaw_loop_host` (CHECKLIST WS4 "Runner
//! sheds", PROPOSAL §6.7.2/§6.7.3, `families/loop.md`'s loop_host charter
//! "owns … the model-gateway port adapter").
//!
//! A move is only finished when it is finished on *both* sides, so this gate
//! has four halves and each one fails for its own reason:
//!
//! 1. **Moved items are at the new home and gone from the old one.** Every
//!    symbol below is defined exactly once, in `ironclaw_loop_host`, and
//!    defined nowhere in `ironclaw_runner`. A half-move that leaves a copy
//!    behind, or a re-import that recreates one, fails here rather than
//!    compiling into two divergent gateways.
//! 2. **The manifest edges the move bought are actually gone**, proved through
//!    `cargo metadata` rather than by grepping a literal path — so the WS7
//!    family move (`crates/loop/ironclaw_turn_runner/…`) cannot silently blind
//!    this scan. `ironclaw_runner` no longer names `ironclaw_llm` or
//!    `ironclaw_common` under **any** dependency kind; `ironclaw_loop_host`
//!    names both.
//! 3. **The runner residue is enumerated, shrink-only, and reasoned.** What the
//!    runner still names from the moved clusters is listed with a per-entry
//!    reason. The list only shrinks.
//! 4. **The layer re-declaration that dissolved three `LAYER_MATRIX_EXCEPTIONS`
//!    stays declared.** `ironclaw_runner` and `ironclaw_hooks` are `loops`.
//!    Reverting either to `kernel`/`substrates` would make three deleted
//!    exceptions necessary again, and the exception ratchet is shrink-only — so
//!    the revert must fail *here*, loudly, rather than at a confusing
//!    "undeclared edge" message three crates away.
//!
//! Scanning discipline, in the house style: production files only (via
//! `ratchet_support::production_rust_files`, which subtracts `#[cfg(test)] mod`
//! chains as well as test-shaped filenames), comments and string literals
//! stripped **before** `#[cfg(test)]` blocks (the reverse order swallows
//! production code — see the ordering guard below), every I/O error fatal, and
//! non-vacuity asserted so a broken walk can never read as a clean tree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[allow(dead_code)]
mod ratchet_support;

use ratchet_support::{production_rust_files, strip_comments_and_strings, workspace_root};

const RUNNER: &str = "ironclaw_runner";
const LOOP_HOST: &str = "ironclaw_loop_host";
const HOOKS: &str = "ironclaw_hooks";

/// Every item the WS3 shed relocated, with the keyword that introduces its
/// definition. Each must be defined exactly once in `LOOP_HOST` and never in
/// `RUNNER`.
///
/// The list is the *shed inventory*, not "everything in the moved files": free
/// functions and one-off helpers are not named here, because pinning them would
/// turn every internal refactor of the gateway into a gate edit.
///
/// It does include five `pub(crate)` types from the tool-disclosure cluster
/// (`CapabilityCatalog`, `PromotedSet`, `DisclosureCaps`, `ActiveSet`,
/// `ToolTier`). They are not module-private helpers: they are the cluster's
/// load-bearing vocabulary, the cluster moved as a unit, and a half-move that
/// left one of them behind in the runner would compile — which is exactly the
/// failure this gate exists to catch. Visibility is not the criterion; being
/// part of the moved unit's contract is.
const MOVED_ITEMS: &[(&str, &str)] = &[
    // --- model gateway (PROPOSAL §6.7.2 "gains: runner's model-gateway adapter")
    ("struct ", "LlmProviderModelGateway"),
    ("struct ", "RoutedLlmProviderModelGateway"),
    ("struct ", "ThreadBackedLoopModelGateway"),
    ("struct ", "ThreadResolvingLoopModelGateway"),
    ("struct ", "LlmModelProfilePolicy"),
    ("struct ", "StaticModelRouteProviderPool"),
    ("trait ", "ModelRouteProviderPool"),
    // --- model routing vocabulary (had to travel: the gateway names eight of
    //     its types, so leaving it behind would have made `loop_host -> runner`
    //     a cycle against the pre-existing `runner -> loop_host` edge)
    ("enum ", "ModelSlot"),
    ("struct ", "ModelRoute"),
    ("struct ", "ActiveModelRouteSettings"),
    ("enum ", "ModelSelectionMode"),
    ("struct ", "ModelRoutePolicy"),
    ("struct ", "ModelRouteProviderKey"),
    ("enum ", "ModelRouteSource"),
    ("struct ", "ResolvedModelRouteSnapshot"),
    ("trait ", "ModelRouteResolver"),
    ("struct ", "StaticModelRouteResolver"),
    ("enum ", "ModelRouteErrorKind"),
    ("struct ", "ModelRouteError"),
    // --- driver-host port adapters (shrinks the Loop*Port implementer census
    //     that `families/loop.md` declares)
    ("struct ", "NoExtraLoopInputPort"),
    ("struct ", "HostManagedLoopCheckpointPort"),
    ("struct ", "HostManagedLoopProgressPort"),
    // --- progressive tool disclosure: a capability-surface-filtering decorator,
    //     which `families/loop.md` assigns to loop_host, plus its own switch
    ("struct ", "ToolDisclosureCapabilityDecorator"),
    ("enum ", "ToolDisclosureMode"),
    ("struct ", "CapabilityCatalog"),
    ("struct ", "PromotedSet"),
    ("struct ", "DisclosureCaps"),
    ("struct ", "ActiveSet"),
    ("enum ", "ToolTier"),
];

/// Manifest edges the shed deleted, and the crate that took them on.
///
/// `ironclaw_llm` was named by exactly three runner files (`model_gateway.rs`,
/// `model_gateway/prompt_cache_activity.rs`, `model_routes.rs`) and
/// `ironclaw_common` by exactly one (`model_gateway.rs`, for `llm_costs`), so
/// both edges left with the cluster. That is the shed's headline result: the
/// runner no longer carries the provider cone at all.
const SHED_MANIFEST_EDGES: &[&str] = &["ironclaw_llm", "ironclaw_common"];

/// What `ironclaw_runner` still names from the moved clusters, with the reason
/// it is not a defect. **Shrink-only**: an entry that stops matching is stale
/// and must be deleted; a new one is a regression that needs a reviewed row.
///
/// Every entry is a *consumer* reference through the crate's public surface —
/// the legal direction, since `runner -> loop_host` is a normal dependency and
/// (since this PR) a `loops -> loops` edge with no exception behind it.
const RUNNER_RESIDUE: &[(&str, &str)] = &[
    (
        "src/runtime.rs",
        "builds the capability-port factory: names ToolDisclosureCapabilityDecorator + \
         ToolDisclosureMode to decide whether to attach the decorator, and ModelRouteResolver \
         as a parts field. Construction of the decorator chain is the runner's declared job \
         (families/loop.md), so this is the seam working, not residue to remove.",
    ),
    (
        "src/loop_driver_host.rs",
        "composes a claimed run's ports: names ThreadResolvingLoopModelGateway, the three \
         driver-host port adapters, bridge_capability_ids (host-exempt synthetic ids), and \
         ModelRoute/ModelRouteError/ModelRouteResolver/ModelSlot for route resolution.",
    ),
];

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

fn crate_src(root: &Path, crate_name: &str) -> PathBuf {
    let directory = crate_directory(root, crate_name);
    let src = directory.join("src");
    assert!(
        src.is_dir(),
        "{crate_name} must have a src/ directory at {}",
        src.display()
    );
    src
}

/// Resolve a crate name to its directory through the manifest tree rather than
/// `crates/<name>`, so the WS7 family move (`crates/loop/ironclaw_turn_runner`)
/// relocates this gate's inputs instead of emptying them.
fn crate_directory(root: &Path, crate_name: &str) -> PathBuf {
    let mut found: Vec<PathBuf> = Vec::new();
    collect_manifest_dirs(&root.join("crates"), crate_name, &mut found);
    assert_eq!(
        found.len(),
        1,
        "expected exactly one crate directory named {crate_name} under crates/, found {found:?}. \
         Zero means this gate would scan nothing; more than one means it would scan the wrong tree."
    );
    found.remove(0)
}

fn collect_manifest_dirs(dir: &Path, crate_name: &str, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("cannot read a dir entry: {error}"));
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|name| name.to_str());
        if matches!(name, Some("target" | ".git" | "node_modules")) {
            continue;
        }
        if path.join("Cargo.toml").is_file() && name == Some(crate_name) {
            out.push(path.clone());
        }
        collect_manifest_dirs(&path, crate_name, out);
    }
}

/// Production `.rs` files of a crate, paired with their source **already
/// stripped of comments/strings and then of `#[cfg(test)]` blocks** — in that
/// order. Reversing it silently deletes production code (see the ordering
/// guard test), which is the worse failure: it makes the scan vacuous.
fn production_sources(root: &Path, crate_name: &str) -> Vec<(PathBuf, String)> {
    let src = crate_src(root, crate_name);
    let files = production_rust_files(&src);
    assert!(
        files.len() >= 5,
        "{crate_name} resolved to only {} production file(s) — the walk is broken, not the crate",
        files.len()
    );
    files
        .into_iter()
        .map(|path| {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let cleaned = strip_cfg_test_blocks(&strip_comments_and_strings(&raw));
            (path, cleaned)
        })
        .collect()
}

/// Remove `#[cfg(test)]`-gated items by brace balance. Must run **after**
/// comment/string stripping — it counts raw `{`/`}` bytes, so a brace inside a
/// comment or a string literal would desynchronise it.
///
/// Deliberately does not strip `#[cfg(feature = "test-support")]`: that
/// compiles in the `--all-features` lanes this suite runs under, so an item
/// behind it is a real definition.
fn strip_cfg_test_blocks(source: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(index) = rest.find(MARKER) {
        out.push_str(&rest[..index]);
        let after = &rest[index + MARKER.len()..];
        let Some(open) = after.find('{') else {
            // `#[cfg(test)] mod tests;` — an out-of-line declaration, no block.
            match after.find(';') {
                Some(semi) => {
                    rest = &after[semi + 1..];
                    continue;
                }
                None => return out,
            }
        };
        if let Some(semi) = after.find(';')
            && semi < open
        {
            rest = &after[semi + 1..];
            continue;
        }
        let mut depth = 0usize;
        let mut end = None;
        for (offset, byte) in after.bytes().enumerate().skip(open) {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        match end {
            Some(offset) => rest = &after[offset + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Definition sites of `name` introduced by `keyword`, in already-cleaned
/// source. Matches the *definition* form (`pub struct Foo`, `struct Foo<T>`,
/// `pub(crate) enum Foo`), not references, `impl` headers, or same-prefixed
/// neighbours (`ModelRouteErrorKind` must not satisfy `ModelRouteError`).
fn defines(cleaned: &str, keyword: &str, name: &str) -> bool {
    cleaned.lines().any(|line| {
        let line = line.trim_start();
        let Some(after_visibility) = strip_definition_visibility(line) else {
            return false;
        };
        let Some(after_keyword) = after_visibility.strip_prefix(keyword) else {
            return false;
        };
        let after_keyword = after_keyword.trim_start();
        let Some(tail) = after_keyword.strip_prefix(name) else {
            return false;
        };
        // A definition is followed by a generic list, a where/brace/paren/semi,
        // or nothing — never by another identifier character.
        !tail.starts_with(|ch: char| ch.is_alphanumeric() || ch == '_')
    })
}

/// Strip an optional visibility modifier and the `unsafe`/`auto` qualifiers a
/// trait definition may carry, returning the rest of the line.
fn strip_definition_visibility(line: &str) -> Option<&str> {
    let mut rest = line;
    if let Some(after) = rest.strip_prefix("pub") {
        rest = if let Some(open) = after.strip_prefix('(') {
            let close = open.find(')')?;
            open[close + 1..].trim_start()
        } else if after.starts_with(char::is_whitespace) {
            after.trim_start()
        } else {
            // `pubsomething` — not a visibility modifier.
            return None;
        };
    }
    for qualifier in ["unsafe ", "auto "] {
        if let Some(after) = rest.strip_prefix(qualifier) {
            rest = after.trim_start();
        }
    }
    Some(rest)
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

// ---------------------------------------------------------------------------
// 1. Moved items live at the new home and nowhere at the old one
// ---------------------------------------------------------------------------

#[test]
fn reborn_runner_shed_items_are_defined_in_loop_host_and_absent_from_the_runner() {
    let root = workspace_root();
    let loop_host = production_sources(&root, LOOP_HOST);
    let runner = production_sources(&root, RUNNER);

    let mut missing = Vec::new();
    let mut duplicated = Vec::new();
    let mut left_behind = Vec::new();

    for (keyword, name) in MOVED_ITEMS {
        let homes: Vec<&PathBuf> = loop_host
            .iter()
            .filter(|(_, cleaned)| defines(cleaned, keyword, name))
            .map(|(path, _)| path)
            .collect();
        match homes.len() {
            0 => missing.push(format!("    {keyword}{name}")),
            1 => {}
            _ => duplicated.push(format!(
                "    {keyword}{name} at {:?}",
                homes
                    .iter()
                    .map(|path| relative(path, &root))
                    .collect::<Vec<_>>()
            )),
        }

        for (path, cleaned) in &runner {
            if defines(cleaned, keyword, name) {
                left_behind.push(format!("    {keyword}{name} at {}", relative(path, &root)));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "these WS3-shed items are no longer defined in {LOOP_HOST} production source. If one was \
         renamed or deleted, update MOVED_ITEMS in the same change — a pin that stops matching is \
         a pin that stops protecting:\n{}",
        missing.join("\n")
    );
    assert!(
        duplicated.is_empty(),
        "a shed item is defined more than once in {LOOP_HOST}; one of them is a shadow copy:\n{}",
        duplicated.join("\n")
    );
    assert!(
        left_behind.is_empty(),
        "these items moved to {LOOP_HOST} with the WS3 runner sheds and must not be redefined in \
         {RUNNER}. Two definitions of a model gateway or a disclosure catalog diverge silently — \
         import from {LOOP_HOST} instead:\n{}",
        left_behind.join("\n")
    );
}

// ---------------------------------------------------------------------------
// 2. The manifest edges, through cargo metadata
// ---------------------------------------------------------------------------

fn cargo_metadata(root: &Path) -> serde_json::Value {
    let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string()))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("cargo metadata must run — a gate that cannot read the manifest graph fails");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must be valid JSON")
}

fn package<'a>(metadata: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    metadata["packages"]
        .as_array()
        .expect("cargo metadata must list packages")
        .iter()
        .find(|package| package["name"].as_str() == Some(name))
        .unwrap_or_else(|| {
            panic!("{name} must be a workspace package — a missing package is a broken scan")
        })
}

/// Dependency names of `package` across **every** kind (normal, dev, build).
///
/// Reads `dependencies[].name`, which under `cargo metadata --no-deps` is the
/// *package identity*; the manifest alias, when there is one, is the separate
/// `rename` field. So a renamed edge
/// (`llm = { package = "ironclaw_llm" }`) still reports `ironclaw_llm` here and
/// cannot hide from the assertions below — which is why this reads `name` and
/// deliberately ignores `rename`.
fn dependency_names(package: &serde_json::Value) -> BTreeSet<String> {
    package["dependencies"]
        .as_array()
        .expect("a package must carry a dependencies array")
        .iter()
        .filter_map(|dependency| dependency["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn reborn_runner_shed_manifest_edges_moved_to_loop_host() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);

    let runner_deps = dependency_names(package(&metadata, RUNNER));
    let loop_host_deps = dependency_names(package(&metadata, LOOP_HOST));

    // Non-vacuity: prove the reader found the right packages by naming an edge
    // each one certainly has. Without this, a lookup that silently returned an
    // empty set would make every assertion below pass.
    assert!(
        runner_deps.contains(LOOP_HOST),
        "{RUNNER} must still depend on {LOOP_HOST}; seeing no such edge means this scan is reading \
         the wrong package, not that the graph changed. Found: {runner_deps:?}"
    );
    assert!(
        loop_host_deps.contains("ironclaw_loop_contracts"),
        "{LOOP_HOST} must depend on ironclaw_loop_contracts; seeing no such edge means this scan \
         is reading the wrong package. Found: {loop_host_deps:?}"
    );

    for edge in SHED_MANIFEST_EDGES {
        assert!(
            !runner_deps.contains(*edge),
            "{RUNNER} must not name `{edge}` under any dependency kind: the WS3 shed moved every \
             file that used it. Re-adding the edge means behaviour came back with it — put that \
             behaviour in {LOOP_HOST} instead. Found: {runner_deps:?}"
        );
        assert!(
            loop_host_deps.contains(*edge),
            "{LOOP_HOST} must name `{edge}` — it took over the code that uses it. If this edge is \
             genuinely gone, remove it from SHED_MANIFEST_EDGES in the same change. \
             Found: {loop_host_deps:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Runner residue: exact-match, shrink-only, reasoned
// ---------------------------------------------------------------------------

/// Every moved-item identifier, for the residue scan. The keyword is irrelevant
/// here — a *reference* has no definition keyword in front of it.
fn moved_identifiers() -> BTreeSet<&'static str> {
    MOVED_ITEMS.iter().map(|(_, name)| *name).collect()
}

#[test]
fn reborn_runner_residue_of_the_shed_clusters_is_enumerated_and_shrink_only() {
    let root = workspace_root();
    let runner = production_sources(&root, RUNNER);
    let identifiers = moved_identifiers();
    // Loop-invariant: resolving the crate directory walks all of `crates/`.
    let crate_dir = crate_directory(&root, RUNNER);

    let mut found: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for (path, cleaned) in &runner {
        let mut named = BTreeSet::new();
        for identifier in &identifiers {
            if cleaned
                .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                .any(|token| token == *identifier)
            {
                named.insert(*identifier);
            }
        }
        if !named.is_empty() {
            found.insert(relative(path, &crate_dir), named);
        }
    }

    let declared: BTreeMap<&str, &str> = RUNNER_RESIDUE.iter().copied().collect();

    let new_rows: Vec<String> = found
        .iter()
        .filter(|(path, _)| !declared.contains_key(path.as_str()))
        .map(|(path, named)| format!("    (\"{path}\", \"…reason…\"),  // names {named:?}"))
        .collect();
    assert!(
        new_rows.is_empty(),
        "new {RUNNER} production files name symbols the WS3 shed moved to {LOOP_HOST}. That is \
         legal (the dependency runs that way), but it is inventory: add a row with a reason to \
         RUNNER_RESIDUE, or move the code:\n{}",
        new_rows.join("\n")
    );

    let stale: Vec<&str> = declared
        .keys()
        .copied()
        .filter(|path| !found.contains_key(*path))
        .collect();
    assert!(
        stale.is_empty(),
        "these RUNNER_RESIDUE rows no longer match anything — the list is shrink-only, so delete \
         them in the change that cleared them: {stale:?}"
    );

    let empty_reasons: Vec<&str> = RUNNER_RESIDUE
        .iter()
        .filter(|(_, reason)| reason.trim().is_empty())
        .map(|(path, _)| *path)
        .collect();
    assert!(
        empty_reasons.is_empty(),
        "every residue row carries the reason it is not a defect: {empty_reasons:?}"
    );

    // Non-vacuity: the residue scan must be able to see a reference at all.
    assert!(
        !found.is_empty(),
        "the residue scan found no reference to any moved identifier anywhere in {RUNNER} — the \
         runner still composes the decorator chain and resolves model routes, so an empty result \
         means the scanner is broken, not the tree"
    );
}

// ---------------------------------------------------------------------------
// 4. The layer re-declaration that dissolved three exceptions stays declared
// ---------------------------------------------------------------------------

#[test]
fn reborn_loop_tier_crates_declare_the_loops_layer_that_dissolved_their_exceptions() {
    let root = workspace_root();
    let metadata = cargo_metadata(&root);

    for crate_name in [RUNNER, HOOKS] {
        let layer = package(&metadata, crate_name)["metadata"]["ironclaw"]["layer"].as_str();
        assert_eq!(
            layer,
            Some("loops"),
            "{crate_name} must declare `layer = \"loops\"`. Reverting it re-creates \
             LAYER_MATRIX_EXCEPTIONS that this wave deleted (runner -> agent_loop, \
             runner -> loop_host, hooks -> wasm_limiter), and that register is shrink-only — so \
             the revert has to fail here rather than as an undeclared edge elsewhere. Found: \
             {layer:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Scanner self-tests — the guard must fail loudly on its own regressions
// ---------------------------------------------------------------------------

#[test]
fn reborn_runner_sheds_definition_scanner_reads_real_definition_shapes() {
    let sample = r##"
        pub struct LlmProviderModelGateway<P> { }
        pub(crate) enum ToolDisclosureMode { Off }
        pub(super) struct HostManagedLoopProgressPort;
        trait ModelRouteProviderPool: Send + Sync { }
        pub unsafe trait UnsafeRouteMarker { }
        pub struct ModelRouteErrorKind;
        impl LlmProviderModelGateway<()> { }
        impl ImplementedButNeverDefined for Thing { }
        pub struct ModelRouteProviderKeyBuilder;
    "##;
    let cleaned = strip_cfg_test_blocks(&strip_comments_and_strings(sample));

    // Positive: every visibility, generic, and qualifier shape.
    assert!(defines(&cleaned, "struct ", "LlmProviderModelGateway"));
    assert!(defines(&cleaned, "enum ", "ToolDisclosureMode"));
    assert!(defines(&cleaned, "struct ", "HostManagedLoopProgressPort"));
    assert!(defines(&cleaned, "trait ", "ModelRouteProviderPool"));
    assert!(defines(&cleaned, "trait ", "UnsafeRouteMarker"));

    // Negative: an `impl` header is a reference, not a definition. The name is
    // present in the fixture ONLY as an impl target, so a regression that
    // accepted impl headers would flip this — which asserting on an absent name
    // could never show.
    assert!(
        !defines(&cleaned, "struct ", "ImplementedButNeverDefined"),
        "an `impl X for Y` header is a reference; treating it as a definition \
         would let a deleted type read as present at its new home"
    );
    assert!(!defines(&cleaned, "trait ", "ImplementedButNeverDefined"));
    // Negative: a name absent from the fixture entirely.
    assert!(!defines(&cleaned, "struct ", "SomethingNeverDefined"));
    // Negative: the keyword must match the item kind.
    assert!(!defines(&cleaned, "enum ", "LlmProviderModelGateway"));
    // Negative: prefix neighbours must not satisfy each other. This is the trap
    // `ModelRouteError` / `ModelRouteErrorKind` sets in the real inventory.
    assert!(
        !defines(&cleaned, "struct ", "ModelRouteError"),
        "only `ModelRouteErrorKind` is defined here; a prefix match would let a deleted type look \
         present"
    );
    assert!(!defines(&cleaned, "struct ", "ModelRouteProviderKey"));
}

#[test]
fn reborn_runner_sheds_cfg_test_stripping_runs_after_comment_and_string_stripping() {
    // The trap is an *opening* brace hidden in a comment and a string literal
    // inside the gated block. Under the naive order those inflate the depth
    // counter, the block's real `}` never brings it back to zero, and the
    // stripper runs off the end — discarding every production item that follows.
    let source = r##"
        pub struct ProductionGateway;
        #[cfg(test)]
        mod tests {
            // an opening brace in a comment: {
            let literal = "another opening brace: {";
            pub struct TestOnlyGateway;
        }
        pub struct SecondProductionGateway;
        fn later() { }
        pub struct ThirdProductionGateway;
    "##;

    let correct = strip_cfg_test_blocks(&strip_comments_and_strings(source));
    assert!(defines(&correct, "struct ", "ProductionGateway"));
    assert!(defines(&correct, "struct ", "SecondProductionGateway"));
    assert!(defines(&correct, "struct ", "ThirdProductionGateway"));
    assert!(
        !defines(&correct, "struct ", "TestOnlyGateway"),
        "a `#[cfg(test)]` definition must never satisfy a production pin"
    );

    // The ordering guard is pointless if the naive order happens to work. Pin
    // the direction of the failure: stripping cfg(test) first desynchronises on
    // the braces hidden in the comment and the string literal, and eats the
    // production item that follows. Silent vacuity is the worse failure mode,
    // which is why the order is fixed rather than merely documented.
    let naive = strip_cfg_test_blocks(source);
    assert!(
        !defines(&naive, "struct ", "SecondProductionGateway")
            && !defines(&naive, "struct ", "ThirdProductionGateway"),
        "the ordering guard no longer demonstrates the hazard — if this fires, the brace trap \
         changed shape and the doc above needs rewriting, not deleting"
    );
    assert!(
        defines(&naive, "struct ", "ProductionGateway"),
        "the naive order should lose only what follows the gated block; losing everything would \
         mean this fixture proves something other than the ordering hazard"
    );

    // Out-of-line `#[cfg(test)] mod x;` has no block and must not eat the rest.
    let out_of_line = strip_cfg_test_blocks(&strip_comments_and_strings(
        "pub struct A;\n#[cfg(test)]\nmod tests;\npub struct B;\n",
    ));
    assert!(defines(&out_of_line, "struct ", "A"));
    assert!(defines(&out_of_line, "struct ", "B"));
}

#[test]
fn reborn_runner_sheds_every_scanned_input_is_non_empty() {
    let root = workspace_root();
    let loop_host = production_sources(&root, LOOP_HOST);
    let runner = production_sources(&root, RUNNER);

    assert!(
        loop_host.len() > 20,
        "{LOOP_HOST} scanned only {} production files",
        loop_host.len()
    );
    assert!(
        runner.len() > 10,
        "{RUNNER} scanned only {} production files",
        runner.len()
    );
    assert!(
        loop_host
            .iter()
            .map(|(_, cleaned)| cleaned.len())
            .sum::<usize>()
            > 100_000,
        "{LOOP_HOST}'s cleaned source is implausibly small — the stripper ate the tree"
    );

    // Positive control for the definition scanner over the real tree: a type
    // that has never been anywhere but loop_host. If this cannot be found, the
    // "defined at the new home" half cannot distinguish moved from missing.
    assert!(
        loop_host
            .iter()
            .any(|(_, cleaned)| defines(cleaned, "trait ", "HostManagedModelGateway")),
        "HostManagedModelGateway is {LOOP_HOST}'s own trait and must be findable by this scanner"
    );
}

#[test]
#[should_panic(expected = "cannot read")]
fn reborn_runner_sheds_unreadable_input_fails_the_scan_rather_than_disappearing_from_it() {
    let temporary = std::env::temp_dir().join(format!(
        "ironclaw-runner-sheds-io-fatality-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temporary).expect("fixture directory");
    // A dangling symlink is deterministic in a root container, where `chmod`
    // is not: root can read a 0000 file.
    #[cfg(unix)]
    std::os::unix::fs::symlink(temporary.join("nowhere"), temporary.join("dangling.rs"))
        .expect("dangling symlink fixture");
    #[cfg(not(unix))]
    panic!("cannot read: fixture unsupported on this platform");

    let files = production_rust_files(&temporary);
    for path in files {
        let _ = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    }
    let _ = std::fs::remove_dir_all(&temporary);
    panic!("cannot read: the fixture produced no unreadable file, so this test proved nothing");
}
