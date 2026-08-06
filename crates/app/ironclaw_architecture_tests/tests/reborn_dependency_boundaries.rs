// arch-exempt: large_file, crate layer boundary gate stays with existing architecture suite, plan #5852
#[allow(dead_code)]
mod ratchet_support;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    process::Command,
};

use serde_json::Value;

// Crate paths are spelled flat (`crates/ironclaw_x/…`) and RESOLVED through the
// crate inventory, so the family move (PROPOSAL §5) repoints them without a
// lockstep edit of the ~100 literals below. On today's tree resolution is the
// identity — pinned by `reborn_crate_inventory.rs`.
use ratchet_support::{
    crate_dir, crate_path, owning_crate_name, production_rust_files, resolve_crate_relative,
    strip_comments_and_strings, try_crate_directory, workspace_root,
};

#[test]
fn reborn_boundary_rules_active_crates_are_workspace_members() {
    // Regression for PR #3212 review: a boundary rule whose crate has a
    // `Cargo.toml` on disk but is missing from `cargo metadata` would
    // previously fail open in `assert_no_normal_workspace_deps`, masking
    // forbidden edges in the unregistered crate. Each active rule must
    // either name a crate that has no directory yet (future-only,
    // tolerated) or a crate that is in the workspace metadata.
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let registered = packages
        .iter()
        .filter_map(|package| package["name"].as_str().map(ToString::to_string))
        .collect::<std::collections::HashSet<_>>();

    let root = workspace_root();
    // Resolved through the crate inventory, not `crates/<name>/`: a family move
    // (PROPOSAL §5) makes the flat join miss every crate at once, and the
    // `continue` below then skips the whole check while the test still passes.
    // That fail-open is the WS10 dark-verdict shape, so the scan is counted.
    let mut checked = 0usize;
    for rule in boundary_rules() {
        let manifest = if rule.crate_name == "ironclaw" {
            crate_path(&root, "crates/ironclaw_cli/Cargo.toml")
        } else {
            match try_crate_directory(&root, rule.crate_name) {
                Ok(directory) => root.join(directory).join("Cargo.toml"),
                // A rule naming a crate with no directory (a retired-crate
                // reintroduction pin) is legitimate and tolerated.
                Err(_) => continue,
            }
        };
        if !manifest.exists() {
            continue;
        }
        checked += 1;
        assert!(
            registered.contains(rule.crate_name),
            "{} has a Cargo.toml at {} but is not registered as a workspace member; \
             add it to the root `Cargo.toml` `workspace.members` so its boundary rule \
             is actually checked",
            rule.crate_name,
            manifest.display()
        );
    }

    assert!(
        checked >= 30,
        "expected every active boundary rule's crate to resolve to a real manifest; only \
         {checked} did. A rule set that resolves to nothing checks nothing while passing \
         (docs/reborn/target-architecture/CHECKLIST.md WS10)."
    );
}

/// A `forbidden` entry must name a **package**, never a crate *directory*.
///
/// `assert_no_normal_workspace_deps` compares each entry against the dependency
/// names `cargo metadata` reports, which are package names. A crate whose
/// directory and package disagree therefore has one spelling that guards and
/// one that is inert — and the inert one fails *silently*, because a forbidden
/// entry that matches nothing simply never fires. `crates/app/ironclaw_cli/`
/// declaring `name = "ironclaw"` is the only such crate in the tree today, and
/// it is exactly the one an author reaches for by directory name.
///
/// **Entries naming crates that do not exist are legitimate and must keep
/// passing.** Around sixty forbidden entries name retired v1 crates
/// (`ironclaw_legacy`, `ironclaw_engine`, `ironclaw_gateway`, `ironclaw_tui`,
/// `ironclaw_storage`) as reintroduction pins. Those have no directory, which
/// is what separates them from a typo: this test flags only an entry that is
/// *not* a package **and** *is* a directory under `crates/`.
#[test]
fn boundary_rule_names_are_package_names_not_crate_directories() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let registered = packages
        .iter()
        .filter_map(|package| package["name"].as_str().map(ToString::to_string))
        .collect::<std::collections::HashSet<_>>();
    let root = workspace_root();

    let mut violations = Vec::new();
    let mut checked = 0usize;
    for rule in boundary_rules() {
        for forbidden in rule.forbidden {
            checked += 1;
            if registered.contains(forbidden) {
                continue;
            }
            // Inventory-resolved, not `crates/<name>/`: under a family move the
            // flat join misses every directory, every entry looks like a
            // reintroduction pin, and the directory-vs-package confusion this
            // test exists to catch goes unchecked (WS10).
            let Ok(directory) = try_crate_directory(&root, forbidden) else {
                // A reintroduction pin for a crate that no longer exists.
                continue;
            };
            let manifest = root.join(directory).join("Cargo.toml");
            if !manifest.exists() {
                continue;
            }
            let declared = std::fs::read_to_string(&manifest)
                .unwrap_or_else(|error| panic!("read {}: {error}", manifest.display()))
                .lines()
                .find_map(|line| {
                    line.trim()
                        .strip_prefix("name = \"")
                        .and_then(|rest| rest.strip_suffix('"'))
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| panic!("{} declares no package name", manifest.display()));
            violations.push(format!(
                "{}'s forbidden list names \"{forbidden}\", which is the crate DIRECTORY. \
                 Its package is \"{declared}\", and forbidden entries are matched against \
                 package names — so this entry can never fire and the edge is unguarded. \
                 Use \"{declared}\"",
                rule.crate_name
            ));
        }
    }

    assert!(
        checked > 500,
        "expected to scan every boundary rule's forbidden list; saw only {checked} entries"
    );
    assert!(
        violations.is_empty(),
        "boundary rules must name packages, not directories:\n{}",
        violations.join("\n")
    );
}

/// PROPOSAL §11.3: this is an internal-only workspace and **nothing publishes**.
///
/// The proposal recorded that as a fact plus one exception —
/// *"nothing publishes; `skills` is the one manifest missing `publish = false`
/// — fix"*. Measured 2026-08-05 the exception had grown to **three**
/// (`ironclaw_skills`, `ironclaw_common`, `ironclaw_safety`), which is what a
/// prose claim with no gate behind it does: every crate added or renamed since
/// is a fresh chance to omit the key, and the omission is invisible until
/// someone runs `cargo publish`. All three now declare it, and this test is why
/// the count cannot drift back — a new crate that forgets the key fails here
/// rather than being noticed by the next reader of the proposal.
///
/// Read from the manifests rather than from `cargo metadata`, deliberately:
/// metadata reports `publish: null` both for "key absent" and for
/// `publish = true`, so the distinction this test exists for is not in it.
#[test]
fn reborn_no_workspace_crate_is_publishable() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let mut missing = Vec::new();
    let mut checked = 0usize;
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        let manifest_path = package["manifest_path"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no manifest_path"));
        let manifest = std::fs::read_to_string(manifest_path)
            .unwrap_or_else(|error| panic!("cannot read {manifest_path}: {error}"));
        checked += 1;
        // Table-scoped: `publish` is only meaningful in `[package]`. A
        // `publish = false` under any other table (say, `[package.metadata]`)
        // is inert to cargo and must not satisfy this gate.
        let mut in_package_table = false;
        let mut declared = false;
        for line in manifest.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_package_table = trimmed == "[package]";
                continue;
            }
            if in_package_table && trimmed == "publish = false" {
                declared = true;
                break;
            }
        }
        if !declared {
            missing.push(format!("{name} at {manifest_path}"));
        }
    }

    // Fail-closed: a metadata query that returned nothing would otherwise pass
    // this test having checked no manifest at all.
    assert!(
        checked >= 60,
        "expected to check every workspace manifest; saw only {checked}"
    );
    assert!(
        missing.is_empty(),
        "every workspace package must declare `publish = false` — this workspace is \
         internal-only and nothing is released to a registry (PROPOSAL §11.3, CHECKLIST \
         WS10). Add the key to:\n{}",
        missing.join("\n")
    );
}

#[test]
fn reborn_workspace_crates_declare_layers_and_follow_layer_matrix() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let mut layers = BTreeMap::new();
    let mut invalid_workspace_package_names = Vec::new();
    let mut missing_metadata = Vec::new();
    let mut invalid_layers = Vec::new();
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !is_ironclaw_workspace_package(name) {
            invalid_workspace_package_names.push(name.to_string());
            continue;
        }

        let manifest_path = package["manifest_path"]
            .as_str()
            .unwrap_or("<unknown manifest>");
        let Some(layer) = package_layer(package) else {
            missing_metadata.push(format!("{name} at {manifest_path}"));
            continue;
        };
        if !IRONCLAW_CRATE_LAYERS.contains(&layer) {
            invalid_layers.push(format!("{name} at {manifest_path} declares `{layer}`"));
            continue;
        }
        layers.insert(name.to_string(), layer.to_string());
    }

    assert!(
        invalid_workspace_package_names.is_empty(),
        "Workspace packages must follow the IronClaw naming convention \
         (`ironclaw` or `ironclaw_*`). Invalid packages:\n{}",
        invalid_workspace_package_names.join("\n")
    );
    assert!(
        missing_metadata.is_empty(),
        "Workspace packages must declare [package.metadata.ironclaw] layer = \"...\":\n{}",
        missing_metadata.join("\n")
    );
    assert!(
        invalid_layers.is_empty(),
        "Workspace packages declare unknown IronClaw layers; valid layers are \
         {IRONCLAW_CRATE_LAYERS:?}:\n{}",
        invalid_layers.join("\n")
    );

    let mut used_exceptions = BTreeSet::new();
    let mut violations = Vec::new();
    for package in packages {
        let Some(crate_name) = package["name"].as_str() else {
            continue;
        };
        let Some(crate_layer) = layers.get(crate_name) else {
            continue;
        };
        for dependency in
            workspace_dependency_names(package).filter(|dep| is_normal_dependency(dep))
        {
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            let Some(dependency_layer) = layers.get(dependency_name) else {
                continue;
            };
            if !layer_allows_dependency(crate_layer, dependency_layer) {
                if let Some(exception) = layer_matrix_exception(crate_name, dependency_name) {
                    used_exceptions.insert((exception.crate_name, exception.dependency_name));
                    continue;
                }
                violations.push(format!(
                    "{crate_name} ({crate_layer}) must not depend on \
                     {dependency_name} ({dependency_layer})"
                ));
                continue;
            }
            if crate_name == "ironclaw_agent_loop" && *dependency_layer != "contracts" {
                if let Some(exception) = layer_matrix_exception(crate_name, dependency_name) {
                    used_exceptions.insert((exception.crate_name, exception.dependency_name));
                    continue;
                }
                violations.push(format!(
                    "ironclaw_agent_loop userland rule allows only contracts-layer normal \
                     dependencies, but it depends on {dependency_name} ({dependency_layer})"
                ));
            }
        }

        // A second pass rejecting a normal dep on a `legacy`-layer crate used to
        // sit here. It was deleted with the variant (§11.3, 2026-08-05): no
        // package has declared `layer = "legacy"` since the v1 tree went, so
        // `layers` could never hold one and the loop could not fire.
    }

    assert!(
        violations.is_empty(),
        "IronClaw crate layer matrix violations:\n{}",
        violations.join("\n")
    );

    let stale_exceptions = LAYER_MATRIX_EXCEPTIONS
        .iter()
        .filter(|exception| {
            !used_exceptions.contains(&(exception.crate_name, exception.dependency_name))
        })
        .map(|exception| {
            format!(
                "{} -> {} from {} should be removed in {}: {}",
                exception.crate_name,
                exception.dependency_name,
                exception.introduced,
                exception.removes_in,
                exception.reason
            )
        })
        .collect::<Vec<_>>();

    assert!(
        stale_exceptions.is_empty(),
        "Stale IronClaw crate layer matrix exceptions:\n{}",
        stale_exceptions.join("\n")
    );
}

#[test]
fn reborn_virtual_roots_match_storage_placement_contract() {
    let root = workspace_root();
    let path_source =
        std::fs::read_to_string(crate_path(&root, "crates/ironclaw_host_api/src/path.rs"))
            .expect("host API path source must be readable");
    let storage_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/storage-placement.md"))
            .expect("storage placement contract must be readable");
    let filesystem_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/filesystem.md"))
            .expect("filesystem contract must be readable");

    let implemented = extract_virtual_roots_const(&path_source);
    let storage = extract_storage_placement_roots(&storage_contract);
    let filesystem = extract_filesystem_namespace_roots(&filesystem_contract);

    assert_eq!(
        implemented, storage,
        "ironclaw_host_api VIRTUAL_ROOTS must match storage-placement.md canonical roots"
    );
    assert_eq!(
        filesystem, storage,
        "filesystem.md namespace roots must match storage-placement.md canonical roots"
    );
}

#[test]
fn reborn_crate_dependency_boundaries_hold() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();

    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_host_api",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| *name != "ironclaw_host_api")
            .collect::<Vec<_>>(),
    );

    // Provider-neutral memory contract: among internal ironclaw crates it may
    // depend only on `ironclaw_host_api` plus `ironclaw_prompt_envelope`, because
    // it owns prompt-safe wrapping for retrieved memory context (#5327). Enforced
    // as an allowlist (forbid every other workspace ironclaw crate) so future
    // deps — e.g. `ironclaw_turns`, `ironclaw_assistant`, `ironclaw_turn_runner`
    // — cannot silently slip past a blocklist that only names today's offenders.
    let memory_contract_allowed = [
        "ironclaw_memory",
        "ironclaw_host_api",
        "ironclaw_prompt_envelope",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_memory",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !memory_contract_allowed.contains(name))
            .collect::<Vec<_>>(),
    );
    // Native memory provider: only the contract + the host/filesystem substrate it
    // is built on, among internal ironclaw crates.
    let memory_native_allowed = [
        "ironclaw_memory_native",
        "ironclaw_host_api",
        "ironclaw_filesystem",
        "ironclaw_memory",
        "ironclaw_prompt_envelope",
        "ironclaw_safety",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_memory_native",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !memory_native_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // Canonical Reborn identity layer: it maps external identities to a stable
    // `UserId` at the bottom of the stack, so among internal ironclaw crates it
    // may depend ONLY on `ironclaw_host_api` (identity/scope newtypes),
    // `ironclaw_filesystem` (the durable substrate it persists behind) and
    // `ironclaw_product_contracts` (the `ProjectService` port its `projects`
    // module implements — added 2026-08-05 by PROPOSAL §12.13 D-Q, and by
    // exactly that one entry). Enforced as an allowlist so it can never reach
    // UPSTREAM (into `ironclaw_composition` / `ironclaw_assistant`) or onto the
    // v1 legacy enclave — the "never reach upstream" property the crate
    // guarantees. All three additions are contracts- or substrates-layer, so
    // the guarantee is unchanged in kind: D-Q widens what this crate may *name*,
    // never the direction it may point.
    let reborn_identity_allowed = [
        "ironclaw_identity",
        "ironclaw_host_api",
        "ironclaw_filesystem",
        "ironclaw_product_contracts",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_identity",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !reborn_identity_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // PROPOSAL §11.2.3 contracts purity — `ironclaw_loop_contracts`.
    //
    // The loop tier's contract crate is the typed membrane between replaceable
    // loop userland and the turn kernel. Its whole reason to exist is that
    // `ironclaw_agent_loop` can satisfy "contracts-layer deps only" through it,
    // so it may name only contracts-layer crates — and, above all, never
    // `ironclaw_turns`: the direction inverts (the kernel implements and
    // validates against these contracts, §6.1.4). An allowlist, not a
    // blocklist, so a future kernel or domain dep cannot slip past a list that
    // only names today's offenders.
    let loop_contracts_allowed = [
        "ironclaw_loop_contracts",
        "ironclaw_host_api",
        "ironclaw_common",
        "ironclaw_prompt_envelope",
        // Added by WS1.3, forced by the code and sanctioned by §8.2's contracts
        // row ("others: host_api/common ± extension_contracts"): the loop's
        // `LoopRuntimeContext` carries `Option<ChannelPresentation>` and
        // `render_presentation_hint` reads it, so when the channel
        // manifest-surface descriptors left `host_api` this edge moved with
        // them. §6.1.4's dependency list predates the extension tier existing.
        "ironclaw_extension_contracts",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_loop_contracts",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !loop_contracts_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // PROPOSAL §11.2.3 contracts purity — `ironclaw_extension_contracts`.
    //
    // The extension tier's contract crate defines the host↔extension membrane:
    // what an installable extension declares (manifest surfaces, auth recipes,
    // memory surface), what state its installation and auth account are in, and
    // the one vendor-implemented codec port. Its whole reason to exist is that
    // channel packages, lanes, `extension_host`, product, and the manager can
    // share that vocabulary without any of them importing a registry or an
    // owner (§6.1.2), so it may name only `ironclaw_host_api` — never the
    // registry crate whose DTOs it must not absorb, and never product.
    // An allowlist, not a blocklist, for the same reason as the loop tier's.
    let extension_contracts_allowed = ["ironclaw_extension_contracts", "ironclaw_host_api"];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_extension_contracts",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !extension_contracts_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // PROPOSAL §11.2.3 contracts purity — `ironclaw_product_contracts`.
    //
    // The product tier's contract crate defines the `ProductSurface` membrane
    // and the wire DTOs that cross it, so WebUI, the OpenAI-compatible adapter,
    // the operator surface, `extension_host`, and channel packages can compile
    // against the product boundary without compiling `ironclaw_assistant`
    // (§6.1.3). It may name `ironclaw_host_api` and — the one-way street
    // §6.1.3 grants explicitly, "for channel-facing DTO reuse" —
    // `ironclaw_extension_contracts`. Never product, never operator, never the
    // extension host. An allowlist, not a blocklist, for the same reason as the
    // two tiers above: a list of today's offenders cannot stop tomorrow's.
    let product_contracts_allowed = [
        "ironclaw_product_contracts",
        "ironclaw_extension_contracts",
        "ironclaw_host_api",
    ];
    assert_no_normal_workspace_deps(
        &dependencies,
        "ironclaw_product_contracts",
        workspace_ironclaw_crates(&dependencies)
            .into_iter()
            .filter(|name| !product_contracts_allowed.contains(name))
            .collect::<Vec<_>>(),
    );

    // Carrier purity — `ironclaw_host_ingress` (WS2 re-layer, #7092).
    //
    // This crate exists for one reason: to keep `axum` out of the contracts
    // tier while still letting a crate hand a prebuilt router plus its
    // `ironclaw_host_api` ingress descriptors to whoever binds the listener.
    // That is the whole charter, and it is why WS2 moved it from `products` to
    // `substrates` — at `products` it blocked `ironclaw_extension_host`'s
    // `products -> loops` flip while depending on nothing above `contracts`.
    //
    // The layer matrix alone will not hold it there. `substrates -> substrates`
    // is legal, so the matrix would happily let this crate acquire
    // `ironclaw_filesystem`, `ironclaw_secrets`, or any other domain — none of
    // which it may have, because every consumer that wants only the carrier
    // shapes would then compile that cone. An allowlist, not a blocklist, for
    // the same reason as the three contracts tiers above: a list of today's
    // offenders cannot stop tomorrow's. `families/product.md` states the same
    // rule in prose ("Never depends on: anything else, without exception").
    //
    // **Every dependency kind, not just `normal`** — unlike every other rule in
    // this function. The layer matrix checks normal deps only, and rightly so:
    // a dev-dependency on a higher layer is legal and used (`extension_host`
    // dev-depends on `ironclaw_assistant`). But this crate's charter is absolute
    // ("without exception"), and the property it protects — that a consumer
    // wanting only the carrier shapes compiles nothing else — is defeated just
    // as thoroughly by a dev- or build-dependency, which still has to resolve
    // and build. The crate has zero dev- and build-dependencies today, so this
    // costs nothing and closes the hole rather than documenting it.
    assert_host_ingress_names_no_other_workspace_crate(packages);

    for rule in boundary_rules() {
        assert_no_normal_workspace_deps(&dependencies, rule.crate_name, rule.forbidden);
    }
}

/// PROPOSAL §11.2.3's **size-ceiling** clause: *"Each contracts crate also
/// carries a checked size ceiling (a line-count ratchet raised only by explicit
/// review) alongside its purity allowlist."*
///
/// ✎ **Added 2026-08-05 (CHECKLIST WS10, §11.2.3).** The Wave 2 truth audit
/// recorded this clause as the one part of item 3 that *"does not exist
/// anywhere"* — and as the clause that would now bite, because the contracts
/// tier grew past every size a thin trait/DTO layer was supposed to have while
/// only its *dependencies* were policed. A crate cannot import a framework, and
/// could absorb an unbounded amount of logic instead; that substitution is
/// exactly what this ratchet makes visible.
///
/// **Ceilings, not equalities**, unlike this file's other ratchets — and
/// deliberately, because the two failure modes differ. A dependency allowlist
/// with slack is an unclaimed budget for the specific debt it names (#7147); a
/// line ceiling with slack just means the crate got smaller, which is the
/// direction wanted, and equality here would red the build on every routine
/// deletion. What must not happen silently is *growth*, so each ceiling sits a
/// bounded `TOLERANCE` above the measured count and a crate that eats through it
/// forces a reviewer to say so in writing.
///
/// The ceiling is also bounded from *below*: a crate more than one tolerance
/// window under its ceiling has banked slack, which is how a ratchet goes inert
/// (the exact way `composition-budget.toml`'s share ceiling did — 17.4pp of
/// slack, constraining nothing, #7151). Re-capture at every wave close.
///
/// Measured through [`production_rust_files`], the suite's single definition of
/// a production source file, so the numbers agree with every sibling gate
/// rather than with a private walk.
#[test]
fn reborn_contracts_crates_carry_a_checked_size_ceiling() {
    /// How far above the measured count each ceiling sits, and the width of
    /// the banked-slack window below it.
    const TOLERANCE: usize = 400;

    /// `(crate, production line ceiling)` — captured 2026-08-05 by running this
    /// test with every ceiling at `0` and reading the counts out of its own
    /// failure message. Never counted by eye.
    const SIZE_CEILINGS: &[(&str, usize)] = &[
        ("ironclaw_common", 3_793),
        ("ironclaw_extension_contracts", 7_727),
        // Raised 17_501 -> 18_570 by #6831 (standardized messaging framework):
        // the growth is the `messaging` vocabulary — the StandardMessagingOp
        // enum, the 12-code error taxonomy, compiled-in canonical schema/prompt
        // constants, and the test-support conformance module. Declarations
        // only; executable validation stays in ironclaw_host_runtime. Rationale
        // reviewed in the PR body's architecture-audit section.
        ("ironclaw_host_api", 18_570),
        ("ironclaw_loop_contracts", 14_479),
        // Raised 14_471 -> 15_685 by #7230 (operator inspector diagnostics):
        // the growth is neutral snapshot, update, cursor, prompt, model-call,
        // tool-execution, activity, and stats wire vocabulary plus serde and
        // bounded-field invariants. Process-local retention and sequencing
        // remain implemented by ironclaw_assistant.
        ("ironclaw_product_contracts", 15_685),
        ("ironclaw_prompt_envelope", 832),
    ];

    let root = workspace_root();
    let mut over = Vec::new();
    let mut banked = Vec::new();
    for (crate_name, ceiling) in SIZE_CEILINGS {
        let src = crate_dir(&root, crate_name).join("src");
        let files = production_rust_files(&src);
        assert!(
            !files.is_empty(),
            "no production sources found under {} — this ratchet would pass having measured \
             nothing",
            src.display()
        );
        let lines: usize = files
            .iter()
            .map(|path| {
                std::fs::read_to_string(path)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
                    .lines()
                    .count()
            })
            .sum();

        if lines > *ceiling {
            over.push(format!(
                "{crate_name}: {lines} production lines over a ceiling of {ceiling}"
            ));
        } else if ceiling.saturating_sub(lines) > TOLERANCE {
            banked.push(format!(
                "{crate_name}: {lines} production lines against a ceiling of {ceiling} \
                 ({} of slack, window is {TOLERANCE})",
                ceiling - lines
            ));
        }
    }

    assert!(
        over.is_empty(),
        "a contracts crate grew past its §11.2.3 size ceiling:\n{}\n\nA contracts crate holds \
         traits and DTOs. Growing one is how logic reaches the tier the dependency allowlist \
         above already protects — the allowlist cannot see a crate that imports nothing and \
         implements everything. Either move the growth to the crate that owns the behavior, or \
         raise the ceiling HERE with the reason in the PR body, which is what \"raised only by \
         explicit review\" means.",
        over.join("\n")
    );
    assert!(
        banked.is_empty(),
        "a contracts crate is now far enough under its ceiling that the ceiling constrains \
         nothing:\n{}\n\nRe-capture it in the PR that shrank the crate. A ratchet left above \
         what it measures is an unclaimed budget for the next unreviewed growth — the specific \
         way `composition-budget.toml`'s share ceiling went inert with 17.4pp of slack (#7151).",
        banked.join("\n")
    );
}

/// PROPOSAL §11.2.3, external half: a contracts crate never acquires a
/// framework, driver, or runtime client. The internal-dep allowlist above
/// cannot see these — every metadata helper in this file filters to `ironclaw_*`
/// names — so the denied set is asserted directly against `cargo metadata`.
///
/// One documented carve-out: `tokio` is permitted with the `rt` feature only,
/// for `CommunicationContextFetch::Spawned`, which carries a `JoinHandle` for an
/// in-flight communication-context fetch. PROPOSAL §11.2.3 phrases the tokio
/// rule as "beyond `sync`-free usage **where feasible**"; this is the one place
/// in the crate it is not, and it is pinned here so widening it is a deliberate
/// edit rather than a drift.
#[test]
fn reborn_contracts_crates_hold_no_framework_dependencies() {
    const DENIED: &[&str] = &[
        "axum",
        "deadpool",
        "deadpool-postgres",
        "hyper",
        "libsql",
        "reqwest",
        "rusqlite",
        "sqlx",
        "tokio-postgres",
        "tonic",
        "tower",
        "tower-http",
        "wasmtime",
        "wasmtime-wasi",
    ];
    const CONTRACTS_CRATES: &[&str] = &[
        "ironclaw_common",
        "ironclaw_extension_contracts",
        "ironclaw_host_api",
        "ironclaw_loop_contracts",
        "ironclaw_product_contracts",
        "ironclaw_prompt_envelope",
    ];

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");

    let mut checked = 0_usize;
    let mut violations = Vec::new();
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if !CONTRACTS_CRATES.contains(&name) {
            continue;
        }
        checked += 1;
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            if !is_normal_dependency(dependency) {
                continue;
            }
            let Some(dependency_name) = dependency["name"].as_str() else {
                continue;
            };
            if DENIED.contains(&dependency_name) {
                violations.push(format!("{name} -> {dependency_name}"));
            }
        }
    }

    assert_eq!(
        checked,
        CONTRACTS_CRATES.len(),
        "every contracts crate must be present in cargo metadata; otherwise this scan is vacuous"
    );
    assert!(
        violations.is_empty(),
        "contracts-layer crates must not depend on a framework, driver, or runtime client \
         (PROPOSAL §11.2.3):\n{}",
        violations.join("\n")
    );
}

/// Every memory *provider* crate. Both are packages under
/// `crates/extensions/packages/` since WS2; the contract crate
/// `ironclaw_memory` is deliberately not here — naming the contract is the
/// point of having one.
const MEMORY_PROVIDER_CRATES: &[&str] = &["ironclaw_memory_native", "ironclaw_memory_mem0"];

/// Crates that still take a normal dependency on a memory provider, with the
/// row that removes each one. **Shrink-only**: a stale entry fails, so this
/// list can never quietly grow back after an edge is cut.
///
/// The target (PROPOSAL §8.2, amended 2026-07-29) is *"no crate outside the
/// provider packages and the binary names a memory provider"* — composition
/// consumes the contract only, and the binary links providers. Neither
/// surviving edge is reachable by a move:
///
/// * `ironclaw_host_runtime` constructs `NativeMemoryService` itself, per
///   invocation, inside `MemoryServiceResolver` (`memory_provider.rs`) and in
///   `first_party_tools/memory.rs` + `user_profile_source.rs`. Cutting it is a
///   port inversion, not a relocation. **CHECKLIST WS3** owns the first half:
///   its row moves `host_runtime/first_party_tools/**` — the memory tools
///   included — into `extensions/ironclaw_extension_support/`.
/// * `ironclaw_composition` builds the `Arc<dyn MemoryService>` in
///   `memory_provider_factory.rs`. Moving that to the binary is the same
///   slice's second half.
///
/// WS2 deliberately did **not** paper over these by adding the providers to
/// `CONCRETE_EXTENSION_CRATES`, which would have required re-opening that
/// gate's `CONCRETE_DEPENDENCY_EXCEPTIONS` ledger — empty since DEL-7.
const MEMORY_PROVIDER_DEPENDENT_RESIDUE: &[(&str, &str, &str)] = &[
    (
        "ironclaw_host_runtime",
        "ironclaw_memory_native",
        "WS3 — first-party memory tools move to extension_support",
    ),
    (
        "ironclaw_composition",
        "ironclaw_memory_native",
        "WS3 — provider construction moves to the binary",
    ),
    (
        "ironclaw_composition",
        "ironclaw_memory_mem0",
        "WS3 — provider construction moves to the binary",
    ),
];

/// PROPOSAL §8.2: *"no crate outside the provider packages and the binary
/// names a memory provider"*.
///
/// Replaces the narrower pre-WS2 rule ("only composition names `memory_mem0`"),
/// which the 2026-07-29 amendment superseded when the providers became
/// packages: composition is supposed to consume the contract, not link a
/// provider, so a test that *blessed* composition was pinning the wrong shape.
/// This one covers both providers and holds the surviving dependents as named,
/// shrink-only residue instead.
#[test]
fn only_the_sanctioned_residue_names_a_memory_provider() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();

    // Guard against measuring nothing: the providers must be workspace members,
    // or every edge onto them is invisible here.
    for provider in MEMORY_PROVIDER_CRATES {
        assert!(
            dependencies.contains_key(*provider),
            "{provider} is not in cargo metadata, so this gate would check no edges at all. \
             If it was renamed or moved, repoint MEMORY_PROVIDER_CRATES in the same change."
        );
    }

    let mut violations = Vec::new();
    let mut used_residue = BTreeSet::new();
    for provider in MEMORY_PROVIDER_CRATES {
        for (crate_name, deps) in &dependencies {
            if crate_name == provider || crate_name == "ironclaw" {
                continue; // the provider itself, and the binary that links it
            }
            if !deps.iter().any(|dependency| dependency == provider) {
                continue;
            }
            if MEMORY_PROVIDER_DEPENDENT_RESIDUE
                .iter()
                .any(|(dependent, named, _)| dependent == crate_name && named == provider)
            {
                used_residue.insert((crate_name.clone(), (*provider).to_string()));
                continue;
            }
            violations.push(format!(
                "{crate_name} takes a normal dependency on the memory provider {provider}; \
                 only the provider packages and the binary may name one — everything else \
                 resolves memory through the provider-neutral contract (PROPOSAL §8.2)"
            ));
        }
    }
    violations.sort_unstable();
    assert!(
        violations.is_empty(),
        "memory-provider naming gate failed:\n{}",
        violations.join("\n")
    );

    let stale: Vec<String> = MEMORY_PROVIDER_DEPENDENT_RESIDUE
        .iter()
        .filter(|(dependent, provider, _)| {
            !used_residue.contains(&((*dependent).to_string(), (*provider).to_string()))
        })
        .map(|(dependent, provider, owner)| format!("{dependent} -> {provider} ({owner})"))
        .collect();
    assert!(
        stale.is_empty(),
        "stale MEMORY_PROVIDER_DEPENDENT_RESIDUE entries — the edge is gone, delete the \
         entry so the residue only ever shrinks:\n{}",
        stale.join("\n")
    );
}

#[test]
fn conversation_trusted_trigger_submitter_stays_conversation_or_composition_owned() {
    let root = workspace_root();
    let mut uses = Vec::new();
    collect_forbidden_string_uses(
        &root.join("crates"),
        "ConversationTrustedTriggerSubmitter",
        &root,
        &mut uses,
    );
    let allowed: BTreeSet<String> = [
        "crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs",
        "crates/ironclaw_conversations/src/inbound.rs",
    ]
    .into_iter()
    .map(|entry| resolve_crate_relative(&root, entry))
    .collect();
    let violations = uses
        .into_iter()
        .filter(|path| !allowed.contains(path.as_str()))
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "Conversation trusted trigger submission must stay conversations/composition-owned; \
         product adapters and capabilities must use untrusted inbound requests. \
         Unexpected call sites:\n{}",
        violations.join("\n")
    );
}

#[test]
fn conversation_trusted_trigger_submitter_stays_out_of_root_exports() {
    let root = workspace_root();
    let lib_source = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_conversations/src/lib.rs",
    ))
    .expect("conversation lib source must be readable");

    assert!(
        !lib_source.contains("ConversationTrustedTriggerSubmitter"),
        "ConversationTrustedTriggerSubmitter must not be re-exported from ironclaw_conversations; \
         composition should use the trusted_trigger_fire_submitter factory returning the trait object"
    );
}

#[test]
fn conversation_trusted_trigger_classifier_stays_out_of_root_exports() {
    let root = workspace_root();
    let lib_source = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_conversations/src/lib.rs",
    ))
    .expect("conversation lib source must be readable");

    assert!(
        !lib_source.contains("classify_trusted_trigger_inbound_error"),
        "classify_trusted_trigger_inbound_error is submitter policy and must not be re-exported \
         from ironclaw_conversations; composition-owned materialization should classify its own \
         local errors"
    );
    assert!(
        !lib_source.contains("classify_inbound_error"),
        "trusted trigger inbound classification must not be re-exported from \
         ironclaw_conversations; keep it private to conversations-owned submitter policy"
    );
    assert!(
        !lib_source.contains("TrustedTriggerInboundFailureKind"),
        "trusted trigger inbound classification types must not be re-exported from \
         ironclaw_conversations; keep them private to conversations-owned submitter policy"
    );
    assert!(
        !lib_source.contains("pub mod trusted_trigger"),
        "trusted_trigger must stay a private implementation module; root exports should name only \
         the narrow symbols downstream composition needs"
    );
}

#[test]
fn trusted_trigger_submit_request_minting_stays_worker_owned() {
    let root = workspace_root();
    let mut struct_literal_uses = Vec::new();
    collect_forbidden_string_uses(
        &root.join("crates"),
        "TrustedTriggerSubmitRequest {",
        &root,
        &mut struct_literal_uses,
    );
    let allowed_struct_literals: BTreeSet<String> = [
        "crates/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs",
        "crates/ironclaw_triggers/src/worker/ports.rs",
    ]
    .into_iter()
    .map(|entry| resolve_crate_relative(&root, entry))
    .collect();
    let struct_literal_violations = struct_literal_uses
        .into_iter()
        .filter(|path| !allowed_struct_literals.contains(path.as_str()))
        .collect::<Vec<_>>();

    assert!(
        struct_literal_violations.is_empty(),
        "TrustedTriggerSubmitRequest fields must stay private; trusted trigger requests \
         are minted by the trigger worker, not by downstream submitter callers. \
         Unexpected struct literal use:\n{}",
        struct_literal_violations.join("\n")
    );
}

#[test]
fn retired_host_trusted_ingress_token_crate_stays_removed() {
    let root = workspace_root();
    let retired_crate_name = ["ironclaw", "trusted", "ingress"].join("_");
    assert!(
        !root
            .join("crates")
            .join(&retired_crate_name)
            .join("Cargo.toml")
            .exists(),
        "a separate trusted ingress crate must stay absent; trusted trigger \
         submission is sealed by ironclaw_triggers and privately converted inside \
         ironclaw_conversations"
    );

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let package_names = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect::<BTreeSet<_>>();
    assert!(
        !package_names.contains(retired_crate_name.as_str()),
        "a separate trusted ingress crate must not be introduced as a workspace crate"
    );

    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();
    let violations = dependencies
        .iter()
        .filter_map(|(crate_name, deps)| {
            deps.iter()
                .any(|dependency| dependency == retired_crate_name.as_str())
                .then_some(crate_name.as_str())
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "a separate trusted ingress crate must not be introduced as a production dependency; \
         trusted trigger submission is now sealed by ironclaw_triggers and privately \
         converted inside ironclaw_conversations. Unexpected dependents:\n{}",
        violations.join("\n")
    );
}

/// The module that received the dissolved `ironclaw_first_party_extension_ports`
/// crate, resolved through the crate inventory like every other path here.
const DISSOLVED_PORTS_MODULE: &str = "crates/ironclaw_loop_host/src/skill_activation";

/// The dissolved crate's entire workspace dependency set, frozen as an
/// **equality** (CHECKLIST WS8, 2026-08-05).
///
/// `ironclaw_first_party_extension_ports` declared exactly these five workspace
/// dependencies plus `ironclaw_loop_host` itself, and a `BoundaryRule` enforced
/// the complement. The crate is gone — its contents live in
/// `ironclaw_loop_host` — and a crate-granular rule can no longer say anything
/// useful about them, because `loop_host` legitimately depends on nine of the
/// crates that rule forbade (`approvals`, `capabilities`, `host_runtime`,
/// `llm`, `memory`, `outbound`, `processes`, `resources`, `safety`). Dissolving
/// the crate without this test would have widened the moved code's legal reach
/// by those nine in silence: the classic "removing a redundant layer un-masks
/// behavior" trade, where the layer was backstopping an import restriction.
///
/// An **equality**, not a denylist, because the denylist form only catches the
/// crates someone thought to enumerate. Widening this set is allowed and is
/// exactly the reviewable moment: it means the skill-activation adapters have
/// grown a new dependency the dissolved crate never had.
const DISSOLVED_PORTS_MODULE_REACH: &[&str] = &[
    "ironclaw_filesystem",
    "ironclaw_host_api",
    "ironclaw_loop_contracts",
    "ironclaw_skills",
    "ironclaw_turns",
];

/// Every `ironclaw_*` crate named in code (not comments or string literals)
/// under `dir`, recursively.
fn workspace_crates_named_in_code(
    dir: &std::path::Path,
    root: &std::path::Path,
    named: &mut std::collections::BTreeMap<String, String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            workspace_crates_named_in_code(&path, root, named);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        // Fatal on unreadable: a scanner that skips a file it cannot read
        // reports an empty reach and passes while measuring nothing (WS10).
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let code = strip_comments_and_strings(&source);
        let chars: Vec<char> = code.chars().collect();
        let mut index = 0usize;
        while index < chars.len() {
            if !(chars[index] == 'i' && code_word_starts_at(&chars, index, "ironclaw_")) {
                index += 1;
                continue;
            }
            let mut end = index + "ironclaw_".len();
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            let name: String = chars[index..end].iter().collect();
            let relative = path.strip_prefix(root).unwrap_or(&path);
            named
                .entry(name)
                .or_insert_with(|| relative.display().to_string());
            index = end;
        }
    }
}

/// `word` begins at `index` and is not preceded by an identifier character.
fn code_word_starts_at(chars: &[char], index: usize, word: &str) -> bool {
    if index > 0 {
        let previous = chars[index - 1];
        if previous.is_ascii_alphanumeric() || previous == '_' {
            return false;
        }
    }
    chars[index..]
        .iter()
        .zip(word.chars())
        .filter(|(a, b)| **a == *b)
        .count()
        == word.chars().count()
}

/// ✎ WS8, 2026-08-05 — the module-granular successor to the deleted
/// `BoundaryRule { crate_name: "ironclaw_first_party_extension_ports", .. }`.
#[test]
fn dissolved_ports_module_keeps_its_crate_boundary() {
    let root = workspace_root();
    let dir = crate_path(&root, DISSOLVED_PORTS_MODULE);
    assert!(
        dir.is_dir(),
        "{DISSOLVED_PORTS_MODULE} does not resolve — the skill-activation module moved again \
         and this gate now measures nothing. Repoint it rather than deleting it."
    );

    let mut named = std::collections::BTreeMap::new();
    workspace_crates_named_in_code(&dir, &root, &mut named);
    // `ironclaw_loop_host` is the crate the module now lives in; it names
    // itself through `crate::`, so a literal self-reference would be a
    // leftover from the move.
    let permitted: std::collections::BTreeSet<&str> =
        DISSOLVED_PORTS_MODULE_REACH.iter().copied().collect();

    let unexpected: Vec<String> = named
        .iter()
        .filter(|(name, _)| !permitted.contains(name.as_str()))
        .map(|(name, first)| format!("    {name} (first seen in {first})"))
        .collect();
    assert!(
        unexpected.is_empty(),
        "the skill-activation module reaches crates the dissolved \
         `ironclaw_first_party_extension_ports` never depended on. Its old crate boundary is \
         now enforced here, at module granularity, because `ironclaw_loop_host` legitimately \
         depends on nine crates that boundary forbade — so folding the module in widened its \
         *legal* reach and only this equality keeps its *actual* reach honest. If the new \
         dependency is intended, add it to DISSOLVED_PORTS_MODULE_REACH in the same change \
         with the reason.\n{}",
        unexpected.join("\n")
    );

    let missing: Vec<&&str> = DISSOLVED_PORTS_MODULE_REACH
        .iter()
        .filter(|name| !named.contains_key(**name))
        .collect();
    assert!(
        missing.is_empty(),
        "these crates are in DISSOLVED_PORTS_MODULE_REACH but the skill-activation module no \
         longer names any of them: {missing:?}. A stale allowance is a hole waiting to be \
         reused — drop it in the change that removed the dependency."
    );
}

/// The scanner must see a rogue import and must not be fooled by prose. Without
/// this, the equality above could pass on an empty measurement.
#[test]
fn dissolved_ports_module_scanner_reads_code_not_comments() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-dissolved-ports-scan-{}",
        std::process::id()
    ));
    let src = root.join("crates/example/src/skill_activation");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    std::fs::write(
        src.join("prose.rs"),
        "//! Mentions ironclaw_host_runtime in prose only.\n\
         // and ironclaw_approvals in a line comment\n\
         fn f() { let _ = \"ironclaw_secrets\"; }\n",
    )
    .expect("prose fixture must be written");
    std::fs::write(
        src.join("rogue.rs"),
        "use ironclaw_processes::ProcessId;\nuse my_ironclaw_turns::Nope;\n",
    )
    .expect("rogue fixture must be written");

    let mut named = std::collections::BTreeMap::new();
    workspace_crates_named_in_code(&src, &root, &mut named);
    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    let found: Vec<&String> = named.keys().collect();
    assert_eq!(
        found,
        vec!["ironclaw_processes"],
        "the scanner must report the real import, ignore comment/string mentions, and not \
         match inside a longer identifier: {named:?}"
    );
}

#[test]
fn untrusted_ingress_paths_cannot_submit_host_trusted_inbound() {
    let root = workspace_root();
    let forbidden = [
        ForbiddenUse {
            pattern: "ConversationTrustedTriggerSubmitter",
            reason: "untrusted ingress paths must not construct conversation-owned trusted trigger submitters",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "trusted_trigger_fire_submitter",
            reason: "untrusted ingress paths must not build host-trusted trigger submitters",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TrustedTriggerSubmitRequest",
            reason: "untrusted ingress paths must not submit host-trusted trigger fires",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TrustedTriggerFireSubmitter",
            reason: "untrusted ingress paths must not implement host-trusted trigger submission",
            exempt: None,
        },
    ];
    let untrusted_src_roots = [
        "crates/ironclaw_capabilities/src",
        // ✎ WS8, 2026-08-05: was `crates/ironclaw_first_party_extension_ports/src`.
        // That crate dissolved into `ironclaw_loop_host`, so this narrows to the
        // module that received it rather than dropping the tree out of the guard
        // (the rest of `loop_host` is not an ingress path and is not covered
        // here — widening it to the whole crate would be a different claim).
        "crates/ironclaw_loop_host/src/skill_activation",
        "crates/extensions/ironclaw_extension_support/src",
        "crates/ironclaw_extension_contracts/src",
        // WS2.4: the manager holds the extension-management capability
        // handlers, which is exactly the shape this guard covers.
        "crates/ironclaw_extension_manager/src",
        "crates/ironclaw_host_api/src",
        "crates/ironclaw_host_runtime/src",
        "crates/ironclaw_assistant/src",
        "crates/ironclaw_product_contracts/src",
        "crates/ironclaw_webui/src",
        "crates/extensions/packages/telegram/src",
        "crates/extensions/packages/slack/src",
    ];

    let mut violations = Vec::new();
    for relative_root in untrusted_src_roots {
        let dir = crate_path(&root, relative_root);
        // A missing root is a stale entry, not a pass: silently skipping it is
        // exactly how a crate rename would drop a whole tree out of this guard
        // while the list still reads as covering it.
        assert!(
            dir.is_dir(),
            "untrusted-ingress scan root {relative_root} does not exist — if the crate was \
             renamed or removed, update this list in the same change so the guard keeps \
             scanning the real tree"
        );
        collect_forbidden_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Untrusted ingress, product, and capability paths must not submit or construct host-trusted synthetic inbound requests; \
         those operations belong to the conversations/composition boundary only:\n{}",
        violations.join("\n")
    );
}

#[test]
fn reborn_cli_binary_crate_stays_separate_from_v1_root() {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    let dependencies = packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>();
    let dependencies_all_kinds = packages
        .iter()
        .filter_map(package_dependencies_all_kinds)
        .collect::<HashMap<_, _>>();

    let root = workspace_root();
    let manifest_path = crate_path(&root, "crates/ironclaw_cli/Cargo.toml");
    assert!(
        manifest_path.exists(),
        "Reborn should ship as a separate binary crate at {}",
        manifest_path.display()
    );

    let manifest =
        std::fs::read_to_string(&manifest_path).expect("Reborn CLI manifest must be readable");
    assert!(
        manifest.contains("name = \"ironclaw\""),
        "Reborn CLI crate package name should be ironclaw"
    );
    assert!(
        manifest.contains("[[bin]]") && manifest.contains("name = \"ironclaw\""),
        "Reborn CLI crate must declare the canonical ironclaw binary explicitly"
    );

    let command_module_paths = [
        "crates/ironclaw_cli/AGENTS.md",
        "crates/ironclaw_cli/src/commands/mod.rs",
        "crates/ironclaw_cli/src/commands/completion.rs",
        "crates/ironclaw_cli/src/commands/doctor.rs",
        "crates/ironclaw_cli/src/commands/repl.rs",
        "crates/ironclaw_cli/src/commands/run.rs",
        "crates/ironclaw_cli/src/commands/serve.rs",
        "crates/ironclaw_cli/src/context.rs",
    ];
    for path in command_module_paths {
        assert!(
            crate_path(&root, path).exists(),
            "Reborn CLI commands should use an agent-friendly one-command-per-file layout; missing {path}"
        );
    }

    let agent_contract =
        std::fs::read_to_string(crate_path(&root, "crates/ironclaw_cli/AGENTS.md"))
            .expect("Reborn CLI crate-local AGENTS.md must be readable");
    for required_phrase in [
        "one command per file",
        "RebornCliContext",
        "no v1 runtime imports",
    ] {
        assert!(
            agent_contract.contains(required_phrase),
            "Reborn CLI AGENTS.md should document `{required_phrase}` for future command agents"
        );
    }

    assert_workspace_deps_exactly(
        &dependencies,
        "ironclaw",
        [
            "ironclaw_auth",
            // Same class as `ironclaw_host_api` in this list — a neutral
            // contracts-layer crate, added by WS1.3 when the extension tier's
            // vocabulary left `host_api`. The `extension` command renders the
            // public lifecycle projection through
            // `package_lifecycle::public_lifecycle_response_json`.
            "ironclaw_extension_contracts",
            "ironclaw_extension_host",
            // WS2.4. The `extension` and `ironhub` commands render the
            // extension-management command surface, which the
            // `ironclaw_extension_manager` split moved out of
            // `ironclaw_extension_host`. The edge is not new — the binary has
            // always linked that code — the split just names it honestly, and
            // both entries are product-face command rendering, not runtime
            // assembly (which still enters through
            // `ironclaw_composition`).
            "ironclaw_extension_manager",
            "ironclaw_extension_support",
            "ironclaw_host_api",
            "ironclaw_operator",
            // Same class again — the product tier's half of the neutral
            // contracts, added by WS1.4. The `models` command speaks the
            // operator LLM menu vocabulary (`operator_llm`) and the
            // `extension` command's lifecycle projection now lives here.
            "ironclaw_product_contracts",
            "ironclaw_composition",
            "ironclaw_config",
            "ironclaw_trace_commons",
            "ironclaw_webui",
            "ironclaw_slack_extension",
            "ironclaw_telegram_extension",
        ],
        "ironclaw should enter Reborn through ironclaw_composition (assembled runtime), ironclaw_operator (operator/admin control-plane), ironclaw_host_api (neutral provider DTO contracts), ironclaw_extension_contracts (the extension tier's half of those neutral contracts, since WS1.3), ironclaw_product_contracts (the product tier's half, since WS1.4), ironclaw_config (boot-config contract), ironclaw_trace_commons (contributor-side TraceCommons client extracted from the legacy monolith), ironclaw_auth (auth-owned contracts used by binary-assembled first-party credential wiring), and ironclaw_webui (host-owned WebUI serve lifecycle) — plus ironclaw_extension_host (the NativeExtensionFactory contract), ironclaw_extension_manager (the extension/ironhub command surface, since WS2.4) and concrete extension crates for the binary-assembled native factory registry (DEL-7: only the binary and tests may link concrete extension crates). Adding any other workspace crate here re-opens speculative public API access to internal Reborn types.",
    );
    assert_workspace_deps_exactly(
        &dependencies_all_kinds,
        "ironclaw_config",
        [],
        "ironclaw_config must remain a standalone boot contract crate with no IronClaw workspace dependencies of any dependency kind",
    );

    let runtime_dir = crate_path(&root, "crates/ironclaw_cli/src/runtime");
    let mut cli_runtime_source = String::new();
    let cli_runtime_files = collect_runtime_rs(&runtime_dir, &mut cli_runtime_source);
    assert!(
        cli_runtime_files > 0,
        "expected the Reborn CLI runtime module tree at {}; found no .rs files, so this scan \
         would be enforcing nothing",
        runtime_dir.display()
    );
    assert!(
        cli_runtime_source.contains("build_reborn_runtime"),
        "Reborn CLI should enter the assembled runtime through ironclaw_composition::build_reborn_runtime"
    );
    for forbidden in [
        "use ironclaw_host_runtime::",
        "use ironclaw_turn_runner::",
        "use ironclaw_threads::",
        "use ironclaw_turns::",
        "HostRuntimeServices",
        "build_default_planned_runtime",
    ] {
        assert!(
            !cli_runtime_source.contains(forbidden),
            "Reborn CLI runtime/ must not wire lower-level Reborn runtime pieces directly via `{forbidden}`; keep REPL as a UX shell over ironclaw_composition."
        );
    }
}

#[test]
fn reborn_host_runtime_services_do_not_expose_lower_substrate_handles() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_host_runtime/src/lib.rs"))
        .expect("host runtime lib.rs must be readable");
    let services = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_host_runtime/src/services.rs",
    ))
    .expect("host runtime services.rs must be readable");
    // WS3 split `obligations.rs` into `obligations/{mod,handler,staged_handoffs,
    // process_store,tests}.rs` (one module per chartered owner). This gate is
    // about the *escape hatches* the obligation code may not make public, and
    // those can now be written in any of those modules — so it scans the whole
    // directory rather than one file. The file count is asserted so a future
    // move that empties the directory fails here instead of scanning nothing
    // and reporting success (the loud-vs-silent distinction WS10 tracks).
    let obligations_dir = crate_path(&root, "crates/ironclaw_host_runtime/src/obligations");
    let mut obligations = String::new();
    let obligations_files = collect_runtime_rs(&obligations_dir, &mut obligations);
    assert!(
        obligations_files >= 4,
        "expected the host-runtime obligation owners under {} (mod + the three chartered \
         modules at minimum); found {obligations_files} .rs files, so this gate would be \
         scanning nothing",
        obligations_dir.display()
    );
    let host_runtime_contract =
        std::fs::read_to_string(root.join("docs/reborn/contracts/host-runtime.md"))
            .expect("host runtime contract must be readable");
    // WS3 re-point: the script lane merged into `ironclaw_sandbox` and no longer
    // lives in a `lib.rs`. Scanning the whole crate source tree keeps the rule
    // non-vacuous across that move (and the family `git mv` still to come) —
    // reading one hardcoded file would have gone silently green.
    let scripts = production_crate_source_tokens(&crate_path(&root, "crates/ironclaw_sandbox/src"));
    assert!(
        scripts.contains("pub struct ScriptRuntime"),
        "sandbox lane scan is vacuous: it found no script-lane source under \
         crates/ironclaw_sandbox/src (did the lane move again?)"
    );
    let scripts_manifest =
        std::fs::read_to_string(crate_path(&root, "crates/ironclaw_sandbox/Cargo.toml"))
            .expect("sandbox lane Cargo.toml must be readable");
    // WS6 module charters: the MCP lane is no longer one file. Scanning the
    // whole crate source tree keeps the rule non-vacuous across the §6.6.3
    // split (and the family `git mv` still to come) — reading `lib.rs` alone
    // would now see only the re-export list and go silently green. The scan is
    // over *production tokens* (see `production_crate_source_tokens`): a
    // comment, a string literal, or a `#[cfg(test)]` fixture naming the runtime
    // must not keep this probe green after the production runtime is gone.
    let mcp = production_crate_source_tokens(&crate_path(&root, "crates/ironclaw_mcp/src"));
    assert!(
        mcp.contains("pub struct McpRuntime<C>"),
        "MCP lane scan is vacuous: it found no MCP runtime source under \
         crates/ironclaw_mcp/src (did the lane move again?)"
    );
    let mcp_manifest = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_mcp/Cargo.toml"))
        .expect("MCP runtime Cargo.toml must be readable");

    let forbidden_lib_exports = [
        "RuntimeDispatchProcessExecutor",
        "ScriptRuntimeAdapter",
        "McpRuntimeAdapter",
        "WasmRuntimeAdapter",
    ];
    for export in forbidden_lib_exports {
        assert!(
            !lib.contains(export),
            "ironclaw_host_runtime must not re-export lower substrate handle `{export}`; upper Reborn code should enter through HostRuntimeServices::host_runtime / Arc<dyn HostRuntime>"
        );
    }

    let obligations_pub_use = extract_pub_use_block(&lib, "pub use obligations::{");
    let forbidden_obligation_exports = [
        "NetworkObligationPolicyStore",
        "RuntimeSecretInjectionStore",
        "RuntimeSecretInjectionStoreError",
    ];
    for export in forbidden_obligation_exports {
        assert!(
            !obligations_pub_use.contains(export),
            "ironclaw_host_runtime must not re-export lower substrate handoff store `{export}`; upper Reborn code should enter through HostRuntimeServices::host_runtime / Arc<dyn HostRuntime>"
        );
    }

    let forbidden_lib_accessors = [
        "pub use obligations::NetworkObligationPolicyStore",
        "pub use obligations::RuntimeSecretInjectionStore",
        "pub use obligations::RuntimeSecretInjectionStoreError",
        "pub use obligations::*",
        "pub fn with_secret_injection_store(",
        "pub fn with_network_policy_store(",
        "pub fn network(&self) -> &N",
        "pub fn secrets(&self) -> &S",
    ];
    for pattern in forbidden_lib_accessors {
        assert!(
            !lib.contains(pattern),
            "HostHttpEgressService must not expose lower substrate escape hatch `{pattern}`; keep raw network/secret/policy handoff wiring private to host-runtime composition"
        );
    }

    let forbidden_public_services = [
        "pub fn registry(",
        "pub fn filesystem(",
        "pub fn governor(",
        "pub fn authorizer(",
        "pub fn process_services(",
        "pub fn process_host(",
        "pub fn with_wasm_runtime(",
        "pub fn runtime_dispatcher(",
        "pub fn runtime_dispatcher_arc(",
        "pub fn capability_host",
        "pub fn secret_injection_store(",
        "pub fn network_policy_store(",
        "pub fn with_host_http_egress<N, SecretBackend>",
        "pub struct RuntimeDispatchProcessExecutor",
        "pub struct ScriptRuntimeAdapter",
        "pub struct McpRuntimeAdapter",
        "pub struct WasmRuntimeAdapter",
    ];
    for pattern in forbidden_public_services {
        assert!(
            !services.contains(pattern),
            "HostRuntimeServices must not expose lower substrate escape hatch `{pattern}`; keep dispatcher/capability/process handles private to the host-runtime crate"
        );
    }

    let forbidden_obligation_accessors = [
        "pub struct RuntimeSecretInjectionStore",
        "pub enum RuntimeSecretInjectionStoreError",
        "pub struct NetworkObligationPolicyStore",
        "pub fn insert(",
        "pub fn take(",
        "pub fn discard_for_capability(",
        "pub fn with_handoff_stores(",
        "pub fn with_network_policy_store(",
        "pub fn with_secret_injection_store(",
        "pub fn network_policy_store(&self)",
        "pub fn secret_injection_store(&self)",
        "pub fn staged_network_policy_present_for_diagnostics(",
        "pub fn staged_secret_present_for_diagnostics(",
    ];
    for pattern in forbidden_obligation_accessors {
        assert!(
            !obligations.contains(pattern),
            "BuiltinObligationServices and lower handoff stores must not expose lower substrate escape hatch `{pattern}`; keep secret/network handoff stores private to host-runtime composition"
        );
    }

    for required_phrase in [
        "try_with_host_http_egress",
        "low-level host-runtime/test harness escape hatches",
        "upper Reborn crates must not use them",
    ] {
        assert!(
            host_runtime_contract.contains(required_phrase),
            "host-runtime contract should document `{required_phrase}` so raw handoff store seams are not mistaken for upper Reborn APIs"
        );
    }

    let forbidden_script_lane_surface = [
        "RuntimeAdapter",
        "pub struct ScriptRuntimeAdapter",
        "pub fn script_error_kind",
    ];
    for pattern in forbidden_script_lane_surface {
        assert!(
            !scripts.contains(pattern),
            "ironclaw_sandbox must not expose host-runtime dispatcher composition surface `{pattern}`; compose script dispatch adapters inside ironclaw_host_runtime"
        );
    }

    // WS8 re-point: the `ironclaw_dispatcher` shim this used to name was
    // deleted, so guarding it would be vacuous. The invariant is unchanged and
    // now names the crate that actually owns `RuntimeDispatcher`.
    assert!(
        !scripts_manifest.contains("ironclaw_capabilities"),
        "ironclaw_sandbox must not depend on ironclaw_capabilities; script dispatcher adapters are host-runtime-private composition"
    );

    let forbidden_mcp_lane_surface = [
        "RuntimeAdapter",
        "pub struct McpRuntimeAdapter",
        "pub fn mcp_error_kind",
    ];
    for pattern in forbidden_mcp_lane_surface {
        assert!(
            !mcp.contains(pattern),
            "ironclaw_mcp must not expose host-runtime dispatcher composition surface `{pattern}`; compose MCP dispatch adapters inside ironclaw_host_runtime"
        );
    }
    // WS8 re-point, same reasoning as the scripts lane above.
    assert!(
        !mcp_manifest.contains("ironclaw_capabilities"),
        "ironclaw_mcp must not depend on ironclaw_capabilities; MCP dispatcher adapters are host-runtime-private composition"
    );
}

fn extract_pub_use_block<'a>(contents: &'a str, start_marker: &str) -> &'a str {
    let Some(start) = contents.find(start_marker) else {
        return "";
    };
    let after_start = &contents[start..];
    let Some(end) = after_start.find("};") else {
        return after_start;
    };
    &after_start[..end]
}

#[test]
fn reborn_turns_public_surface_keeps_runner_api_explicit() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_turns/src/lib.rs"))
        .expect("turns lib.rs must be readable");

    let forbidden_public_exports = [
        "pub use runner::",
        "pub use crate::runner::",
        "pub use self::runner::",
    ];
    for pattern in forbidden_public_exports {
        assert!(
            !lib.contains(pattern),
            "ironclaw_turns public prelude must not re-export trusted runner transition API `{pattern}`; adapters must import ironclaw_turns::runner explicitly"
        );
    }
}

#[test]
fn reborn_runner_llm_wiring_is_isolated() {
    let root = workspace_root();

    // WS3 runner sheds: the gateway is a host-port adapter, so it lives with the
    // other host-port adapters (PROPOSAL §6.7.2 "gains: runner's model-gateway
    // adapter"). The pin moved with it, and gained its other half: the adapter
    // must be at the new home AND absent from the old one, so a half-finished
    // move fails here instead of leaving two gateways.
    let reborn_gateway = crate_path(&root, "crates/ironclaw_loop_host/src/model_gateway.rs");
    assert!(
        reborn_gateway.exists(),
        "expected Reborn LLM gateway wiring at {}",
        reborn_gateway.display()
    );
    let reborn_gateway_source = std::fs::read_to_string(&reborn_gateway)
        .expect("Reborn model gateway source must be readable");
    assert!(
        reborn_gateway_source.contains("LlmProviderModelGateway"),
        "Reborn LLM gateway wiring should expose LlmProviderModelGateway from \
         crates/ironclaw_loop_host"
    );
    assert!(
        !crate_path(&root, "crates/ironclaw_turn_runner/src/model_gateway.rs").exists(),
        "the model gateway moved to ironclaw_loop_host (WS3 runner sheds); a file back at \
         crates/ironclaw_turn_runner/src/model_gateway.rs is a re-import, not a fix"
    );

    // Reborn crates may reuse the extracted LLM crate, but never on its default
    // terms: `ironclaw_llm`'s defaults pull in extra provider/backend features
    // (e.g. `bedrock`) Reborn doesn't need. `default-features = false` on every
    // edge is the durable invariant, keeping the Reborn stack's dependency
    // footprint minimal and explicit.
    for manifest_path in [
        // `ironclaw_turn_runner` left this list with WS3: shedding the model gateway
        // took its whole `ironclaw_llm` edge, and the rule below asserts a
        // manifest line that is no longer there.
        "crates/ironclaw_loop_host/Cargo.toml",
        "crates/ironclaw_composition/Cargo.toml",
    ] {
        let manifest = std::fs::read_to_string(crate_path(&root, manifest_path))
            .unwrap_or_else(|_| panic!("{manifest_path} must be readable"));
        let llm_dep = manifest
            .lines()
            .find(|line| line.trim_start().starts_with("ironclaw_llm = "))
            .unwrap_or_else(|| panic!("{manifest_path} must depend on ironclaw_llm"));
        assert!(
            llm_dep.contains("default-features = false"),
            "{manifest_path} must depend on `ironclaw_llm` with `default-features = false`, \
             so the Reborn stack never enables the root app's default postgres/libsql/tui \
             feature set: {llm_dep}"
        );
    }
}

#[test]
fn provider_tool_names_stay_at_model_protocol_boundaries() {
    let root = workspace_root();
    let mut uses = Vec::new();
    collect_provider_tool_name_boundary_uses(&root.join("crates"), &root, &mut uses);

    let allowed: BTreeSet<String> = [
        // Type definition and provider-wire validation.
        "crates/ironclaw_host_api/src/ids.rs",
        "crates/ironclaw_safety/src/lib.rs",
        "crates/ironclaw_safety/src/provider_validation.rs",
        // Host loop/run/thread protocol structs that preserve exact model
        // provider names for tool-result roundtrips and historical replay.
        // The provider-tool-call DTOs live in the `capability` submodule of the
        // loop-tier contract crate (WS1.2 moved `turns::run_profile/**` there).
        "crates/ironclaw_loop_contracts/src/host/capability.rs",
        "crates/ironclaw_threads/src/tool_result_reference.rs",
        // Loop support owns capability-id <-> provider-name surface snapshots,
        // synthetic provider tools, provider-call registration, and replay refs.
        "crates/ironclaw_loop_host/src/lib.rs",
        "crates/ironclaw_loop_host/src/capability_info.rs",
        "crates/ironclaw_loop_host/src/capability_port.rs",
        "crates/ironclaw_loop_host/src/capability_port/provider_validation.rs",
        "crates/ironclaw_loop_host/src/capability_port/surface_snapshot.rs",
        "crates/ironclaw_loop_host/src/external_tool_capability.rs",
        "crates/ironclaw_loop_host/src/subagent_spawn_port.rs",
        // The model gateway is the LLM wire boundary. Executor helpers may
        // rebuild provider calls only from stored replay metadata.
        "crates/ironclaw_loop_host/src/model_gateway.rs",
        "crates/ironclaw_agent_loop/src/executor/capability_helpers.rs",
        // Progressive tool disclosure is itself a model-protocol boundary: the
        // catalog/selector and the bridging decorator map provider tool names
        // (advertised, deferred, and synthetic bridge names) to/from capability
        // ids and rebuild provider calls for the resolved target.
        "crates/ironclaw_loop_host/src/tool_disclosure.rs",
        "crates/ironclaw_loop_host/src/tool_disclosure_port.rs",
        // Composition-local protocol surfaces that reconstruct provider-shaped
        // output or synthetic provider tools.
        "crates/ironclaw_composition/src/llm_admin/openai_compat_serve.rs",
        "crates/ironclaw_loop_host/src/synthetic_capability.rs",
        // Trace capture rebuilds a provider-shaped tool-call transcript from
        // stored replay metadata for the Trace Commons envelope; it moved out
        // of composition into the turn-runner observer seam (WS6, §6.10.1).
        "crates/ironclaw_turn_runner/src/trace_capture.rs",
    ]
    .into_iter()
    .map(|entry| resolve_crate_relative(&root, entry))
    .collect();
    let violations = uses
        .into_iter()
        .filter(|use_site| !allowed.contains(use_site.path.as_str()))
        .map(|use_site| {
            format!(
                "{}:{} contains `{}`",
                use_site.path, use_site.line_number, use_site.pattern
            )
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "ProviderToolName/provider_tool_name is provider-protocol identity, not canonical \
         capability identity. Product, frontend, workflow, and ordinary routing code must use \
         CapabilityId and convert only at model-provider/replay boundaries. Unexpected uses:\n{}",
        violations.join("\n")
    );
}

/// Lock the narrowed `ironclaw_turn_runner` public surface in place.
///
/// `ironclaw_turn_runner` previously exposed ~25 types as a wall of `pub use`
/// re-exports (capability resolvers, surface profile filters, milestone
/// scope/sink, model route policies, planned-driver factory helpers, the
/// loop-driver-host factory, etc.). Internal-trace audits found that **no
/// crate outside the reborn family ever named any of those items** and that
/// composition does not need them either — it imports via submodule paths
/// (`ironclaw_turn_runner::driver_registry::DriverRegistry`, etc.). The wall was
/// pure speculative public API.
///
/// This test pins the cleanup: `crates/loop/ironclaw_turn_runner/src/lib.rs` must be a
/// directory of `pub mod` declarations and nothing else. A future contributor
/// who tries to re-add the convenience `pub use` block fails this test
/// alongside the boundary rule that forbids any non-composition crate from
/// taking a normal cargo dep on `ironclaw_turn_runner`.
#[test]
fn reborn_internal_crate_keeps_directory_of_modules_lib_rs() {
    let root = workspace_root();
    let lib = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_turn_runner/src/lib.rs"))
        .expect("ironclaw_turn_runner lib.rs must be readable");

    // The forbidden re-export prefixes correspond to the original noisy
    // wall. Anyone wanting these items must reach them through a `pub mod`
    // path or (preferably) consume them through `ironclaw_composition`.
    let forbidden_reexports = [
        "pub use ironclaw_loop_host::",
        "pub use loop_driver_host::",
        "pub use milestone_events::",
        "pub use planned_driver::",
        "pub use planned_driver_factory::",
        "pub use text_loop_driver::",
        "pub use app_loop_family::",
    ];
    for forbidden in forbidden_reexports {
        assert!(
            !lib.contains(forbidden),
            "ironclaw_turn_runner/src/lib.rs must not re-export internal items via `{forbidden}`. \
             Reach them through the `pub mod` path or through ironclaw_composition. \
             See `reborn_internal_crate_keeps_directory_of_modules_lib_rs` for context."
        );
    }

    // The composition root is the sanctioned consumer of `ironclaw_turn_runner`'s
    // module paths. Confirm the run-state assembly is wired there (it would
    // otherwise have to live in the CLI or root app, which the dep rules
    // forbid).
    let composition_runtime = crate_path(&root, "crates/ironclaw_composition/src/runtime.rs");
    let composition_capability_host = crate_path(
        &root,
        "crates/ironclaw_composition/src/runtime/capability_host.rs",
    );
    assert!(
        composition_runtime.exists(),
        "expected Reborn runtime assembly at {}",
        composition_runtime.display()
    );
    assert!(
        composition_capability_host.exists(),
        "expected capability-host runtime assembly at {}",
        composition_capability_host.display()
    );
    let composition_runtime_source = std::fs::read_to_string(&composition_runtime)
        .expect("composition runtime.rs must be readable");
    let composition_runtime_sources = format!(
        "{}\n{}",
        composition_runtime_source,
        std::fs::read_to_string(&composition_capability_host)
            .expect("composition runtime/capability_host.rs must be readable")
    );
    for required in [
        "pub async fn build_reborn_runtime",
        "pub struct RebornRuntime",
        "use ironclaw_turn_runner::runtime::",
        "build_default_planned_runtime",
        "DefaultPlannedRuntimeParts",
    ] {
        assert!(
            composition_runtime_source.contains(required),
            "composition runtime.rs missing `{required}` -- the runtime assembly slice \
             must live in `ironclaw_composition` so the CLI and other \
             ingress points can avoid importing `ironclaw_turn_runner` directly."
        );
    }
    assert!(
        composition_runtime_sources.contains("use ironclaw_loop_host::")
            && composition_runtime_sources.contains("LoopCapabilityPortFactory"),
        "composition runtime module set missing loop-host capability factory wiring -- \
         the host adapter assembly may live in a runtime submodule, but it must stay inside \
         `ironclaw_composition` rather than the CLI or other ingress points."
    );
}

#[test]
fn composition_runtime_has_no_slack_output_policy() {
    let disguised_policy = r#"
        struct WorkspaceEntityMaskingGateway;

        impl HostManagedModelGateway for WorkspaceEntityMaskingGateway {
            async fn stream_model(&self, request: HostManagedModelRequest) {
                let uses_integration = request.capabilities.iter()
                    .any(|capability| capability.starts_with("slack."));
                if uses_integration {
                    response.output = ParentLoopOutput::AssistantReply(
                        response.content.replace("W0FIXTURE1", "[redacted]")
                    );
                }
            }
        }
    "#;
    assert!(
        source_has_slack_specific_model_output_policy(disguised_policy),
        "the boundary classifier must catch a renamed or moved Slack model-output decorator"
    );

    let imported_policy_installation = r#"
        use crate::{
            outbound::GenericDeliveryProvider,
            slack::entity_policy::WorkspaceEntityMaskingGateway,
        };

        async fn build_runtime(model_gateway: Arc<dyn HostManagedModelGateway>) {
            let model_gateway: Arc<dyn HostManagedModelGateway> = Arc::new(
                WorkspaceEntityMaskingGateway::new(model_gateway),
            );
        }
    "#;
    assert!(
        source_installs_slack_specific_model_output_policy(imported_policy_installation),
        "the boundary classifier must catch an imported Slack decorator at a runtime seam"
    );

    let distant_helper_policy = format!(
        r#"
            struct WorkspaceEntityPolicyGateway;
            impl HostManagedModelGateway for WorkspaceEntityPolicyGateway {{
                async fn stream_model(&self, request: HostManagedModelRequest) {{
                    self.rewrite_response(request).await
                }}
            }}
            {}
            impl WorkspaceEntityPolicyGateway {{
                fn rewrite_response(&self, response: &mut HostManagedModelResponse) {{
                    if response.capability_id.starts_with("slack.") {{
                        response.content = response.content.replace("W0FIXTURE1", "[redacted]");
                    }}
                }}
            }}
        "#,
        "\n".repeat(200)
    );
    assert!(
        source_has_slack_specific_model_output_policy(&distant_helper_policy),
        "the boundary classifier must not depend on helper proximity"
    );

    let allowed_outbound_delivery = r#"
        struct SlackOutboundDeliveryTargetProvider;

        impl OutboundDeliveryTargetProvider for SlackOutboundDeliveryTargetProvider {
            async fn deliver(&self, payload: ProductOutboundPayload) {
                self.slack_client.send(payload).await;
            }
        }
    "#;
    assert!(
        !source_has_slack_specific_model_output_policy(allowed_outbound_delivery),
        "normal integration-owned outbound delivery must remain allowed"
    );

    let co_located_but_unrelated = r#"
        struct GenericSecretMaskingGateway;
        impl HostManagedModelGateway for GenericSecretMaskingGateway {
            async fn stream_model(&self, response: HostManagedModelResponse) {
                response.content = mask_secret(response.content);
            }
        }

        struct SlackOutboundDeliveryTargetProvider;
        impl OutboundDeliveryTargetProvider for SlackOutboundDeliveryTargetProvider {
            async fn deliver(&self, payload: ProductOutboundPayload) {
                self.slack_client.send(payload).await;
            }
        }

        async fn build_runtime(model_gateway: Arc<dyn HostManagedModelGateway>) {
            let model_gateway = Arc::new(GenericSecretMaskingGateway::new(model_gateway));
        }
    "#;
    assert!(
        !source_has_slack_specific_model_output_policy(co_located_but_unrelated),
        "unrelated Slack delivery and generic model policy in one module must not false-positive"
    );
    assert!(
        !source_installs_slack_specific_model_output_policy(co_located_but_unrelated),
        "nearby Slack delivery must not taint a generic gateway installation"
    );

    let aliased_multiline_policy = r#"
        use ironclaw_loop_host::HostManagedModelGateway as ModelGateway;
        struct SlackEntityPolicyGateway;
        impl
            ModelGateway
            for SlackEntityPolicyGateway
        {
            async fn stream_model(&self, response: HostManagedModelResponse) {
                response.content = mask_slack_id(response.content);
            }
        }
    "#;
    assert!(
        source_has_slack_specific_model_output_policy(aliased_multiline_policy),
        "trait aliases and multiline impl headers must not evade the definition scan"
    );

    let interleaved_test_module = r#"
        fn production_before() {}
        #[cfg(test)]
        mod shell_tests;
        fn production_after() {}
    "#;
    let production = source_without_cfg_test_modules(interleaved_test_module);
    assert!(production.contains("production_before"));
    assert!(production.contains("production_after"));
    assert!(!production.contains("shell_tests"));

    let test_support_module = r#"
        #[cfg(any(test, feature = "test-support"))]
        mod runtime_support {
            fn feature_enabled_runtime_policy() {}
        }
    "#;
    let production = source_without_cfg_test_modules(test_support_module);
    assert!(
        production.contains("feature_enabled_runtime_policy"),
        "modules available through a production feature must remain visible to the neutrality scan"
    );

    let production_only_module = r#"
        #[cfg(not(test))]
        mod slack_host_state {
            fn production_runtime_policy() {}
        }
    "#;
    let production = source_without_cfg_test_modules(production_only_module);
    assert!(
        production.contains("production_runtime_policy"),
        "cfg(not(test)) modules are production code and must remain visible to the neutrality scan"
    );

    for production_cfg_module in [
        r#"
            #[cfg(all(unix, any(test, feature = "test-support")))]
            mod nested_test_support {
                fn production_runtime_policy() {}
            }
        "#,
        r#"
            #[cfg(all(unix, /* future,test,arm */ feature = "runtime-policy"))]
            mod commented_cfg_terms {
                fn production_runtime_policy() {}
            }
        "#,
        r#"
            #[cfg(all(unix, feature = "future,test,arm"))]
            mod comma_delimited_feature_name {
                fn production_runtime_policy() {}
            }
        "#,
    ] {
        let production = source_without_cfg_test_modules(production_cfg_module);
        assert!(
            production.contains("production_runtime_policy"),
            "only a direct positive test conjunct may make a cfg(all(...)) module test-only"
        );
    }

    let positive_test_conjunct = r#"
        #[cfg(all(unix, test))]
        mod unix_tests {
            fn test_only_policy() {}
        }
    "#;
    assert!(
        !source_without_cfg_test_modules(positive_test_conjunct).contains("test_only_policy"),
        "a direct positive test conjunct must still be removed from the production scan"
    );

    let root = workspace_root();
    let composition_sources = production_composition_sources(&root);
    let violations = composition_sources
        .iter()
        .filter_map(|(path, source)| {
            let relative = path.strip_prefix(&root).unwrap_or(path).to_string_lossy();
            let is_runtime_root = relative == "crates/ironclaw_composition/src/runtime.rs";
            let is_runtime_source =
                is_runtime_root || relative.starts_with("crates/ironclaw_composition/src/runtime/");
            let defines_policy = source_has_slack_specific_model_output_policy(source);
            let installs_policy =
                is_runtime_source && source_installs_slack_specific_model_output_policy(source);
            (defines_policy || installs_policy).then(|| {
                let kind = if defines_policy && installs_policy {
                    "defines and installs"
                } else if defines_policy {
                    "defines"
                } else {
                    "installs"
                };
                format!("{relative}: {kind} Slack-specific model-output policy")
            })
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "Reborn composition runtime must remain integration-neutral and must not \
         intercept model output with Slack-specific policy. Violations:\n{}",
        violations.join("\n")
    );

    let slack_policy_module = crate_path(
        &root,
        "crates/ironclaw_composition/src/runtime/slack_output_hygiene.rs",
    );
    assert!(
        !slack_policy_module.exists(),
        "Reborn composition must not own the Slack-specific output policy module at {}",
        slack_policy_module.display()
    );
}

fn production_composition_sources(root: &std::path::Path) -> Vec<(PathBuf, String)> {
    let composition_src = crate_path(root, "crates/ironclaw_composition/src");
    let mut paths = Vec::new();
    let mut pending = vec![composition_src];

    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|error| panic!("failed to read runtime dir entry: {error}"))
                .path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                paths.push(path);
            }
        }
    }

    paths.sort();
    paths
        .into_iter()
        .filter(|path| {
            let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
            !is_rust_test_source_path(&relative)
        })
        .map(|path| {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            (path, source_without_cfg_test_modules(&source))
        })
        .collect()
}

fn source_without_cfg_test_modules(source: &str) -> String {
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if cfg_attribute_is_test_only(trimmed) {
            let mut module_line = index + 1;
            while module_line < lines.len()
                && (lines[module_line].trim().is_empty()
                    || lines[module_line].trim_start().starts_with("#["))
            {
                module_line += 1;
            }
            if module_line < lines.len() && lines[module_line].trim_start().starts_with("mod ") {
                let module_declaration = lines[module_line];
                if module_declaration.trim_end().ends_with(';') {
                    index = module_line + 1;
                    continue;
                }

                let mut depth = 0;
                let mut saw_opening_brace = false;
                let mut in_block_comment = false;
                index = module_line;
                while index < lines.len() {
                    let code = strip_line_strings_and_comments(lines[index], &mut in_block_comment);
                    if code.contains('{') {
                        saw_opening_brace = true;
                    }
                    depth = update_brace_depth(depth, &code);
                    index += 1;
                    if saw_opening_brace && depth == 0 {
                        break;
                    }
                }
                continue;
            }
        }

        output.push(lines[index]);
        index += 1;
    }

    output.join("\n")
}

fn cfg_attribute_is_test_only(attribute: &str) -> bool {
    if !attribute.starts_with("#[cfg(") {
        return false;
    }
    let Some(expression) = attribute
        .strip_prefix("#[cfg(")
        .and_then(|value| value.strip_suffix(")]"))
        .map(str::trim)
    else {
        return false;
    };
    if expression == "test" {
        return true;
    }
    let Some(all_terms) = expression
        .strip_prefix("all")
        .map(str::trim_start)
        .and_then(|value| value.strip_prefix('('))
        .and_then(|value| value.strip_suffix(')'))
    else {
        return false;
    };
    cfg_all_has_direct_test_conjunct(all_terms)
}

fn cfg_all_has_direct_test_conjunct(all_terms: &str) -> bool {
    let bytes = all_terms.as_bytes();
    let mut index = 0;
    let mut parenthesis_depth = 0_usize;
    let mut normalized_term = Vec::new();

    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            let Some(comment_end) = nested_block_comment_end(bytes, index) else {
                return false;
            };
            index = comment_end;
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            return false;
        }
        if let Some((quote_index, hash_count)) = raw_string_delimiter(bytes, index) {
            if parenthesis_depth == 0 {
                normalized_term.push(b'"');
            }
            let Some(string_end) = raw_string_end(bytes, quote_index, hash_count) else {
                return false;
            };
            index = string_end;
            continue;
        }
        if matches!(bytes[index], b'"' | b'\'') {
            if parenthesis_depth == 0 {
                normalized_term.push(bytes[index]);
            }
            let Some(string_end) = quoted_literal_end(bytes, index, bytes[index]) else {
                return false;
            };
            index = string_end;
            continue;
        }

        match bytes[index] {
            b'(' => {
                if parenthesis_depth == 0 {
                    normalized_term.push(b'(');
                }
                parenthesis_depth += 1;
            }
            b')' => {
                let Some(next_depth) = parenthesis_depth.checked_sub(1) else {
                    return false;
                };
                parenthesis_depth = next_depth;
                if parenthesis_depth == 0 {
                    normalized_term.push(b')');
                }
            }
            b',' if parenthesis_depth == 0 => {
                if normalized_term == b"test" {
                    return true;
                }
                normalized_term.clear();
            }
            byte if parenthesis_depth == 0 && !byte.is_ascii_whitespace() => {
                normalized_term.push(byte);
            }
            _ => {}
        }
        index += 1;
    }

    parenthesis_depth == 0 && normalized_term == b"test"
}

fn nested_block_comment_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 2;
    let mut depth = 1_usize;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Some(index);
            }
        } else {
            index += 1;
        }
    }
    None
}

fn raw_string_delimiter(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    if bytes.get(index) == Some(&b'b') {
        index += 1;
    }
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let hash_start = index;
    while bytes.get(index) == Some(&b'#') {
        index += 1;
    }
    (bytes.get(index) == Some(&b'"')).then_some((index, index - hash_start))
}

fn raw_string_end(bytes: &[u8], quote_index: usize, hash_count: usize) -> Option<usize> {
    let mut index = quote_index + 1;
    while index < bytes.len() {
        if bytes[index] == b'"'
            && bytes
                .get(index + 1..index + 1 + hash_count)
                .is_some_and(|hashes| hashes.iter().all(|byte| *byte == b'#'))
        {
            return Some(index + 1 + hash_count);
        }
        index += 1;
    }
    None
}

fn quoted_literal_end(bytes: &[u8], quote_index: usize, quote: u8) -> Option<usize> {
    let mut index = quote_index + 1;
    let mut escaped = false;
    while index < bytes.len() {
        if escaped {
            escaped = false;
        } else if bytes[index] == b'\\' {
            escaped = true;
        } else if bytes[index] == quote {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn source_has_slack_specific_model_output_policy(source: &str) -> bool {
    const OUTPUT_MUTATION_MARKERS: &[&str] = &[
        "safe_text_update",
        "safe_reasoning_update",
        "safe_text_deltas",
        "safe_reasoning_deltas",
        "parentloopoutput::assistantreply",
        "response.output =",
        ".content =",
        ".content.replace",
        "redact",
        "sanitize",
        "mask",
    ];

    let impl_blocks = rust_impl_blocks(source);
    let gateway_interception_markers = gateway_trait_impl_markers(source);
    let gateway_types = impl_blocks
        .iter()
        .flat_map(|(header, _)| {
            let normalized_header = normalize_rust_whitespace(header);
            gateway_interception_markers
                .iter()
                .filter_map(move |marker| impl_type_after_marker(&normalized_header, marker))
        })
        .collect::<BTreeSet<_>>();

    gateway_types.into_iter().any(|gateway_type| {
        let owned_regions = impl_blocks
            .iter()
            .filter(|(header, _)| rust_header_mentions_type(header, &gateway_type))
            .map(|(_, body)| body.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            .to_ascii_lowercase();
        owned_regions.contains("slack")
            && OUTPUT_MUTATION_MARKERS
                .iter()
                .any(|marker| owned_regions.contains(marker))
    })
}

fn gateway_trait_impl_markers(source: &str) -> BTreeSet<String> {
    const GATEWAY_TRAITS: &[&str] = &["HostManagedModelGateway", "HostManagedModelStreamSink"];

    let normalized_source = normalize_rust_whitespace(source);
    let mut markers = GATEWAY_TRAITS
        .iter()
        .map(|trait_name| format!("{trait_name} for"))
        .collect::<BTreeSet<_>>();
    for trait_name in GATEWAY_TRAITS {
        let alias_prefix = format!("{trait_name} as ");
        let mut remainder = normalized_source.as_str();
        while let Some(index) = remainder.find(&alias_prefix) {
            let after_alias = &remainder[index + alias_prefix.len()..];
            let alias = after_alias
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !alias.is_empty() {
                markers.insert(format!("{alias} for"));
            }
            remainder = after_alias;
        }
    }
    markers
}

fn normalize_rust_whitespace(source: &str) -> String {
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn rust_impl_blocks(source: &str) -> Vec<(String, String)> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if !(trimmed == "impl" || trimmed.starts_with("impl ") || trimmed.starts_with("impl<")) {
            index += 1;
            continue;
        }

        let start = index;
        let mut depth = 0;
        let mut saw_opening_brace = false;
        let mut in_block_comment = false;
        while index < lines.len() {
            let code = strip_line_strings_and_comments(lines[index], &mut in_block_comment);
            if code.contains('{') {
                saw_opening_brace = true;
            }
            depth = update_brace_depth(depth, &code);
            index += 1;
            if saw_opening_brace && depth == 0 {
                break;
            }
        }

        let body = lines[start..index].join("\n");
        let header = body.split('{').next().unwrap_or_default().to_string();
        blocks.push((header, body));
    }

    blocks
}

fn impl_type_after_marker(header: &str, marker: &str) -> Option<String> {
    header
        .split_once(marker)
        .and_then(|(_, suffix)| {
            suffix
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .find(|token| !token.is_empty())
        })
        .map(ToString::to_string)
}

fn rust_header_mentions_type(header: &str, type_name: &str) -> bool {
    header
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| token == type_name)
}

fn source_installs_slack_specific_model_output_policy(source: &str) -> bool {
    const WRAPPER_CONSTRUCTION_MARKERS: &[&str] = &[
        "arc::new",
        "box::new",
        "::new(",
        "wrap(",
        "decorate",
        "let model_gateway",
    ];

    let (slack_import_symbols, has_slack_wildcard_import) = slack_import_symbols(source);
    model_gateway_statements(source)
        .into_iter()
        .any(|statement| {
            let lower_statement = statement.to_ascii_lowercase();
            let constructs_wrapper = WRAPPER_CONSTRUCTION_MARKERS
                .iter()
                .any(|marker| lower_statement.contains(marker));
            let names_slack_directly = lower_statement.contains("slack::")
                || lower_statement.contains("crate::slack")
                || lower_statement
                    .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                    .any(|token| token.starts_with("slack") && token.len() > "slack".len());
            let uses_slack_import = slack_import_symbols
                .iter()
                .any(|symbol| rust_header_mentions_type(&statement, symbol));
            constructs_wrapper
                && (names_slack_directly || uses_slack_import || has_slack_wildcard_import)
        })
}

fn model_gateway_statements(source: &str) -> Vec<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut statements = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("model_gateway") {
            continue;
        }

        let mut start = index;
        while start > 0 {
            let trimmed = lines[start].trim_start();
            if trimmed.starts_with("let ") || trimmed.starts_with("model_gateway =") {
                break;
            }
            let previous = lines[start - 1].trim();
            if previous.ends_with(';') || previous.ends_with('{') || previous.ends_with('}') {
                break;
            }
            start -= 1;
        }

        let mut end = index;
        while end + 1 < lines.len() && !lines[end].contains(';') {
            end += 1;
        }
        statements.insert(lines[start..=end].join("\n"));
    }
    statements.into_iter().collect()
}

fn slack_import_symbols(source: &str) -> (BTreeSet<String>, bool) {
    const IGNORED_IMPORT_TOKENS: &[&str] = &["as", "crate", "pub", "self", "slack", "super", "use"];

    let mut symbols = BTreeSet::new();
    let mut has_wildcard = false;
    for statement in rust_use_statements(source) {
        let normalized = normalize_rust_whitespace(&statement);
        let lower = normalized.to_ascii_lowercase();
        if lower.contains("slack::*") {
            has_wildcard = true;
        }

        let mut search_from = 0;
        while let Some(relative_index) = lower[search_from..].find("slack::") {
            let suffix_start = search_from + relative_index + "slack::".len();
            let suffix = &normalized[suffix_start..];
            let segment = slack_import_segment(suffix);
            for token in segment
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            {
                if token.len() > 2 && !IGNORED_IMPORT_TOKENS.contains(&token) {
                    symbols.insert(token.to_string());
                }
            }
            search_from = suffix_start;
        }

        if let Some(alias_index) = lower.find("slack as ") {
            let alias = normalized[alias_index + "slack as ".len()..]
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !alias.is_empty() {
                symbols.insert(alias.to_string());
            }
        }
    }
    (symbols, has_wildcard)
}

fn rust_use_statements(source: &str) -> Vec<String> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if !(trimmed.starts_with("use ") || trimmed.starts_with("pub use ")) {
            index += 1;
            continue;
        }

        let start = index;
        while index + 1 < lines.len() && !lines[index].contains(';') {
            index += 1;
        }
        statements.push(lines[start..=index].join("\n"));
        index += 1;
    }
    statements
}

fn slack_import_segment(suffix: &str) -> &str {
    if !suffix.starts_with('{') {
        return suffix.split([',', ';', '}']).next().unwrap_or_default();
    }

    let mut depth = 0;
    for (index, character) in suffix.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &suffix[1..index];
                }
            }
            _ => {}
        }
    }
    suffix
}

/// Lock the boot-config TOML + provider-catalog layering for the
/// standalone `ironclaw-reborn` binary.
///
/// Three properties:
///
/// 1. `ironclaw_config` continues to expose the boot-time parser
///    (`RebornConfigFile`) and the file-path accessors
///    (`RebornHome::config_file_path` / `providers_file_path`). These are
///    the surface the CLI relies on to find both files without
///    hardcoding the paths itself, and they're what shell tooling /
///    operator runbooks pattern-match on.
///
/// 2. The provider catalog file lives at `<home>/providers.json` —
///    same filename as v1's `~/.ironclaw/providers.json` so operator
///    muscle memory transfers and the same JSON editor tooling
///    applies. The boot TOML lives at `<home>/config.toml`. Changing
///    either filename breaks all existing operator-side documentation.
///
/// 3. `RebornConfigFile` rejects inline secret material at parse time.
///    The unit test in `secrets_guard` covers the patterns; this
///    boundary test asserts that the rejection path is *wired through*
///    `RebornConfigFile::validate` (file-level grep). A regression
///    that bypasses the guard for the boot file fails here loudly
///    rather than silently round-tripping a secret through git.
#[test]
fn reborn_boot_config_file_layout_is_pinned() {
    let root = workspace_root();

    let config_lib =
        std::fs::read_to_string(crate_path(&root, "crates/ironclaw_config/src/lib.rs"))
            .expect("reborn config lib.rs must be readable");
    for required_export in [
        "pub use config_file::",
        "RebornConfigFile",
        "REBORN_CONFIG_API_VERSION",
        "InlineSecretError",
    ] {
        assert!(
            config_lib.contains(required_export),
            "ironclaw_config/src/lib.rs must export `{required_export}`; \
             see reborn_boot_config_file_layout_is_pinned for context"
        );
    }

    let home_src = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_config/src/home.rs"))
        .expect("reborn config home.rs must be readable");
    for required_method in ["pub fn config_file_path", "pub fn providers_file_path"] {
        assert!(
            home_src.contains(required_method),
            "RebornHome must expose `{required_method}` so the CLI / composition can locate \
             the boot files without hardcoding paths; see \
             reborn_boot_config_file_layout_is_pinned"
        );
    }
    // File names — these match v1's `~/.ironclaw/providers.json` so the
    // same operator tooling / documentation applies.
    assert!(
        home_src.contains("\"config.toml\""),
        "boot config file name must be `config.toml`"
    );
    assert!(
        home_src.contains("\"providers.json\""),
        "provider catalog file name must be `providers.json` to match v1's filename for \
         operator-tooling compatibility"
    );

    // The boot TOML parser must wire the inline-secret guard. A
    // regression that bypasses it (e.g. a future contributor adds a
    // new section and forgets to call `reject_inline_secret`) would
    // silently allow pasted credentials through.
    let config_file_src = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_config/src/config_file.rs",
    ))
    .expect("reborn config_file.rs must be readable");
    assert!(
        config_file_src.contains("reject_inline_secret"),
        "RebornConfigFile::validate must call `reject_inline_secret` on operator-pasteable \
         fields. See `docs/reborn/contracts/secrets.md` and epic #3036's `Pitfalls & \
         Landmines` section: \"Do not bake secret material into blueprints/config.\""
    );

    // Provider-catalog load-from-path must be reachable from the operator
    // control-plane without forcing `ironclaw_config` to depend on
    // `ironclaw_llm` (which would violate _config's standalone boundary).
    // The composition crate only re-exports this surface for compatibility.
    let llm_catalog = crate_path(
        &root,
        "crates/ironclaw_operator/src/llm_admin/llm_catalog.rs",
    );
    assert!(
        llm_catalog.exists(),
        "operator must expose a catalog resolver at {} so the CLI can stitch \
         RebornConfigFile + providers.json into a RebornLlmConfig without itself \
         depending on ironclaw_llm",
        llm_catalog.display()
    );
    let llm_catalog_src = std::fs::read_to_string(&llm_catalog).expect("llm_catalog readable");
    for required in [
        "pub fn resolve_llm_selection_against_catalog",
        "pub fn resolve_against_registry",
        "ProviderRegistry::load_from_path",
    ] {
        assert!(
            llm_catalog_src.contains(required),
            "operator llm_catalog must expose `{required}` so the resolver path is \
             stable; see reborn_boot_config_file_layout_is_pinned"
        );
    }

    // `ironclaw_llm` must expose the path-overridable loader so the
    // catalog file location is selectable per-deployment (the
    // standalone Reborn binary points at $IRONCLAW_REBORN_HOME/providers.json,
    // not v1's ~/.ironclaw/providers.json).
    let llm_registry =
        std::fs::read_to_string(crate_path(&root, "crates/ironclaw_llm/src/registry.rs"))
            .expect("ironclaw_llm registry.rs must be readable");
    assert!(
        llm_registry.contains("pub fn load_from_path"),
        "ironclaw_llm::ProviderRegistry must expose `load_from_path` so callers can \
         override the user-overlay catalog path; v1 hardcoded ~/.ironclaw/providers.json \
         and the Reborn standalone needs its own home."
    );
}

/// Does this source text compile the provider catalog into itself?
///
/// Looks for an `include_str!` / `include_bytes!` whose *argument* names
/// `providers.json`, rather than for the two tokens anywhere in the same file
/// — many CLI tests legitimately name the runtime
/// `$IRONCLAW_REBORN_HOME/providers.json` and separately embed some other
/// asset, and a file-level conjunction reports those as boundary breaches.
fn embeds_provider_catalog(source: &str) -> bool {
    for macro_name in ["include_str!", "include_bytes!"] {
        let mut rest = source;
        while let Some(index) = rest.find(macro_name) {
            rest = &rest[index + macro_name.len()..];
            // The argument is the next parenthesised string literal; stop at
            // the closing paren so a later, unrelated mention cannot match.
            let Some(open) = rest.find('(') else { continue };
            let Some(close) = rest[open..].find(')') else {
                continue;
            };
            if rest[open..open + close].contains("providers.json") {
                return true;
            }
        }
    }
    false
}

/// The provider catalog is an asset of the crate that compiles it in, and no
/// other crate reaches across a package boundary to embed it.
///
/// CHECKLIST WS6: *"`llm` `providers.json` becomes a crate asset/composition
/// input + boundary rule added"*. Before that row landed, `providers.json` sat
/// at the **repository root** and was embedded from `ironclaw_llm` (20 sites)
/// and from the CLI's tests, five directories up — the "repo-root asset
/// reach-in" shape §11.2.7's scanner inventories. A root-level data file has no
/// owning crate, so nothing could say who was allowed to change it, and every
/// consumer compiled it in behind Cargo's back (a catalog edit did not
/// invalidate dependents the way a source edit does).
///
/// This rule pins the fix in both directions: the asset lives with its owner,
/// and the owner is the only crate that embeds it. Consumers that need the
/// catalog take it through `ProviderRegistry` (in-process) or
/// `ProviderRegistry::load_from_path` (deployment-selected file) — both pinned
/// by `reborn_boot_config_file_layout_is_pinned` above.
///
/// It also carries the `DEFAULT_LLM_*` drift assertions relocated out of
/// `ironclaw_cli`'s `commands::config::init` tests: the CLI is excluded
/// from depending on `ironclaw_llm` (see
/// `reborn_cli_binary_crate_stays_separate_from_v1_root`), so it mirrors three
/// catalog values as string constants and used to compare them by embedding the
/// catalog. Reading both files from disk here keeps the invariant without
/// giving the CLI a compile-time reach into another crate's tree.
#[test]
fn reborn_provider_catalog_is_owned_by_its_crate() {
    let root = workspace_root();

    // 1. The catalog lives with the crate that compiles it in.
    let catalog_path = crate_path(&root, "crates/ironclaw_llm/assets/providers.json");
    assert!(
        catalog_path.exists(),
        "the provider catalog must be `ironclaw_llm`'s own asset at {} (CHECKLIST WS6)",
        catalog_path.display()
    );
    assert!(
        !root.join("providers.json").exists(),
        "`providers.json` must not sit at the repository root: a root-level data file has no \
         owning crate, so no boundary rule can govern who edits it, and every consumer embeds \
         it with an escaping `include_str!`. It belongs at crates/ironclaw_llm/assets/."
    );

    // 2. `ironclaw_llm` is the only crate that embeds it. Scanning source text
    //    (rather than trusting the manifest) is what makes this catch the
    //    reach-in shape — an `include_str!` needs no dependency edge at all,
    //    which is exactly why it can cross a boundary unnoticed.
    let mut embedders: Vec<String> = Vec::new();
    let mut scanned_files = 0usize;
    let mut collect = |dir: std::path::PathBuf| {
        let mut stack = vec![dir];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) != Some("target") {
                        stack.push(path);
                    }
                    continue;
                }
                if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                scanned_files += 1;
                // Match the *call site*, not the file: plenty of tests mention
                // the runtime `$IRONCLAW_REBORN_HOME/providers.json` and also
                // happen to `include_str!` something unrelated. A file-level
                // conjunction reports those as boundary breaches.
                if !embeds_provider_catalog(&source) {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                // Ownership by crate NAME, not by a `crates/<name>/` prefix:
                // the family move (PROPOSAL §5) put the catalog's owner at
                // `crates/domains/ironclaw_llm/`, and a prefix test then reads
                // the crate's own file as a cross-package reach (WS10).
                let owner = owning_crate_name(&root, &path);
                if owner != "ironclaw_llm" && owner != "ironclaw_architecture_tests" {
                    embedders.push(relative);
                }
            }
        }
    };
    collect(root.join("crates"));
    collect(root.join("tests"));

    // A path-keyed walker that silently walks nothing is the failure mode this
    // program has hit nine times; prove it saw the tree.
    assert!(
        scanned_files > 500,
        "the catalog-embedder scan walked only {scanned_files} Rust files — it is not \
         reaching the workspace and would pass no matter what the tree contained"
    );
    assert!(
        embedders.is_empty(),
        "only `ironclaw_llm` may embed its own provider catalog; these crates reach across a \
         package boundary to `include_str!` it, which compiles another crate's asset into \
         their own tree (§11.2.7): {embedders:#?}. Take the catalog through \
         `ProviderRegistry` / `ProviderRegistry::load_from_path` instead."
    );

    // 3. The CLI's mirrored default-LLM constants still match the catalog.
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&catalog_path).expect("catalog readable"))
            .expect("providers.json must parse as JSON");
    let init_rs = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_cli/src/commands/config/init.rs",
    ))
    .expect("CLI config init.rs must be readable");

    // Pull each `const NAME: &str = "value";` out of the CLI source. If the
    // constant is renamed or restructured the extraction fails loudly rather
    // than vacuously passing on zero matches.
    let const_value = |name: &str| -> String {
        let needle = format!("const {name}: &str = \"");
        let start = init_rs.split(&needle).nth(1).unwrap_or_else(|| {
            panic!(
                "`{name}` is no longer declared as a plain `const {name}: &str = \"…\";` in the \
                 CLI's config init module. This test extracts it textually — update the \
                 extraction rather than deleting the drift check."
            )
        });
        start
            .split('"')
            .next()
            .expect("constant literal must terminate")
            .to_string()
    };

    let provider_id = const_value("DEFAULT_LLM_PROVIDER_ID");
    let entry = catalog
        .as_array()
        .expect("providers.json is a JSON array")
        .iter()
        .find(|entry| entry.get("id").and_then(|id| id.as_str()) == Some(provider_id.as_str()))
        .unwrap_or_else(|| panic!("providers.json has no `{provider_id}` entry"));

    for (const_name, catalog_field) in [
        ("DEFAULT_LLM_MODEL", "default_model"),
        ("DEFAULT_LLM_API_KEY_ENV", "api_key_env"),
    ] {
        assert_eq!(
            entry.get(catalog_field).and_then(|v| v.as_str()),
            Some(const_value(const_name).as_str()),
            "`{const_name}` in crates/ironclaw_cli/src/commands/config/init.rs has \
             drifted from the `{provider_id}` entry's `{catalog_field}` in \
             crates/ironclaw_llm/assets/providers.json. The CLI mirrors these values as \
             constants because it may not depend on `ironclaw_llm`; when the catalog changes, \
             the mirror must change with it."
        );
    }
}

#[test]
fn reborn_turns_public_surface_uses_turn_ids_not_runtime_or_process_ids() {
    let root = workspace_root();
    let turns_src = crate_path(&root, "crates/ironclaw_turns/src");
    let mut violations = Vec::new();
    collect_forbidden_turns_identifier_uses(&turns_src, &root, &mut violations);

    assert!(
        violations.is_empty(),
        "ironclaw_turns public API must use TurnId/TurnRunId instead of lower runtime/process identifiers:\n{}",
        violations.join("\n")
    );
}

#[test]
fn wasm_sandbox_core_module_stays_domain_free_v1_parity_kernel() {
    let workspace = workspace_root();
    let module = crate_path(&workspace, "crates/ironclaw_wasm/src/wasm_sandbox_core.rs");
    assert!(
        module.exists(),
        "shared WASM sandbox core should stay as a module inside ironclaw_wasm after W2.3"
    );
    let guardrails =
        std::fs::read_to_string(crate_path(&workspace, "crates/ironclaw_wasm/AGENTS.md"))
            .expect("ironclaw_wasm guardrails must be readable");
    assert!(
        guardrails.contains("wasm_sandbox_core")
            && guardrails.contains("Do not put ProductAdapter"),
        "ironclaw_wasm guardrails must preserve the folded sandbox-core domain-free rule"
    );

    let source = std::fs::read_to_string(&module).expect("WASM sandbox core module is readable");
    for forbidden in [
        "ironclaw_assistant",
        // WS8 re-point: was `ironclaw_dispatcher` until that shim was deleted.
        "ironclaw_capabilities",
        "ironclaw_extension_registry",
        "ironclaw_filesystem",
        "ironclaw_network",
        "ironclaw_secrets",
        "ironclaw_host_runtime",
        "ironclaw_composition",
    ] {
        assert!(
            !source.contains(forbidden),
            "folded WASM sandbox core module must stay independent of product/runtime/app crates; \
             unexpected reference to `{forbidden}`"
        );
    }

    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    assert!(
        packages
            .iter()
            .all(|package| package["name"] != "ironclaw_wasm_sandbox_core"),
        "ironclaw_wasm_sandbox_core should remain folded into ironclaw_wasm after W2.3"
    );

    let limiter_package = packages
        .iter()
        .find(|package| package["name"] == "ironclaw_wasm_limiter")
        .expect("ironclaw_wasm_limiter must be a workspace package");
    let limiter_workspace_deps = workspace_dependency_names(limiter_package)
        .filter_map(|dependency| dependency["name"].as_str())
        .collect::<Vec<_>>();
    assert!(
        limiter_workspace_deps.is_empty(),
        "ironclaw_wasm_limiter is allowed only as low-level WASM accounting; \
         got workspace deps: {limiter_workspace_deps:?}"
    );
}

#[test]
fn reborn_runtime_http_egress_has_single_network_boundary() {
    let forbidden = [
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::Client",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::blocking::Client",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reqwest::ClientBuilder",
            reason: "runtime crates must use ironclaw_network for outbound HTTP transport",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "ToSocketAddrs",
            reason: "runtime crates must not perform ad-hoc DNS resolution",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: ".to_socket_addrs(",
            reason: "runtime crates must not perform ad-hoc DNS resolution",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "ssrf_safe_client_builder",
            reason: "runtime crates must not reuse V1 WASM SSRF helpers",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "validate_and_resolve_http_target",
            reason: "runtime crates must not reuse V1 WASM SSRF helpers",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "reject_private_ip",
            reason: "runtime crates must not perform ad-hoc SSRF checks",
        },
        ForbiddenRuntimeNetworkUse {
            pattern: "is_private_or_loopback_ip",
            reason: "runtime crates must not perform ad-hoc private-IP checks",
        },
    ];

    let root = workspace_root();
    let runtime_src_roots = [
        "crates/ironclaw_wasm/src",
        "crates/ironclaw_sandbox/src",
        "crates/ironclaw_mcp/src",
        "crates/ironclaw_host_runtime/src",
    ];

    let mut violations = Vec::new();
    for relative_root in runtime_src_roots {
        let dir = crate_path(&root, relative_root);
        if !dir.exists() {
            continue;
        }
        collect_forbidden_runtime_network_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Reborn runtime HTTP must use the shared host egress service and ironclaw_network only:\n{}",
        violations.join("\n")
    );
}

#[test]
fn hosted_mcp_discovery_is_never_driven_by_ambient_startup_composition() {
    let root = workspace_root();
    let factory = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_composition/src/factory.rs",
    ))
    .expect("composition factory source must be readable");
    let owner_transaction = std::fs::read_to_string(crate_path(
        &root,
        "crates/ironclaw_extension_host/src/activation_transaction.rs",
    ))
    .expect("extension-host activation transaction source must be readable");

    for forbidden in [
        "reconcile_hosted_mcp_runtime_readiness",
        "reconcile_hosted_mcp_startup",
    ] {
        assert!(
            !factory.contains(forbidden),
            "composition startup must not invoke hosted-MCP discovery through `{forbidden}`; \
             discovery requires a real caller/run ResourceScope"
        );
        assert!(
            !owner_transaction.contains(forbidden),
            "extension-host must not expose ambient hosted-MCP startup probing through \
             `{forbidden}`"
        );
    }
}

#[test]
fn reborn_product_api_crates_do_not_bind_http_ingress() {
    let forbidden = [
        ForbiddenUse {
            pattern: "tokio::net::TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "std::net::TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "TcpListener::bind",
            reason: "Reborn product/API crates must expose route descriptors, not bind listeners",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "axum::serve",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "hyper::Server",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "Server::bind",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "axum_server::bind",
            reason: "Reborn product/API crates must not own server lifecycle",
            exempt: None,
        },
    ];

    let root = workspace_root();
    // Every entry must resolve — asserted below. The silent `if !dir.exists()
    // { continue }` this list used to rely on let `crates/ironclaw_reborn_api/src`
    // sit here long after that crate was gone, scoping the rule over a crate
    // that did not exist while looking fully populated (CHECKLIST WS0, #6963).
    // `crates/ironclaw_assistant/src` also appeared three times, a leftover of the
    // #6583 four-crate fold; duplicates only multiplied the reported violations.
    //
    // KNOWN GAP, deliberately not closed here: the trailing comment below
    // describes a WebChat v2 route-surface entry that is absent, so the rule
    // does not cover `crates/product/ironclaw_webui/src` at all. Adding it turns this
    // gate red (that tree carries `axum::serve` and `TcpListener::bind` hits),
    // which is an architecture decision — either webui owns server lifecycle it
    // should not, or the rule owes it an `exempt`. Not a gate repoint; recorded
    // for the owner rather than silently introduced by a CI-gates change.
    let reborn_product_api_src_roots = [
        "crates/ironclaw_turn_runner/src",
        "crates/ironclaw_cli/src",
        "crates/ironclaw_composition/src",
        "crates/ironclaw_config/src",
        "crates/ironclaw_event_store/src",
        "crates/ironclaw_openai_compat/src",
        "crates/ironclaw_assistant/src",
        "crates/extensions/packages/telegram/src",
        "crates/extensions/packages/slack/src",
        "crates/ironclaw_outbound/src",
        "crates/ironclaw_conversations/src",
        "crates/ironclaw_turns/src",
        "crates/ironclaw_threads/src",
        "crates/ironclaw_loop_host/src",
        // WebChat v2 route surface: a Product/API crate that exposes
        // axum handler functions and `IngressRouteDescriptor`s but must
        // never bind sockets or call `axum::serve` itself — that is
        // host composition's job. Without this entry the contract fails
        // open for the new route crate.
    ];

    let missing: Vec<&str> = reborn_product_api_src_roots
        .iter()
        .copied()
        .filter(|relative_root| !crate_path(&root, relative_root).is_dir())
        .collect();
    assert!(
        missing.is_empty(),
        "server-lifecycle rule names src trees that do not exist: {missing:?}. Repoint or \
         drop them — a root that resolves to nothing scopes the rule over nothing while \
         the list still looks populated."
    );

    let mut violations = Vec::new();
    for relative_root in reborn_product_api_src_roots {
        let dir = crate_path(&root, relative_root);
        collect_forbidden_uses(&dir, &root, &forbidden, &mut violations);
    }

    assert!(
        violations.is_empty(),
        "Reborn HTTP ingress must be host-owned; product/API crates may expose descriptors or route fragments but must not bind/serve listeners:\n{}",
        violations.join("\n")
    );
}

#[test]
fn reborn_openai_compat_routes_do_not_depend_on_v1_gateway_or_legacy_streams() {
    let forbidden = [
        ForbiddenUse {
            pattern: "src/channels/web",
            reason: "OpenAI-compatible Reborn routes must not route through v1 gateway handlers",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "crate::channels::web",
            reason: "OpenAI-compatible Reborn routes must not import v1 gateway modules",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ironclaw::channels::web",
            reason: "OpenAI-compatible Reborn routes must not import v1 gateway modules",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "GatewayState",
            reason: "OpenAI-compatible Reborn routes must not depend on v1 gateway state",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SseManager",
            reason: "OpenAI-compatible Reborn streaming must use projection-stream ports, not raw legacy SSE streams",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "AppEvent",
            reason: "OpenAI-compatible Reborn streaming must translate ProductProjectionItem state, not raw legacy AppEvent streams (the enum itself was deleted in WS1.6 — zero consumers; this entry stays as a reintroduction pin)",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "IncomingMessage",
            reason: "OpenAI-compatible Reborn routes must enter through ProductSurface, not legacy channel ingress",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "get_or_create_assistant_conversation",
            reason: "OpenAI-compatible Reborn retrieve/cancel must use opaque refs and projection readers, not legacy conversation reconstruction",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ConversationManager",
            reason: "OpenAI-compatible Reborn routes must not reconstruct legacy conversations directly",
            exempt: None,
        },
    ];

    let root = workspace_root();
    let compat_src = crate_path(&root, "crates/ironclaw_openai_compat/src");
    let mut violations = Vec::new();
    collect_forbidden_uses(&compat_src, &root, &forbidden, &mut violations);

    assert!(
        violations.is_empty(),
        "Reborn OpenAI-compatible routes must stay ProductSurface/projection-port backed and independent of v1 gateway handlers, legacy SSE/AppEvent streams, and legacy conversation reconstruction:\n{}",
        violations.join("\n")
    );
}

#[test]
fn reborn_product_auth_contract_stays_reborn_native() {
    let forbidden = [
        ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        },
        ForbiddenUse {
            pattern: "src/extensions",
            reason: "v1 extension paths are inventory only, not Reborn auth implementation",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "src/channels/web",
            reason: "v1 web routes are inventory only, not Reborn auth implementation",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ExtensionManager",
            reason: "Reborn product auth must not call through the v1 extension manager",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "PendingOAuth",
            reason: "Reborn product auth must not reuse v1 pending OAuth maps",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "PendingGate",
            reason: "Reborn product auth must not reuse v1 pending gate maps",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SecretsStore",
            reason: "Reborn product auth must use opaque handles, not raw v1 secrets storage",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "get_decrypted",
            reason: "Reborn product auth must not retrieve raw secret material directly",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "auth-token",
            reason: "Reborn manual-token setup must not fall back to v1 chat token route names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "auth_token",
            reason: "Reborn manual-token setup must not fall back to v1 chat token command paths",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "IncomingMessage",
            reason: "Reborn product auth must not capture manual tokens through chat transcripts",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "ChatMessage",
            reason: "Reborn product auth must not capture manual tokens through chat transcripts",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "secret_name",
            reason: "Reborn product auth must use scoped credential accounts and opaque handles, not raw v1 secret names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "SecretName",
            reason: "Reborn product auth must use scoped credential accounts and opaque handles, not raw v1 secret names",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "reqwest",
            reason: "Reborn product auth must not own outbound HTTP transport",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "authorization_code: String",
            reason: "raw OAuth codes must be one-shot non-serializable provider inputs",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "pkce_verifier: String",
            reason: "raw PKCE verifiers must be one-shot non-serializable provider inputs",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "access_token: String",
            reason: "raw provider tokens must not enter product auth contract records",
            exempt: None,
        },
        ForbiddenUse {
            pattern: "refresh_token: String",
            reason: "raw provider tokens must not enter product auth contract records",
            exempt: None,
        },
    ];

    let root = workspace_root();
    let manifest = std::fs::read_to_string(crate_path(&root, "crates/ironclaw_auth/Cargo.toml"))
        .expect("ironclaw_auth manifest must be readable");
    assert!(
        !manifest.contains("reqwest"),
        "ironclaw_auth must not depend on reqwest directly; provider transport belongs behind Reborn-native composition"
    );

    let auth_src = crate_path(&root, "crates/ironclaw_auth/src");
    assert!(
        auth_src.exists(),
        "Reborn product auth contract crate must have a src directory at {}",
        auth_src.display()
    );

    let mut violations = Vec::new();
    collect_forbidden_uses(&auth_src, &root, &forbidden, &mut violations);
    collect_forbidden_reborn_auth_path_uses(
        &crate_path(&root, "crates/ironclaw_webui/src/product_auth"),
        &crate_path(&root, "crates/ironclaw_webui/src/product_auth.rs"),
        &root,
        &forbidden,
        &mut violations,
    );

    assert!(
        violations.is_empty(),
        "Reborn product auth can be behavior-compatible with v1, but implementation and composition code paths must not mingle with v1 routes, v1 extension/secrets managers, raw provider transport, or raw secret records:\n{}",
        violations.join("\n")
    );
}

struct ForbiddenRuntimeNetworkUse {
    pattern: &'static str,
    reason: &'static str,
}

struct ForbiddenUse {
    pattern: &'static str,
    reason: &'static str,
    exempt: Option<fn(&str) -> bool>,
}

fn collect_forbidden_turns_identifier_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_turns_identifier_uses(&path, root, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if path
            .components()
            .any(|component| component.as_os_str() == "process_projection")
            || path.file_name().and_then(|name| name.to_str()) == Some("process_projection.rs")
        {
            continue;
        }
        let mut contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        if path.file_name().and_then(|name| name.to_str()) == Some("lib.rs") {
            contents = strip_process_projection_reexport_block(&contents);
        }
        for pattern in ["InvocationId", "ProcessId"] {
            if contents.contains(pattern) {
                violations.push(format!(
                    "{} contains forbidden lower identifier `{pattern}`",
                    path.strip_prefix(root).unwrap_or(&path).display()
                ));
            }
        }
    }
}

fn strip_process_projection_reexport_block(contents: &str) -> String {
    let mut stripped = String::new();
    let mut skipping = false;
    for line in contents.lines() {
        if line
            .trim_start()
            .starts_with("pub use process_projection::{")
        {
            skipping = !line.trim_end().ends_with("};");
            continue;
        }
        if skipping {
            if line.trim_start() == "};" {
                skipping = false;
            }
            continue;
        }
        stripped.push_str(line);
        stripped.push('\n');
    }
    stripped
}

#[test]
fn process_projection_reexport_stripping_preserves_following_source() {
    for source in [
        "pub use process_projection::{InvocationId};\npub struct InvocationIdLeak;\n",
        "pub use process_projection::{\n    InvocationId,\n};\npub struct InvocationIdLeak;\n",
    ] {
        let stripped = strip_process_projection_reexport_block(source);
        assert!(!stripped.contains("pub use process_projection"));
        assert!(
            stripped.contains("pub struct InvocationIdLeak"),
            "source following a re-export must remain scannable: {stripped:?}"
        );
    }
}

fn collect_forbidden_string_uses(
    dir: &std::path::Path,
    needle: &str,
    root: &std::path::Path,
    matches: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_string_uses(&path, needle, root, matches);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        if contents.contains(needle) {
            matches.push(
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
}

struct ProviderToolNameUse {
    path: String,
    line_number: usize,
    pattern: &'static str,
}

fn collect_provider_tool_name_boundary_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    uses: &mut Vec<ProviderToolNameUse>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|err| panic!("failed to read dir entry: {err}"));
        let path = entry.path();
        if path.is_dir() {
            collect_provider_tool_name_boundary_uses(&path, root, uses);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if is_rust_test_source_path(&relative) {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        collect_provider_tool_name_uses_in_source(&relative, &contents, uses);
    }
}

fn is_rust_test_source_path(relative: &str) -> bool {
    relative.contains("/tests/")
        || relative.ends_with("/tests.rs")
        || relative.ends_with("_tests.rs")
}

fn collect_provider_tool_name_uses_in_source(
    relative: &str,
    contents: &str,
    uses: &mut Vec<ProviderToolNameUse>,
) {
    let mut pending_cfg_test = false;
    let mut skipping_test_module_depth: Option<usize> = None;
    let mut in_block_comment = false;
    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let code = strip_line_strings_and_comments(line, &mut in_block_comment);
        if let Some(depth) = skipping_test_module_depth {
            let next_depth = update_brace_depth(depth, &code);
            if next_depth == 0 {
                skipping_test_module_depth = None;
            } else {
                skipping_test_module_depth = Some(next_depth);
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
            continue;
        }
        let code_trimmed = code.trim();
        if pending_cfg_test {
            if code_trimmed.starts_with("#[") || code_trimmed.is_empty() {
                continue;
            }
            if code_trimmed.contains('{') {
                let depth = update_brace_depth(0, &code);
                if depth > 0 {
                    skipping_test_module_depth = Some(depth);
                }
                pending_cfg_test = false;
                continue;
            }
            pending_cfg_test = false;
        }

        for pattern in ["ProviderToolName", "provider_tool_name"] {
            if code.contains(pattern) {
                uses.push(ProviderToolNameUse {
                    path: relative.to_string(),
                    line_number,
                    pattern,
                });
            }
        }
    }
}

fn strip_line_strings_and_comments(line: &str, in_block_comment: &mut bool) -> String {
    let mut output = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(character) = chars.next() {
        if *in_block_comment {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                *in_block_comment = false;
                output.push(' ');
                output.push(' ');
            } else {
                output.push(' ');
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            output.push(' ');
            continue;
        }
        if character == '"' {
            in_string = true;
            output.push(' ');
            continue;
        }
        if character == '/' && chars.peek() == Some(&'/') {
            break;
        }
        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            *in_block_comment = true;
            output.push(' ');
            output.push(' ');
            continue;
        }
        output.push(character);
    }
    output
}

fn update_brace_depth(depth: usize, line: &str) -> usize {
    let opens = line.chars().filter(|character| *character == '{').count();
    let closes = line.chars().filter(|character| *character == '}').count();
    depth.saturating_add(opens).saturating_sub(closes)
}

struct BoundaryRule {
    crate_name: &'static str,
    forbidden: Vec<&'static str>,
}

fn boundary_rules() -> Vec<BoundaryRule> {
    vec![
        BoundaryRule {
            // `ironclaw_extension_registry` was on this crate's *guidance* forbidden
            // list (`CLAUDE.md`) but never on the enforced one, and the crate
            // held the dependency the whole time — the guidance-vs-code
            // contradiction PROPOSAL §6.9.1 names. Resolved by CHECKLIST WS5's
            // `product` narrows row: `adapter_registry` (product's only
            // consumer of the registry) moved to its chartered owners, the
            // manifest entry is gone, and the rule is enforced from here on.
            crate_name: "ironclaw_assistant",
            forbidden: vec![
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_wasm",
                "ironclaw_sandbox",
                "ironclaw_network",
                "ironclaw_engine",
                "ironclaw_gateway",
            ],
        },
        BoundaryRule {
            // Product auth is a Reborn contract/facade vocabulary plus the
            // recipe-driven auth engine (extension-runtime workstream D). The
            // engine owns token secret storage, so the Reborn-native
            // `ironclaw_secrets` store is allowed; implementation code must
            // still not reach into v1 routes, extension managers, runtimes,
            // or channel-specific stacks.
            crate_name: "ironclaw_auth",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_extension_registry",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        // NOTE(webui-merge): the former `ironclaw_webui_v2` BoundaryRule was
        // removed when that crate's route surface was folded into
        // `ironclaw_webui` (as its `webui_v2` module). The
        // "handlers do not touch adapters/dispatcher/runtime directly"
        // invariant is now carried by the `ironclaw_webui`
        // rule's forbidden list.
        BoundaryRule {
            // OpenAI-compatible route surface is a Reborn product/API facade.
            // It may depend on host ingress vocabulary, product adapter
            // contracts, and the ProductSurface facade, but it must not revive
            // v1 gateway/LLM proxy paths or reach into runtime/composition
            // services directly.
            crate_name: "ironclaw_openai_compat",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                // `ironclaw_filesystem` is permitted: the durable
                // OpenAiCompatRefStore persists opaque refs through the universal
                // RootFilesystem port. It is unconditional — this crate declares no
                // cargo features, and WS5 collapsed the LibSql/Postgres ref-store
                // newtypes onto that one backend-neutral form.
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_extension_support",
                "ironclaw_first_party_extension_ports",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_storage",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // First-party extensions are userland implementation packages.
            // They may consume scoped storage and pure safety helpers, but
            // must not receive ambient runtime authority or loop-facing
            // runtime handles.
            crate_name: "ironclaw_extension_support",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_first_party_extension_ports",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        // ✎ WS8, 2026-08-05 — the `ironclaw_first_party_extension_ports` rule
        // that sat here is gone with the crate. Its replacement is
        // `dissolved_ports_module_keeps_its_crate_boundary` below, which pins
        // the same reach at module granularity. Deleting it outright would
        // have widened the moved code's legal imports by nine crates in
        // silence; leaving it would have left an inert rule, since a rule
        // whose crate has no directory is skipped by
        // `reborn_boundary_rules_active_crates_are_workspace_members`.
        // `ironclaw_first_party_extension_ports` stays in every *forbidden*
        // list that named it — those are reintroduction pins now, and the
        // crate must not come back.
        BoundaryRule {
            crate_name: "ironclaw_config",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_api",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_turn_runner",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // The standalone CLI reaches runtime and provider/admin UX through
            // `ironclaw_composition` facades. Adding any of the
            // forbidden deps here re-opens "speculative public API" access to
            // internal Reborn types (turn coordinator, session thread service,
            // loop drivers, LLM registry/auth internals, etc.) and
            // re-introduces the narrow-surface regression this rule exists to
            // prevent.
            crate_name: "ironclaw",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_engine",
                "ironclaw_gateway",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_turn_runner",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_tui",
                "ironclaw_turns",
            ],
        },
        BoundaryRule {
            // Host-owned WebUI ingress: binds the TCP listener and runs
            // the axum serve loop for the composed v2 Router. Since the
            // `ironclaw_webui_v2` route surface was folded into this crate
            // (as its `webui_v2` module), it now legitimately consumes the
            // `ironclaw_assistant` `ProductSurface` facade the v2
            // handlers dispatch through. It still must not pull lower
            // substrate handles, product adapters, or v1 surface code into
            // the binary path. Reaches the rest of Reborn through
            // the ProductSurface/runtime handles it receives from
            // ironclaw_composition, the neutral ironclaw_host_ingress
            // Axum mount carriers, and auth-owned product-auth service
            // contracts. The direct `ironclaw_common` edge is limited to shared
            // response/error DTO primitives used by product-auth HTTP routes;
            // secret storage and durable auth ownership stay behind
            // `ironclaw_auth`, not `ironclaw_secrets`.
            //
            // ✎ **Re-derived 2026-08-04 (WS5 `webui` row) against PROPOSAL
            // §6.9.4.** The row asked for the derivation nobody had done. Result:
            // **zero removals, nine additions**, all no-op ratchets (webui's ten
            // normal workspace deps are `host_api`, `product_contracts`,
            // `extension_contracts`, `extension_host`, `host_ingress`, `auth`,
            // `attachments`, `common`, `product`, `reborn_openai_compat` — none of
            // the nine appears in any dependency kind).
            //
            // What the derivation is *from*, since §6.9.4 carries no forbidden
            // list of its own: §8.2's `product/` row ("**app ✗**", "product still
            // ✗ host_runtime/dispatch/lanes"), §8.2's retained named rules
            // ("concrete extension crates link only from the binary"), and
            // `families/product.md`'s "never touches a lane crate / the extension
            // registry or hosting crates". Additions, in that order:
            // `ironclaw_composition` (§8.2 app ✗ — and the direction is
            // backwards: this crate *receives* handles from composition; it is on
            // 15 other rules and on every other products-layer rule but
            // `ironclaw_assistant`'s), `ironclaw_wasm_limiter` (§8.2 ✗ lanes — the
            // **only one of the nine no other gate covers**; zero rules named it
            // workspace-wide, and the existing wasm_limiter gate checks its
            // outbound deps, not its inbound edges), `ironclaw_slack_extension` +
            // `ironclaw_telegram_extension` (concrete extension crates),
            // `ironclaw_event_projections` + `ironclaw_event_streams` +
            // `ironclaw_extension_support` + `ironclaw_first_party_extension_ports`
            // + `ironclaw_storage` (parity with `ironclaw_openai_compat`,
            // the sibling transport, whose list these five closed the gap to).
            //
            // Explicitly NOT added, because the charter sanctions them:
            // `ironclaw_assistant` (§12.11 D-B, permanent edge — not a pending
            // flip), `ironclaw_extension_host` (§6.9.4's 2026-08-02 pairing
            // amendment; the pairing *service* core stays in the host by §6.8.2),
            // `ironclaw_openai_compat`, `ironclaw_attachments`,
            // `ironclaw_host_api`, `ironclaw_host_ingress`,
            // `ironclaw_product_contracts`, `ironclaw_extension_contracts`,
            // `ironclaw_auth`, `ironclaw_common`. Memory providers are covered by
            // `only_the_sanctioned_residue_names_a_memory_provider`, deliberately.
            //
            // **Keep this rule normal-deps-only.** Four entries on the list below
            // (`ironclaw_secrets`, `ironclaw_loop_host`, `ironclaw_threads`,
            // `ironclaw_turns`) are live *dev*-dependencies of `ironclaw_webui`;
            // tightening this rule to all dependency kinds would go red on four
            // counts on the day it landed.
            crate_name: "ironclaw_webui",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_event_log",
                "ironclaw_extension_support",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_first_party_extension_ports",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_slack_extension",
                "ironclaw_storage",
                "ironclaw_telegram_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_wasm_limiter",
            ],
        },
        BoundaryRule {
            // The extension-tier contract stays a leaf of the contracts family.
            // The allowlist in `reborn_crate_dependency_boundaries_hold` is the
            // authority; this rule names the edges whose appearance would be
            // most damaging, so the failure message says which invariant broke:
            // `ironclaw_extension_registry` (the registry crate whose installation
            // stores and manifest parsing §6.1.2 forbids here),
            // `ironclaw_extension_host` (lifecycle execution and ingress
            // routing), and `ironclaw_assistant` (the product workflow this crate
            // exists to keep the channel packages away from).
            crate_name: "ironclaw_extension_contracts",
            forbidden: vec![
                "ironclaw_auth",
                "ironclaw_capabilities",
                "ironclaw_extension_host",
                "ironclaw_extension_registry",
                "ironclaw_extension_support",
                "ironclaw_host_runtime",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_assistant",
                "ironclaw_composition",
                "ironclaw_sandbox",
                "ironclaw_slack_extension",
                "ironclaw_telegram_extension",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_webui",
            ],
        },
        BoundaryRule {
            // The product-tier contract stays a leaf of the contracts family.
            // The allowlist in `reborn_crate_dependency_boundaries_hold` is the
            // authority; this rule names the edges whose appearance would be
            // most damaging, so the failure message says which invariant broke:
            // `ironclaw_assistant` (the `ProductSurface` *implementation* and all
            // admission/delivery workflow — the single thing this crate exists
            // so its collaborators never have to compile), `ironclaw_operator`
            // and `ironclaw_extension_host` (the two crates whose ports it
            // declares, so the inversion cannot silently re-invert), and
            // `ironclaw_webui`/`ironclaw_host_ingress` (a transport edge would
            // mean HTTP had reached the contracts tier).
            crate_name: "ironclaw_product_contracts",
            forbidden: vec![
                "ironclaw_auth",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_extension_host",
                "ironclaw_extension_registry",
                "ironclaw_extension_support",
                "ironclaw_host_ingress",
                "ironclaw_host_runtime",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_operator",
                "ironclaw_outbound",
                "ironclaw_assistant",
                "ironclaw_composition",
                "ironclaw_openai_compat",
                "ironclaw_turn_runner",
                "ironclaw_sandbox",
                "ironclaw_slack_extension",
                "ironclaw_telegram_extension",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_webui",
            ],
        },
        BoundaryRule {
            // The deployment-operator control plane (PROPOSAL §6.9.2). It had
            // **no rule at all** until the WS5 operator row — the audit's
            // clearest correlation was guidance-and-gate presence ↔ discipline,
            // and this crate had neither, which is how its `ironclaw_assistant`
            // edge survived every earlier sweep.
            //
            // `ironclaw_assistant` is the headline: operator is product's
            // *sibling*, not its consumer. It implements product-side ports,
            // and those ports are declared in `ironclaw_product_contracts` —
            // so naming product here is the inversion re-inverting.
            // `reborn_operator_port_inversion.rs` proves the same fact through
            // `cargo metadata` and additionally pins where each port landed;
            // this rule states the invariant where a reader looking for the
            // crate's boundary will find it.
            //
            // The rest are shape rules: no assembly root
            // (`ironclaw_composition`), no transports
            // (`ironclaw_webui`, `ironclaw_openai_compat`), no extension
            // machinery (`ironclaw_extension_host`, `ironclaw_extension_registry`,
            // `ironclaw_extension_support`), no lanes
            // (`ironclaw_host_runtime`, `ironclaw_mcp`, `ironclaw_wasm`,
            // `ironclaw_sandbox`) and no turn kernel (`ironclaw_turns`,
            // `ironclaw_turn_runner`, `ironclaw_loop_host`). Operator administers
            // LLM providers, rings logs, and controls an OS service; none of
            // that needs to see a turn.
            //
            // ✎ **`ironclaw_secrets` joined the list with WS3's row** ("tighten
            // direct `secrets` consumers: remove the `webui` and `operator`
            // edges via `product_contracts` ports"). PROPOSAL §12.1b required
            // the port replacement to land first, and it did, in the same
            // change: `ironclaw_product_contracts::operator_secrets::
            // OperatorSecretValueStore` is what `LlmKeyStore` now holds, and
            // `ironclaw_composition::RuntimeOperatorSecretValueStore`
            // implements it over the substrate. This entry is the "add the
            // boundary rule" half of the row and is what stops the edge coming
            // back through some later convenience.
            //
            // The rule is normal-deps only, so a *dev*-dependency would still be
            // legal here — and there is deliberately none: the substrate-fidelity
            // tests moved to the port's implementor rather than keeping a test
            // seam open into the substrate from the products tier.
            crate_name: "ironclaw_operator",
            forbidden: vec![
                "ironclaw_extension_host",
                "ironclaw_extension_registry",
                "ironclaw_extension_support",
                "ironclaw_host_runtime",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_assistant",
                "ironclaw_composition",
                "ironclaw_openai_compat",
                "ironclaw_turn_runner",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_slack_extension",
                "ironclaw_telegram_extension",
                "ironclaw_turns",
                "ironclaw_wasm",
                "ironclaw_webui",
            ],
        },
        BoundaryRule {
            // Concrete Telegram channel extension. It had no rule at all until
            // WS1.3, which is how its `ironclaw_assistant` edge survived: the
            // crate reached product for `PreferenceTargetCodec` /
            // `PreferenceTargetEncodeRequest` / `ExternalConversationRef` while
            // its Slack sibling — same shape, same surfaces — already forbade
            // product here. The codec port moved to
            // `ironclaw_extension_contracts` (its implementors are exactly the
            // channel packages), the edge is gone from both `[dependencies]`
            // and `[dev-dependencies]`, and this rule keeps it gone.
            // The protocol half is now in-crate (WS2 merged
            // `ironclaw_telegram_v2_adapter` into this package); `ironclaw_host_api` /
            // `ironclaw_extension_contracts` are the sanctioned contract deps.
            crate_name: "ironclaw_telegram_extension",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_auth",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_slack_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // The loop-tier contract stays a leaf of the contracts family. The
            // allowlist in `reborn_crate_dependency_boundaries_hold` is the
            // authority; this rule names the edges whose appearance would be
            // most damaging — above all `ironclaw_turns`, whose dependency runs
            // the other way — so the failure message says which invariant broke.
            crate_name: "ironclaw_loop_contracts",
            forbidden: vec![
                "ironclaw_agent_loop",
                "ironclaw_capabilities",
                "ironclaw_extension_host",
                "ironclaw_hooks",
                "ironclaw_host_runtime",
                "ironclaw_loop_host",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_composition",
                "ironclaw_turn_runner",
                "ironclaw_threads",
                "ironclaw_turns",
            ],
        },
        BoundaryRule {
            // The product face of extensions (PROPOSAL §6.8.3). It calls the
            // host's authority; it never becomes a second assembly layer and
            // never serves a route of its own. The edges named here are the
            // ones whose appearance would mean exactly that: composition and
            // the CLI (assembly — both depend on the manager, never the
            // reverse), `ironclaw_webui`/`ironclaw_host_ingress` (a transport
            // edge would put HTTP inside a product sub-owner), and
            // `ironclaw_openai_compat`/`ironclaw_operator` (sibling
            // product surfaces). `ironclaw_assistant` is deliberately NOT here:
            // the manager still names seven product DTO/capability-id symbols,
            // frozen shrink-only by `reborn_extension_manager_split.rs`, which
            // is where that edge is tracked to zero.
            crate_name: "ironclaw_extension_manager",
            forbidden: vec![
                "ironclaw_host_ingress",
                "ironclaw_operator",
                // The CLI, by its PACKAGE name. `crates/app/ironclaw_cli/`
                // is only the directory; `forbidden` is matched against
                // `cargo metadata` package names, so the directory spelling
                // is an entry that can never fire. Pinned by
                // `boundary_rule_names_are_package_names_not_crate_directories`.
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_openai_compat",
                "ironclaw_webui",
            ],
        },
        BoundaryRule {
            // Shared libSQL runtime owns connection mechanics only. It must
            // remain below every adapter that consumes its read/write lanes.
            crate_name: "ironclaw_libsql_runtime",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_composition",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_triggers",
                "ironclaw_turns",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_filesystem",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_resources",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                // ironclaw_filesystem is permitted: ResourceGovernorStore
                // routes the resource-governor snapshot through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_trust",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_extension_registry",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_support",
                "ironclaw_first_party_extension_ports",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_event_log",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // Product-facing projection reducers consume typed domain events.
            // `ironclaw_turns` is intentionally allowed here for
            // `TurnLifecycleEvent`-derived read models such as pending gates;
            // projection crates must still stay below product/runtime
            // composition and must not import root `src/` or legacy engine
            // pending-gate types.
            crate_name: "ironclaw_event_projections",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_event_store",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_event_streams",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_assistant",
                "ironclaw_event_store",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_telegram_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // Concrete Slack protocol adapter owns only Slack payload
            // normalization/rendering over the ProductAdapter DTO surface.
            // Host auth verification, credential resolution, delivery fanout,
            // workflow admission, and runtime/network authority stay outside
            // the adapter crate. `ironclaw_host_api` is deliberately allowed:
            // concrete extension crates depend on host_api /
            // product_adapters CONTRACT types only (`RestrictedEgress`,
            // `SecretHandle` — extension-runtime implementation.md §3), which
            // the P4 `ChannelAdapter` signatures require.
            crate_name: "ironclaw_slack_extension",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_auth",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_telegram_extension",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_outbound",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_extension_registry",
                // ironclaw_filesystem is permitted: OutboundStateStore
                // routes outbound persistence through ScopedFilesystem under
                // the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // Trigger core owns source evaluation and trigger-domain state.
            // Durable storage, poller lifecycle, capability registration,
            // product adapters, and outbound delivery are wired by later
            // owners, not by reaching upward from this crate.
            //
            // `ironclaw_safety` is deliberately NOT forbidden (2026-08-04,
            // PROPOSAL §6.4.2/§6.4.3). This crate's boundary role is
            // host-trusted ingress minting, and a mint that does not validate
            // what it seals is not a mint. `ironclaw_safety` is a same-layer
            // `substrates` leaf with no I/O and no ironclaw dependencies, so
            // the edge is a peer edge, not a reach upward. The matching
            // narrowing is on `ironclaw_conversations` below, which must not
            // re-acquire the scan.
            crate_name: "ironclaw_triggers",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_engine",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            // External<->canonical binding, actor pairing, and accepted-message
            // / turn-submission idempotency (PROPOSAL §6.4.2). Its one legal
            // upward edge is `ironclaw_turns` (turn admission authority — the
            // tracked LAYER_MATRIX_EXCEPTION below); everything else above or
            // beside it belongs to another owner.
            //
            // `ironclaw_safety` is listed because the crate *used* to hold it
            // (2026-08-04): the trusted-trigger prompt scan lived inside this
            // crate's `TrustedTriggerFireSubmitter` impl, where swapping or
            // adding a submitter silently dropped the guard. §6.4.2 moved the
            // scan behind the seam into `ironclaw_triggers`' mint of the
            // sealed request. Re-adding the dependency here would re-open that
            // fail-open, so it is pinned rather than merely absent.
            //
            // `ironclaw_threads` is listed for the sibling reason: §6.4.2 says
            // "Never: payload parsing, transcript content" — recording prompt
            // and transcript content is the thread service's job, reached by
            // composition, never from inside binding/idempotency.
            crate_name: "ironclaw_conversations",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_engine",
                "ironclaw_gateway",
                "ironclaw_storage",
                "ironclaw_tui",
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_resources",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_scripts",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_event_store",
            // ironclaw_filesystem is permitted: FilesystemEventLog routes the
            // durable log through the universal RootFilesystem dispatch
            // fabric. See `2026-05-14-universal-fs-dispatch.md`.
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_secrets",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                // ironclaw_filesystem is permitted: SecretStore /
                // CredentialBroker route secret + credential
                // persistence through ScopedFilesystem under the
                // universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_network",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_authorization",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_threads",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_engine",
                "ironclaw_event_log",
                "ironclaw_extension_registry",
                // ironclaw_filesystem is permitted: FilesystemSessionThreadService
                // routes thread/transcript persistence through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_approvals",
                // ironclaw_safety is permitted: thread/transcript storage
                // validates provider-originated replay metadata before it can
                // be persisted or exposed back to a model-visible context.
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_approvals",
            forbidden: vec![
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_processes",
                "ironclaw_resources",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_processes",
            forbidden: vec![
                "ironclaw_authorization",
                "ironclaw_approvals",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_turns",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                // ironclaw_filesystem is permitted for agent-turn projections.
                // routes turn-coordination persistence through ScopedFilesystem
                // under the universal-fs-dispatch rework (plan
                // 2026-05-14-universal-fs-dispatch).
                "ironclaw_hooks",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                // ironclaw_processes is permitted during the process-journal
                // kernel migration: turns adapts its existing turn-run store
                // to the canonical process lifecycle vocabulary owned there.
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        // The hooks framework depends on `ironclaw_turns` and host primitives
        // but must not pull in runtime adapters or dispatcher concretions.
        // This keeps the contract surface narrow and prevents the framework
        // from acquiring authority it should not have.
        BoundaryRule {
            crate_name: "ironclaw_hooks",
            forbidden: vec![
                "ironclaw_approvals",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_host_runtime",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_processes",
                "ironclaw_turn_runner",
                "ironclaw_approvals",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_wasm",
            ],
        },
        // The agent-loop framework crate owns reusable loop mechanics
        // (executor, strategies, families, state) and depends upward on neutral
        // contracts in `ironclaw_turns`. It must not import host runtime crates,
        // product adapters, dispatcher, capability host, filesystem, network,
        // secrets, DB backends, or the loop-host adapter layer — those all
        // sit above agent_loop in the stack and would create an inversion.
        BoundaryRule {
            crate_name: "ironclaw_agent_loop",
            forbidden: vec![
                "ironclaw_legacy",
                "ironclaw_approvals",
                "ironclaw_auth",
                "ironclaw_authorization",
                "ironclaw_capabilities",
                "ironclaw_conversations",
                "ironclaw_engine",
                "ironclaw_event_projections",
                "ironclaw_event_streams",
                "ironclaw_extension_registry",
                "ironclaw_filesystem",
                "ironclaw_gateway",
                "ironclaw_host_runtime",
                "ironclaw_llm",
                "ironclaw_loop_host",
                "ironclaw_mcp",
                "ironclaw_memory",
                "ironclaw_network",
                "ironclaw_outbound",
                "ironclaw_processes",
                "ironclaw_assistant",
                "ironclaw_assistant",
                "ironclaw_turn_runner",
                "ironclaw",
                "ironclaw_composition",
                "ironclaw_config",
                "ironclaw_event_store",
                "ironclaw_trace_commons",
                "ironclaw_webui",
                "ironclaw_resources",
                "ironclaw_approvals",
                "ironclaw_runtime_policy",
                "ironclaw_safety",
                "ironclaw_sandbox",
                "ironclaw_secrets",
                "ironclaw_skills",
                "ironclaw_threads",
                "ironclaw_trust",
                "ironclaw_tui",
                "ironclaw_wasm",
            ],
        },
        BoundaryRule {
            crate_name: "ironclaw_capabilities",
            forbidden: vec![
                "ironclaw_host_runtime",
                "ironclaw_secrets",
                "ironclaw_network",
                "ironclaw_mcp",
                "ironclaw_sandbox",
                "ironclaw_wasm",
            ],
        },
    ]
}

/// The closed layer set — PROPOSAL §8.1 rule 1's seven-layer ladder, and
/// nothing else.
///
/// ✎ **The vestigial `legacy` variant was deleted here 2026-08-05** (PROPOSAL
/// §11.2.11 and §11.3, CHECKLIST WS10). It described the v1 package tree, which
/// no longer exists: measured through `cargo metadata` across all 66 workspace
/// packages, **zero** declared `layer = "legacy"`, so both rules keyed on it
/// (`layer_allows_dependency`'s permissive `"legacy" => true` arm and the
/// second dependency pass that rejected a normal dep on a legacy-layer crate)
/// were unreachable — a `match` arm and a loop that could not fire.
///
/// ⚠ **Not to be confused with the ~60 `ironclaw_legacy` entries in
/// `boundary_rules()`.** Those name a retired v1 *crate* and are live
/// reintroduction pins, deliberately kept and deliberately matching nothing
/// (`boundary_rule_names_are_package_names_not_crate_directories` documents
/// exactly that distinction). Deleting a dead *layer* variant does not touch
/// them.
const IRONCLAW_CRATE_LAYERS: [&str; 7] = [
    "contracts",
    "substrates",
    "runtimes",
    "kernel",
    "loops",
    "products",
    "app",
];

struct LayerMatrixException {
    crate_name: &'static str,
    dependency_name: &'static str,
    introduced: &'static str,
    removes_in: &'static str,
    reason: &'static str,
}

/// WS0 baseline for the §11.2.2 exception ratchet (target-architecture epic
/// #3773, workstream #6920): the number of standing layer-matrix exceptions
/// measured **on this checkout**, not copied from the design docs.
///
/// Measured 2026-07-30 against `origin/main` @ `ae0989c37` by counting the
/// entries of `LAYER_MATRIX_EXCEPTIONS` below — the recount agreed with the 20
/// the target-architecture PROPOSAL/CHECKLIST document:
///
/// ```text
/// rg -c "^    LayerMatrixException \{" \
///   crates/app/ironclaw_architecture_tests/tests/reborn_dependency_boundaries.rs
/// ```
///
/// The target is the empty list (PROPOSAL §11.2.2, CHECKLIST WS12). This
/// number is therefore a ceiling that only ever moves **down**: when a wave
/// deletes exceptions, lower it in the same PR so the new floor is locked in.
/// The ratchet below refuses growth; it cannot make the list shrink on its own.
///
/// **20 → 15 (WS1.1, turn-vocabulary completion).** `auth`, `event_streams`,
/// `outbound`, `triggers`, and `event_projections` reached `ironclaw_turns`
/// for turn vocabulary only. That vocabulary is now complete in
/// `ironclaw_host_api::turn`, those five crates import it from there, and
/// their `ironclaw_turns` dependency is gone — so the five exceptions were not
/// waived, their edges no longer exist.
///
/// **15 → 13 (WS1.2, `ironclaw_loop_contracts` extraction).** `hooks` and
/// `agent_loop` reached `ironclaw_turns` for the loop-tier port set and the
/// `LoopExit` claim vocabulary. Both now live in `ironclaw_loop_contracts`,
/// both crates dropped their `ironclaw_turns` manifest dependency, and
/// `agent_loop`'s contracts-only rule passes with zero exceptions. The third
/// `→ turns` entry, `conversations`, did **not** fall: re-verified against the
/// live tree it is turn *admission* (a `TurnCoordinator` handle and
/// `submit_turn` call), not vocabulary, so `loop_contracts` cannot dissolve it
/// — its entry now records that and points at WS5.
///
/// **10 → 6 (WS2, `ironclaw_extension_registry` re-layered `loops` → `substrates`).**
/// Four entries — `host_runtime`, `capabilities`, `mcp` and `scripts` reaching
/// `ironclaw_extension_registry` — were all the same edge under four names: a
/// kernel/runtimes-tier crate consuming the registry's manifest DTOs. None was
/// waived and none of those edges was deleted; the *registry* moved down to
/// `substrates`, where every layer above may legally reach it (PROPOSAL §6.8.1:
/// "Layer substrates legalizes `capabilities → extensions` and
/// `host_runtime → extensions`" — it legalizes `mcp` and `scripts` too, which
/// that entry undercounts). The one edge that blocked the move,
/// `extensions → trust` (kernel), is also gone rather than waived:
/// `TrustPolicyInput` is requested-trust *vocabulary* and now lives beside
/// `PackageIdentity` in `ironclaw_host_api::trust`, which is §6.8.1's own
/// prescription ("`trust`-vocabulary via `host_api`").
///
/// **Label semantics (recorded 2026-08-04).** `removes_in: "W7"` on the
/// original entries is the retired **July-train** wave label (the tags were
/// written when this gate was armed — every one carries `introduced:
/// 2026-07-09`, predating the target-architecture program), **not** the
/// program's Wave 5 or its WS7 physical-move workstream: PROPOSAL §8.3
/// resolves every W7-tagged edge through WS2/WS3/WS4 work. Surviving entries
/// carry a workstream-or-issue key; do not read any remaining `W7` as a
/// schedule.
///
/// **6 → 5 (WS3, consolidated).** Two WS3 slices touch this list and their
/// edits are disjoint, so the consolidated PR recomputes the constant rather
/// than inheriting either slice's figure.
///
/// *Sandbox lane merge + `ironclaw_mcp` contracts flip.* Authored against a
/// 10-entry list, where it deleted `ironclaw_mcp → ironclaw_extension_registry` and
/// `ironclaw_scripts → ironclaw_extension_registry` and computed `10 → 8`. **WS2's
/// re-layer above deleted both of those edges first**, by legalizing the whole
/// `→ ironclaw_extension_registry` class rather than crate by crate — so against live
/// `main` this slice's only remaining effect on the list is a rename:
/// `ironclaw_scripts → ironclaw_resources` becomes
/// `ironclaw_sandbox → ironclaw_resources`, the same edge carried with the
/// crate the merge absorbed `ironclaw_scripts` into. One entry out, one in,
/// net zero. The slice's *evidence* is unaffected — those two edges are gone
/// from the manifests either way — but its arithmetic is superseded.
///
/// *First-party skill tools move to `extension_support`.* `host_runtime`
/// reached `ironclaw_skills` for exactly two things, both inside
/// `first_party_tools/`: `InstalledSkillMetadataSource` when rewriting a URL
/// install's input, and the `MAX_INSTALL_BUNDLE_*` limits enforced while
/// unpacking a fetched bundle. Both moved to
/// `ironclaw_extension_support::skills` (which already owned the executor half
/// and already depends on `ironclaw_skills`), so the manifest edge is gone
/// rather than waived. `ironclaw_skills` survives as a **dev**-dependency only
/// — the host's own tests still assert against the shared bundle limits, and
/// dev edges are outside the matrix by construction (`is_normal_dependency`).
/// This is the one entry the consolidated slices actually remove: `6 → 5`.
///
/// The obligations/builder split and the operator-secrets tightening move
/// nothing here: an intra-crate module split changes no crate's layer, and
/// `products → substrates` (operator → secrets) is matrix-legal, so that edge
/// was never in this register — it is an §8.2 forbidden-edge rule instead.
///
/// Both surviving `→ ironclaw_resources` rows (`mcp`, `sandbox`) point at
/// #7067, which owns the narrow reserve/reconcile/release port that dissolves
/// them.
///
/// Recomputed as `len()` of the merged list on the pushed ref, per the union
/// rule the CHECKLIST §11.2.2 row records. Each slice's own figure (8 and 9,
/// both computed against a 10-entry list) is superseded by WS2 landing first.
///
/// **5 → 4 (WS3, `ironclaw_processes` re-layered `runtimes` → kernel).** Not a
/// waiver removal and not a code move: the edge became *legal*. `processes`
/// held `ironclaw_resources` (kernel) from `runtimes`, an upward edge, which
/// is the only reason it needed an exception. CHECKLIST WS3 already called for
/// the re-layer and `families/kernel.md` already listed `ironclaw_processes`
/// among the kernel crates — only `Cargo.toml`'s `layer =` still said
/// `runtimes`. With that corrected the pair is kernel → kernel and the
/// exception went **stale**, which is how the ratchet reported it
/// (`reborn_workspace_crates_declare_layers_and_follow_layer_matrix` fails on
/// a stale entry, not just a new one) — so this deletion is the gate's own
/// verdict rather than a judgement call. Every one of the nine crates that
/// depends on `processes` is kernel or above, so the move legalizes an edge
/// without forbidding any existing one.
///
/// **4 → 2 (WS3, #7067 — the lanes take a narrow budget port).** Both
/// `→ ironclaw_resources` lane rows are deleted, and neither was waived: the
/// production edge is gone from `ironclaw_mcp` and `ironclaw_sandbox` alike.
/// What held them was never the estimate/usage vocabulary the older rows
/// blamed — that already lived in `host_api::resource` and both lanes already
/// imported it from there — but `ResourceGovernor`, a 10-method kernel
/// budget-authority trait of which each lane called exactly three, plus the
/// `ResourceError` denial cone. Relocating those would have moved kernel
/// budget authority into the zero-internal-dep contracts crate, which
/// PROPOSAL §8.3's 2026-08-04 amendment rules out. Instead
/// `ironclaw_host_api::resource` declares `RuntimeResourceBudget` — reserve /
/// reconcile / release and nothing else, over shapes that crate already owned
/// — and `ironclaw_resources` implements it over any `ResourceGovernor`
/// (`GovernorRuntimeBudget`), projecting `ResourceError` onto a narrow
/// classified port error. Dependency inversion, `type-placement.md` §2: the
/// port is declared below and implemented above, so the lanes cannot name the
/// authority at all. `ironclaw_resources` survives in both lanes as a **dev**
/// dependency — the lane suites drive the port over the REAL governor so a
/// denial they assert on is one the kernel produced — and dev edges are
/// outside the matrix by construction (`is_normal_dependency`).
///
/// **2 → 1 / 3 → 1 (2026-08-04, Waves 0–4 batch union).** The two stacked
/// slices each removed their own entries off the same 4-entry base — the
/// governor port (#7160) the two lane `→ resources` rows, the conversations
/// port inversion (#7159) the `conversations → turns` row — and this batch
/// merge is where the union lands: recomputed as `len()` of the merged list
/// per the CHECKLIST §11.2.2 union rule. One entry survives.
///
/// **1 → 0 (WS3 closeout, 2026-08-04 — `ironclaw_extension_support` re-layered
/// `loops` → `runtimes`). The list is empty: PROPOSAL §11.2.2's end state and
/// CHECKLIST WS12's gate condition, reached.** The last entry was
/// `host_runtime → extension_support`, and it did not fall to the shed its
/// `removes_in` named. Two measurements, both on this tree, decided that:
///
/// 1. **The blocker the CHECKLIST row recorded was wrong.** The row said the
///    edge "clears only when the last executor family lands". It is held by
///    exactly **two** `use` sites — `first_party_tools/mod.rs`
///    (`extension_support::coding`) and `first_party_tools/skill_management.rs`
///    (`extension_support::skills`) — and both belong to the families whose
///    executors have **already** moved — `coding` before this row was written,
///    `skills` with the row's family 1. The five families still awaiting a move
///    keep their executors in `host_runtime` and hold no edge at all, so moving
///    them could never have cleared this. (`latency.rs`'s third grep hit is a
///    doc comment.)
/// 2. **The seam makes the edge structural, not transitional.** WS3's own
///    executor/adapter seam — PROPOSAL §8.2's 2026-08-03 note and
///    `families/extensions.md` — says a tool arrives in `extension_support` as
///    an *executor* and leaves its `FirstPartyCapabilityHandler`,
///    `CapabilityManifest` and registry wiring on the host side, because that
///    crate's `BoundaryRule` forbids naming `ironclaw_host_runtime`. That makes
///    the kernel a **designed** consumer. Shedding the two adapters upward
///    anyway costs ~8 kernel private→`pub` widenings (`mod post_edit_check` is
///    private in `lib.rs`; `first_party_capability_manifest`,
///    `resource_profile`, `first_party_origin_gate_matrix` are module-private;
///    `bounded_input_size`, `bounded_output_bytes`,
///    `FIRST_PARTY_MAX_OUTPUT_BYTES` are `pub(super)`) and relocates the
///    registration of **builtin** capabilities that 145 references across 31
///    files reach through `builtin_first_party_*` — a semantic change,
///    not a move. It is the same refutation §6.5.9's binder half already
///    carries: paying a kernel API widening to relocate an adapter whose
///    encapsulation already holds.
///
/// A crate the kernel is designed to call cannot be declared two rungs above
/// it, so the layer is what was wrong. `runtimes` is the **least** demotion
/// that legalizes a kernel consumer, and it is where this crate's own §8.2 row
/// already places it in posture — "mediated services arrive by injection",
/// kernel ✗, invoked only through capability dispatch, which is the wasm / mcp
/// / sandbox cell verbatim. Checked both directions before flipping it: all
/// seven normal dependencies (`auth`, `extractors`, `filesystem`,
/// `observability`, `safety`, `skills` — `substrates`; `host_api` —
/// `contracts`) and every domain the crate's charter reserves (`memory`,
/// `traces`, `triggers` — all `substrates`) fit the narrower row, and all five
/// consumers (`host_runtime` kernel; `extension_host`, `extension_manager`
/// products; `reborn_composition`, `reborn_cli` app) are `kernel` or above, so
/// the move forbids no existing edge. **Zero same-layer edges are created** —
/// no `runtimes` crate is a dependency or a consumer — where `substrates`
/// would have hidden six of this crate's seven dependencies from the matrix.
/// The widening it *does* buy is frozen by the `DowngradePin` in
/// `reborn_same_layer_edge_inventory.rs`, which is the pin #7149 exists for.
///
/// Mechanism, not novelty: this is the fourth time the register has moved by a
/// re-layer and the third `loops → *` demotion in this family — WS2's
/// `ironclaw_extension_registry` (4 entries), WS3's `processes` (1), WS4's `skills`
/// (0). PLAN's Wave 2 note states the rule ("a re-layer *downward* is the
/// cheap kind … expect the exception register to move"). What is **not**
/// closed by it: CHECKLIST WS3's `first_party_tools` row, which is executor
/// consolidation and stays open at five of six families. Only the exception is
/// discharged.
///
/// **The ratchet is now an equality in effect.** With the list empty, any new
/// entry trips `reborn_layer_matrix_exceptions_ratchet_down_only` immediately;
/// re-arming it requires an owner-approved baseline raise in the same PR, per
/// that test's own failure message.
const WS0_LAYER_MATRIX_EXCEPTION_BASELINE: usize = 0;

const LAYER_MATRIX_EXCEPTIONS: &[LayerMatrixException] = &[];

/// The tracking metadata every exception must carry to be removable: the edge
/// it names, when it was taken on, the milestone that deletes it, and why it
/// exists. Returns the first missing field, or `None` when the entry is fully
/// tracked. Placeholders are treated as missing — "TBD" is not a milestone.
fn exception_tracking_defect(exception: &LayerMatrixException) -> Option<&'static str> {
    const PLACEHOLDERS: &[&str] = &["tbd", "todo", "unknown", "n/a", "na", "none", "?", "-"];
    let untracked = |value: &str| {
        let trimmed = value.trim();
        trimmed.is_empty() || PLACEHOLDERS.contains(&trimmed.to_ascii_lowercase().as_str())
    };
    [
        ("crate_name", exception.crate_name),
        ("dependency_name", exception.dependency_name),
        ("introduced", exception.introduced),
        ("removes_in", exception.removes_in),
        ("reason", exception.reason),
    ]
    .into_iter()
    .find_map(|(field, value)| untracked(value).then_some(field))
}

/// §11.2.2 exception ratchet (PROPOSAL §11.2 item 2, CHECKLIST WS10), armed at
/// the WS0 baseline. Two properties hold on every commit from here to WS12's
/// empty list:
///
/// 1. **Shrink-only.** The list cannot grow past the recorded baseline, so a
///    new exception cannot land untracked by quietly appending one more entry.
/// 2. **Every entry is removable.** Each carries a `removes_in` milestone plus
///    the edge/date/reason a reviewer needs — §11.2.2's "adding one requires
///    `removes_in` + an owning issue". The owning-issue half arrives with
///    WS10's §11.2.2 row, which adds the field once the list is short enough
///    for exceptions to be genuinely exceptional.
///
/// The complementary staleness half — an exception whose edge no longer exists
/// must be deleted — is enforced by
/// `reborn_workspace_crates_declare_layers_and_follow_layer_matrix` above, so
/// the two together mean the list can only move toward empty.
#[test]
fn reborn_layer_matrix_exceptions_ratchet_down_only() {
    // Expressed as "how far above the baseline are we", not as
    // `len() <= BASELINE`. The two are the same assertion for every baseline,
    // but the comparison form becomes `usize <= 0` once the baseline reaches
    // its target of zero, which is a tautology to `-D warnings`
    // (`clippy::absurd_extreme_comparisons`) and would have had to be silenced
    // with an `allow` on the one gate whose whole job is to be loud. The
    // ceiling semantics are unchanged and deliberately kept: a list *below* the
    // baseline is fine (the baseline is then lowered in the same PR, per the
    // constant's own doc); only growth past it is red.
    let above_baseline = LAYER_MATRIX_EXCEPTIONS
        .len()
        .saturating_sub(WS0_LAYER_MATRIX_EXCEPTION_BASELINE);
    assert_eq!(
        above_baseline,
        0,
        "layer-matrix exceptions grew to {} (WS0 baseline {}): the restructure's exception \
         list is shrink-only on its way to empty (PROPOSAL §11.2.2). Remove the edge instead \
         of allowlisting it — or, if the owner has approved a genuinely new exception, raise \
         WS0_LAYER_MATRIX_EXCEPTION_BASELINE in the same PR with the rationale in the PR body.",
        LAYER_MATRIX_EXCEPTIONS.len(),
        WS0_LAYER_MATRIX_EXCEPTION_BASELINE
    );

    let untracked = LAYER_MATRIX_EXCEPTIONS
        .iter()
        .filter_map(|exception| {
            exception_tracking_defect(exception).map(|field| {
                format!(
                    "    {} -> {}: missing `{}`",
                    exception.crate_name, exception.dependency_name, field
                )
            })
        })
        .collect::<Vec<_>>();

    assert!(
        untracked.is_empty(),
        "layer-matrix exceptions without tracking metadata (every entry needs a `removes_in` \
         milestone and a stated reason so it can be retired — PROPOSAL §11.2.2):\n{}",
        untracked.join("\n")
    );
}

/// Positive + negative fixtures for the tracking predicate above (WS10: every
/// new ratchet lands with regression fixtures, so the guardrail fails loudly on
/// its own regressions rather than silently passing everything).
#[test]
fn reborn_layer_matrix_exception_tracking_self_test() {
    let tracked = LayerMatrixException {
        crate_name: "ironclaw_example",
        dependency_name: "ironclaw_other",
        introduced: "2026-07-30",
        removes_in: "W7",
        reason: "fixture",
    };
    assert_eq!(exception_tracking_defect(&tracked), None);

    let blank_milestone = LayerMatrixException {
        removes_in: "   ",
        ..tracked
    };
    assert_eq!(
        exception_tracking_defect(&blank_milestone),
        Some("removes_in"),
        "a blank milestone must be reported as untracked"
    );

    let placeholder_milestone = LayerMatrixException {
        removes_in: "TBD",
        ..tracked
    };
    assert_eq!(
        exception_tracking_defect(&placeholder_milestone),
        Some("removes_in"),
        "a placeholder milestone must be reported as untracked"
    );

    let blank_reason = LayerMatrixException {
        reason: "",
        ..tracked
    };
    assert_eq!(
        exception_tracking_defect(&blank_reason),
        Some("reason"),
        "a blank reason must be reported as untracked"
    );
}

fn layer_matrix_exception(
    crate_name: &str,
    dependency_name: &str,
) -> Option<&'static LayerMatrixException> {
    LAYER_MATRIX_EXCEPTIONS.iter().find(|exception| {
        exception.crate_name == crate_name && exception.dependency_name == dependency_name
    })
}

fn package_layer(package: &Value) -> Option<&str> {
    package
        .get("metadata")?
        .get("ironclaw")?
        .get("layer")?
        .as_str()
}

fn is_ironclaw_workspace_package(name: &str) -> bool {
    name == "ironclaw" || name.starts_with("ironclaw_")
}

fn layer_allows_dependency(crate_layer: &str, dependency_layer: &str) -> bool {
    match crate_layer {
        "contracts" => matches!(dependency_layer, "contracts"),
        "substrates" => matches!(dependency_layer, "contracts" | "substrates"),
        "runtimes" => matches!(dependency_layer, "contracts" | "substrates" | "runtimes"),
        "kernel" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel"
        ),
        "loops" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops"
        ),
        "products" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops" | "products"
        ),
        "app" => matches!(
            dependency_layer,
            "contracts" | "substrates" | "runtimes" | "kernel" | "loops" | "products" | "app"
        ),
        // No `legacy` arm: the variant is gone from `IRONCLAW_CRATE_LAYERS`, so
        // a manifest declaring it now fails the unknown-layer assertion before
        // this function is ever reached (§11.3, 2026-08-05).
        _ => false,
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

fn extract_virtual_roots_const(source: &str) -> BTreeSet<String> {
    let const_body = source
        .split("const VIRTUAL_ROOTS: &[&str] = &[")
        .nth(1)
        .and_then(|tail| tail.split("];").next())
        .expect("VIRTUAL_ROOTS const array must be present");
    extract_quoted_absolute_paths(const_body)
}

fn extract_storage_placement_roots(contract: &str) -> BTreeSet<String> {
    contract
        .lines()
        .filter_map(|line| {
            let root = line
                .strip_prefix("| `")?
                .split('`')
                .next()
                .expect("table cell must close code span");
            let root = if root.starts_with("/engine/") {
                "/engine"
            } else {
                root
            };
            Some(root.to_string())
        })
        .filter(|root| is_canonical_virtual_root(root))
        .collect()
}

fn extract_filesystem_namespace_roots(contract: &str) -> BTreeSet<String> {
    let roots_block = contract
        .split("Frozen V1 canonical virtual roots")
        .nth(1)
        .and_then(|tail| tail.split("Recommended meaning:").next())
        .expect("filesystem.md must list frozen V1 canonical virtual roots");
    roots_block
        .lines()
        .map(str::trim)
        .filter(|line| is_canonical_virtual_root(line))
        .map(ToString::to_string)
        .collect()
}

fn extract_quoted_absolute_paths(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix('"')?.split('"').next())
        .filter(|root| is_canonical_virtual_root(root))
        .map(ToString::to_string)
        .collect()
}

fn is_canonical_virtual_root(value: &str) -> bool {
    matches!(
        value,
        "/engine"
            | "/system/settings"
            | "/system/extensions"
            | "/system/skills"
            | "/users"
            | "/projects"
            | "/memory"
            | "/artifacts"
            | "/tmp"
            | "/secrets"
            | "/events"
    )
}

fn package_dependencies(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = workspace_dependency_names(package)
        .filter(|dependency| is_normal_dependency(dependency))
        .filter_map(|dependency| dependency["name"].as_str())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some((name, dependencies))
}

fn package_dependencies_all_kinds(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = workspace_dependency_names(package)
        .filter_map(|dependency| dependency["name"].as_str())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some((name, dependencies))
}

fn workspace_dependency_names(package: &Value) -> impl Iterator<Item = &Value> {
    package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| {
            dependency["name"]
                .as_str()
                .is_some_and(|name| name == "ironclaw" || name.starts_with("ironclaw_"))
        })
}

/// `ironclaw_host_ingress` names no workspace crate but `ironclaw_host_api`, in
/// **any** dependency kind (`normal`, `dev`, `build`).
///
/// Separate from `assert_no_normal_workspace_deps` on purpose: that helper
/// filters to normal dependencies, which is correct for the layer matrix (a
/// dev-dependency on a higher layer is legal and used) and wrong for this
/// crate, whose charter is "Never depends on: anything else, without
/// exception" (`families/product.md`). A dev- or build-dependency still
/// resolves and builds, so it defeats the one property this crate exists to
/// provide just as thoroughly as a normal one would.
///
/// An allowlist, not a blocklist: the forbidden set is *every* workspace
/// `ironclaw_*` crate except the one permitted name, so a dependency nobody
/// anticipated still fails. The package must be present — a silent `return`
/// here would make the whole check vacuous the day the crate is renamed.
fn assert_host_ingress_names_no_other_workspace_crate(packages: &[Value]) {
    const CRATE: &str = "ironclaw_host_ingress";
    const ALLOWED: &str = "ironclaw_host_api";

    let package = packages
        .iter()
        .find(|package| package["name"].as_str() == Some(CRATE))
        .unwrap_or_else(|| {
            panic!("{CRATE} must be a workspace member; without it this check is vacuous")
        });

    let mut violations = Vec::new();
    for dependency in package["dependencies"].as_array().into_iter().flatten() {
        let Some(name) = dependency["name"].as_str() else {
            continue;
        };
        if !is_ironclaw_workspace_package(name) || name == ALLOWED {
            continue;
        }
        let kind = dependency
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("normal");
        violations.push(format!("{name} ({kind} dependency)"));
    }

    assert!(
        violations.is_empty(),
        "{CRATE} must not depend on any workspace crate but {ALLOWED}, in any dependency \
         kind — it exists so a consumer needing only the route-mount carrier shapes never \
         compiles a listener's dependency cone, and a dev/build dependency defeats that as \
         surely as a normal one. Offenders: {}",
        violations.join(", ")
    );
}

fn is_normal_dependency(dependency: &Value) -> bool {
    dependency
        .get("kind")
        .and_then(Value::as_str)
        .is_none_or(|kind| kind == "normal")
}

fn workspace_ironclaw_crates(dependencies: &HashMap<String, Vec<String>>) -> Vec<&str> {
    dependencies
        .keys()
        .filter_map(|name| {
            (name == "ironclaw" || name.starts_with("ironclaw_")).then_some(name.as_str())
        })
        .collect()
}

fn assert_workspace_deps_exactly<'a>(
    dependencies: &HashMap<String, Vec<String>>,
    crate_name: &str,
    expected: impl IntoIterator<Item = &'a str>,
    message: &str,
) {
    let actual = dependencies
        .get(crate_name)
        .unwrap_or_else(|| panic!("{crate_name} must be in cargo metadata"))
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = expected
        .into_iter()
        .map(ToString::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual, expected, "{message}");
}

fn assert_no_normal_workspace_deps<'a>(
    dependencies: &HashMap<String, Vec<String>>,
    crate_name: &str,
    forbidden: impl IntoIterator<Item = &'a str>,
) {
    let Some(actual) = dependencies.get(crate_name) else {
        // The landing plan introduces Reborn crates in grouped PRs. Boundary
        // rules become active as soon as their crate is present in the
        // workspace, while absent future crates are ignored in earlier slices.
        // `reborn_boundary_rules_active_crates_are_workspace_members` covers
        // present-on-disk crates that are missing from `cargo metadata`.
        return;
    };
    for forbidden in forbidden {
        assert!(
            !actual.iter().any(|dependency| dependency == forbidden),
            "{crate_name} must not have a normal dependency on {forbidden}; actual normal ironclaw deps: {actual:?}"
        );
    }
}

/// Recursively concatenate every `.rs` file under `dir` into `out`,
/// descending into subdirectories, and return how many files were read.
/// Matches the recursion pattern used by `collect_forbidden_*` walkers above so
/// future boundary checks over a module tree can reuse the same helper. Used by
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` to scan the entire
/// `runtime/` module tree for forbidden imports, and by
/// `reborn_host_runtime_services_do_not_expose_lower_substrate_handles` to scan
/// the obligation owners.
///
/// The count is returned so a caller can assert it actually read something: a
/// path-keyed scan that silently walks an empty directory reports success while
/// enforcing nothing.
fn collect_runtime_rs(dir: &std::path::Path, out: &mut String) -> usize {
    let mut files = 0usize;
    for entry in std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("directory must be readable at {}: {err}", dir.display()))
    {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files += collect_runtime_rs(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("file {} unreadable: {err}", path.display()));
        out.push_str(&content);
        out.push('\n');
        files += 1;
    }
    files
}

fn collect_forbidden_runtime_network_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenRuntimeNetworkUse],
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_runtime_network_uses(&path, root, forbidden, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (line_number, line) in contents.lines().enumerate() {
            for rule in forbidden {
                if line.contains(rule.pattern) {
                    let relative = path.strip_prefix(root).unwrap_or(&path);
                    violations.push(format!(
                        "{}:{} contains `{}` ({})",
                        relative.display(),
                        line_number + 1,
                        rule.pattern,
                        rule.reason
                    ));
                }
            }
        }
    }
}

fn collect_forbidden_uses(
    dir: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| panic!("failed to read dir entry: {error}"));
        let path = entry.path();
        if path.is_dir() {
            collect_forbidden_uses(&path, root, forbidden, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (line_number, line) in contents.lines().enumerate() {
            for rule in forbidden {
                if rule.exempt.is_some_and(|exempt| exempt(line)) {
                    continue;
                }
                if line.contains(rule.pattern) {
                    let relative = path.strip_prefix(root).unwrap_or(&path);
                    violations.push(format!(
                        "{}:{} contains `{}` ({})",
                        relative.display(),
                        line_number + 1,
                        rule.pattern,
                        rule.reason
                    ));
                }
            }
        }
    }
}

fn collect_forbidden_reborn_auth_path_uses(
    module_dir: &std::path::Path,
    legacy_file: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    if module_dir.is_dir() {
        collect_forbidden_uses(module_dir, root, forbidden, violations);
        return;
    }
    collect_forbidden_reborn_auth_file_uses(legacy_file, root, forbidden, violations);
}

fn collect_forbidden_reborn_auth_file_uses(
    path: &std::path::Path,
    root: &std::path::Path,
    forbidden: &[ForbiddenUse],
    violations: &mut Vec<String>,
) {
    let message = format!(
        "failed to read Reborn product-auth boundary file {}",
        path.display()
    );
    let contents = std::fs::read_to_string(path).expect(&message);
    for (line_number, line) in contents.lines().enumerate() {
        for rule in forbidden {
            if rule.exempt.is_some_and(|exempt| exempt(line)) {
                continue;
            }
            if !line.contains(rule.pattern) {
                continue;
            }
            violations.push(format!(
                "{}:{} contains forbidden product-auth implementation pattern `{}`: {}",
                path.strip_prefix(root).unwrap_or(path).display(),
                line_number + 1,
                rule.pattern,
                rule.reason
            ));
        }
    }
}

fn is_reborn_tracing_target_line(line: &str) -> bool {
    line.contains("target: \"ironclaw::reborn::") || line.contains("target = \"ironclaw::reborn::")
}

#[test]
fn collect_forbidden_reborn_auth_file_uses_detects_violation() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_composition/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let auth_rs = src.join("auth.rs");
    std::fs::write(&auth_rs, "fn forbidden() { let _ = \"reqwest\"; }\n")
        .expect("test auth.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_reborn_auth_file_uses(
        &auth_rs,
        &root,
        &[ForbiddenUse {
            pattern: "reqwest",
            reason: "provider transport must stay outside product auth composition",
            exempt: None,
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert_eq!(violations.len(), 1);
    assert!(
        violations[0].contains("crates/ironclaw_composition/src/auth.rs"),
        "violation should report the relative auth.rs path: {:?}",
        violations
    );
    assert!(
        violations[0].contains("provider transport must stay outside product auth composition"),
        "violation should report the forbidden-use reason: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_reborn_auth_file_uses_allows_reborn_tracing_targets() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-tracing-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_composition/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let auth_rs = src.join("auth.rs");
    std::fs::write(
        &auth_rs,
        "fn allowed() { tracing::warn!(target: \"ironclaw::reborn::product_auth::oauth\"); }\n",
    )
    .expect("test auth.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_reborn_auth_file_uses(
        &auth_rs,
        &root,
        &[ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert!(
        violations.is_empty(),
        "Reborn tracing targets are log namespaces, not v1 root crate references: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_uses_allows_reborn_tracing_targets() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-reborn-auth-boundary-dir-tracing-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/ironclaw_webui/src/product_auth");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let mod_rs = src.join("mod.rs");
    std::fs::write(
        &mod_rs,
        "fn allowed() { tracing::warn!(target: \"ironclaw::reborn::product_auth::oauth\"); }\n",
    )
    .expect("test mod.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_uses(
        &src,
        &root,
        &[ForbiddenUse {
            pattern: "ironclaw::",
            reason: "Reborn product auth must not depend on the v1 root crate",
            exempt: Some(is_reborn_tracing_target_line),
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert!(
        violations.is_empty(),
        "Directory scanner should treat Reborn tracing targets as log namespaces: {:?}",
        violations
    );
}

#[test]
fn collect_forbidden_uses_detects_violation() {
    let root = std::env::temp_dir().join(format!(
        "ironclaw-forbidden-use-dir-test-{}",
        std::process::id()
    ));
    let src = root.join("crates/example/src");
    std::fs::create_dir_all(&src).expect("test source directory must be created");
    let mod_rs = src.join("mod.rs");
    std::fs::write(&mod_rs, "fn forbidden() { let _ = \"reqwest\"; }\n")
        .expect("test mod.rs must be written");

    let mut violations = Vec::new();
    collect_forbidden_uses(
        &src,
        &root,
        &[ForbiddenUse {
            pattern: "reqwest",
            reason: "provider transport must stay outside product auth composition",
            exempt: None,
        }],
        &mut violations,
    );

    std::fs::remove_dir_all(&root).expect("test source directory must be removed");

    assert_eq!(violations.len(), 1);
    assert!(
        violations[0].contains("crates/example/src/mod.rs"),
        "violation should report the relative mod.rs path: {:?}",
        violations
    );
    assert!(
        violations[0].contains("provider transport must stay outside product auth composition"),
        "violation should report the forbidden-use reason: {:?}",
        violations
    );
}

/// Concatenate every `.rs` file under a crate's `src/` for substring scanning.
///
/// Every read is fatal: a scanner that silently skips an unreadable file
/// reports success while measuring nothing (WS10's guardrail rule).
/// Every **production** Rust source under `src_root`, concatenated with
/// comments and string literals blanked.
///
/// The vacuity probes above use this as *proof a lane still exists* — a
/// `contains("pub struct McpRuntime<C>")` that goes green off a comment, a
/// doc-example, or a `#[cfg(test)]` fixture is exactly the dark verdict the
/// probe is there to prevent: the gate would keep passing after the production
/// runtime it guards was deleted. So the concatenation is narrowed twice:
///
/// - [`production_rust_files`] drops `tests.rs`/`*_tests.rs`, files reachable
///   only through a `#[cfg(test)]` `mod` declaration, and the
///   `target`/`node_modules`/`tests` directory names, so no test fixture and no
///   build output can satisfy a probe;
/// - [`strip_comments_and_strings`] blanks comment and string bodies (newlines
///   preserved), so prose and literals cannot either.
///
/// What survives is source tokens, which is what "the lane is still here"
/// means.
fn production_crate_source_tokens(src_root: &std::path::Path) -> String {
    let files = production_rust_files(src_root);
    assert!(
        !files.is_empty(),
        "no production Rust sources found under {}",
        src_root.display()
    );
    let mut out = String::new();
    for path in files {
        out.push_str(&strip_comments_and_strings(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
        ));
        out.push('\n');
    }
    out
}

/// A marker that appears only in non-production text must not satisfy a
/// vacuity probe.
///
/// This is the regression for the fail-open shape
/// [`production_crate_source_tokens`] closes: a raw concatenation answers
/// "yes, the runtime is here" to a comment, a doc example, a string literal, or
/// a `#[cfg(test)]` fixture. Each of those is checked separately so a partial
/// regression (say, dropping the string stripper but keeping the comment one)
/// cannot pass.
#[test]
fn a_marker_in_non_production_text_does_not_prove_a_lane_exists() {
    const MARKER: &str = "pub struct McpRuntime<C>";

    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("tests")).expect("create src/tests");

    std::fs::write(
        src.join("lib.rs"),
        format!(
            "// {MARKER}\n\
             /// Doc example:\n\
             /// ```\n\
             /// {MARKER} {{}}\n\
             /// ```\n\
             /* block: {MARKER} */\n\
             pub const NAME: &str = \"{MARKER}\";\n\
             #[cfg(test)]\n\
             mod fixture;\n\
             pub struct SomethingElse;\n"
        ),
    )
    .expect("write lib.rs");
    // Reachable only through the `#[cfg(test)]` decl above — production-looking
    // name, test-only file.
    std::fs::write(src.join("fixture.rs"), format!("{MARKER} {{}}\n")).expect("write fixture.rs");
    // Conventional test-file names, and a `tests/` directory.
    std::fs::write(src.join("tests.rs"), format!("{MARKER} {{}}\n")).expect("write tests.rs");
    std::fs::write(
        src.join("tests").join("more.rs"),
        format!("{MARKER} {{}}\n"),
    )
    .expect("write tests/more.rs");

    let scanned = production_crate_source_tokens(&src);
    assert!(
        !scanned.contains(MARKER),
        "a marker present only in comments, doc examples, string literals, and \
         test-only files must not satisfy a lane-existence probe; scanned:\n{scanned}"
    );
    assert!(
        scanned.contains("pub struct SomethingElse"),
        "the scan must still see real production tokens; scanned:\n{scanned}"
    );

    // And the positive direction: a real production declaration is found.
    std::fs::write(src.join("runtime.rs"), format!("{MARKER} {{}}\n")).expect("write runtime.rs");
    assert!(
        production_crate_source_tokens(&src).contains(MARKER),
        "a production declaration must satisfy the probe"
    );
}
