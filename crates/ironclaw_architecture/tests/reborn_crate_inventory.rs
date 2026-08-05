//! Self-test for the crate inventory the path-keyed architecture gates resolve
//! through (`ratchet_support::{crate_directories, crate_path, …}`).
//!
//! WS10, "loud path-pattern inventory". Roughly 450 literal `crates/ironclaw_*`
//! spellings live in this directory's gates. Wave 5 moves every one of those
//! crates into a family directory, and a gate that resolves its scope with a
//! bare `root.join("crates/ironclaw_x/src")` then scans a directory that is not
//! there. The gates now spell the crate NAME and resolve the DIRECTORY, so the
//! move needs no lockstep sweep of the literals — which is only true if the
//! resolver itself is pinned, both against the rule
//! `scripts/ci/lib/crate_tree.py` states for the Python-side gates and against
//! a tree that has actually been moved.
//!
//! Every case below is paired: the property, and the sabotage that must break
//! it. A resolver that answered "no such crate" with a path matching nothing
//! would reintroduce exactly the silent-dark failure WS0 closed.

mod ratchet_support;

use std::path::{Path, PathBuf};
use std::process::Command;

use ratchet_support::{
    MIN_CRATE_DIRECTORIES, crate_dir, crate_directories, crate_path, nested_workspace_roots,
    resolve_crate_relative, try_crate_directories, try_crate_directory, try_resolve_crate_relative,
    workspace_root,
};

/// Crates named by enough gates that losing one would blind a whole family of
/// them. Named rather than counted so the assertion says what it lost.
const REPRESENTATIVE_CRATES: &[&str] = &[
    "ironclaw_architecture",
    "ironclaw_extension_support",
    "ironclaw_filesystem",
    "ironclaw_host_api",
    "ironclaw_llm",
    "ironclaw_reborn_cli",
    "ironclaw_reborn_composition",
    "ironclaw_webui",
];

// ---------------------------------------------------------------------------
// The real tree
// ---------------------------------------------------------------------------

#[test]
fn reborn_crate_inventory_measures_the_real_tree() {
    let root = workspace_root();
    let inventory = crate_directories(&root);

    assert!(
        inventory.len() >= MIN_CRATE_DIRECTORIES,
        "crate inventory found {} directories, below the fail-closed floor of {}",
        inventory.len(),
        MIN_CRATE_DIRECTORIES
    );

    for name in REPRESENTATIVE_CRATES {
        let directory = try_crate_directory(&root, name)
            .unwrap_or_else(|error| panic!("inventory must resolve {name}: {error}"));
        assert!(
            root.join(&directory).join("Cargo.toml").is_file(),
            "inventory resolved {name} to {directory}, which owns no Cargo.toml"
        );
    }

    // Outermost wins: no entry may be a prefix of another, or `owning crate`
    // would depend on iteration order.
    for outer in &inventory {
        for inner in &inventory {
            assert!(
                outer == inner || !inner.starts_with(&format!("{outer}/")),
                "inventory entry {inner} is nested inside {outer}; outermost-wins pruning failed"
            );
        }
    }

    // The separate-workspace exclusion is a real exclusion, not an empty rule:
    // the silk decoder and the six wasm-src guests are the members today.
    let guests = nested_workspace_roots(&root);
    assert!(
        guests.len() >= 2,
        "expected the separate-workspace roots (silk decoder + wasm-src guests) to be \
         excluded by construction, found {guests:?}"
    );
    for guest in &guests {
        assert!(
            !inventory.contains(guest),
            "{guest} declares its own [workspace] and must not be inventoried as a crate of \
             this one"
        );
    }
}

/// The Python-side gates (`scripts/ci/lib/crate_tree.py`) and this module must
/// answer the same question the same way; two inventories that drift are two
/// different definitions of "which directories are crates", and only one of
/// them gets fixed when the tree moves.
///
/// Fail-closed on a missing `python3` rather than skipping: this repository's
/// pre-push hook already runs Python gates, and a guardrail that quietly opts
/// out on a machine is the class of defect this row exists to close.
#[test]
fn reborn_rust_and_python_crate_inventories_agree() {
    let root = workspace_root();
    let script = root.join("scripts/ci/lib/crate_tree.py");
    assert!(
        script.is_file(),
        "the Python crate inventory must exist at {} — it is the shared definition this \
         module mirrors",
        script.display()
    );

    let output = Command::new("python3")
        .arg(&script)
        .arg(&root)
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "cannot run python3 {}: {error}. python3 is required to cross-check the two \
                 crate inventories; this test refuses rather than silently skipping.",
                script.display()
            )
        });
    assert!(
        output.status.success(),
        "python3 crate inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let python: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        crate_directories(&root),
        python,
        "the Rust and Python crate inventories disagree — fix the rule in both \
         (ratchet_support::try_crate_directories and scripts/ci/lib/crate_tree.py) rather \
         than letting the gates that use each scan different trees"
    );
}

/// The behavior-free half of the adoption, stated so it survives the move it
/// exists for: a logical `crates/<crate>/<rest>` spelling always resolves to
/// the crate's REAL directory plus `<rest>`, and non-crate paths pass through
/// untouched. On today's flat tree every one of these is the identity — which
/// is the evidence that repointing ~450 literals through the resolver changed
/// no gate's verdict — and after Wave 5 the same assertions still hold without
/// an edit. (`reborn_resolution_is_the_identity_on_a_flat_fixture_tree` pins the
/// identity claim itself, on a tree whose shape this test cannot lose.)
#[test]
fn reborn_logical_spellings_resolve_to_each_crates_real_directory() {
    let root = workspace_root();

    for (name, rest) in [
        ("ironclaw_llm", "src/"),
        ("ironclaw_llm", "src/lib.rs"),
        ("ironclaw_reborn_cli", "Cargo.toml"),
        ("ironclaw_webui", "frontend/src"),
        ("ironclaw_extension_support", "src"),
        ("slack", "src"),
        // A path that does not exist but sits inside a crate that does still
        // resolves — absence assertions stay the caller's job, not the
        // resolver's.
        ("ironclaw_llm", "src/definitely_absent.rs"),
    ] {
        let directory = try_crate_directory(&root, name)
            .unwrap_or_else(|error| panic!("inventory must resolve {name}: {error}"));
        let logical = format!("crates/{name}/{rest}");
        assert_eq!(
            resolve_crate_relative(&root, &logical),
            format!("{directory}/{rest}"),
            "{logical} must resolve to the crate's real directory"
        );
        assert_eq!(
            crate_path(&root, &logical),
            root.join(&directory).join(rest)
        );
    }

    // Owned by no crate, or not under `crates/` at all: unchanged, always.
    for spec in [
        "crates/extensions/packages/github/manifest.toml",
        "crates/extensions/packages",
        "crates/AGENTS.md",
        "tests/fixtures/extensions",
        "scripts/ci/lib/crate_tree.py",
    ] {
        assert_eq!(
            resolve_crate_relative(&root, spec),
            spec,
            "{spec} is owned by no crate and must pass through untouched"
        );
        assert_eq!(crate_path(&root, spec), root.join(spec));
    }
}

/// The identity claim, pinned on a tree whose flatness is guaranteed by the
/// fixture rather than by the repository's current shape. This is what makes
/// "adopting the resolver changed nothing" checkable forever, including from a
/// checkout where the family move has already landed.
#[test]
fn reborn_resolution_is_the_identity_on_a_flat_fixture_tree() {
    let (_guard, root) = fixture_root(|root| {
        write(&root.join("crates/ironclaw_llm/src/lib.rs"), "// fixture\n");
        write(&root.join("crates/ironclaw_llm/Cargo.toml"), "[package]\n");
        write(&root.join("crates/AGENTS.md"), "# fixture\n");
    });

    for spec in [
        "crates/ironclaw_llm",
        "crates/ironclaw_llm/src/",
        "crates/ironclaw_llm/src/lib.rs",
        "crates/ironclaw_llm/Cargo.toml",
        "crates/ironclaw_llm/src/definitely_absent.rs",
        "crates/AGENTS.md",
        "tests/fixtures/extensions",
    ] {
        assert_eq!(
            resolve_crate_relative(&root, spec),
            spec,
            "on a flat tree {spec} must resolve to itself, or adopting the resolver is not a \
             behavior-free change"
        );
        assert_eq!(crate_path(&root, spec), root.join(spec));
    }
}

// ---------------------------------------------------------------------------
// Fixture trees: the move, and every way resolution must refuse
// ---------------------------------------------------------------------------

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("fixture directory");
    }
    std::fs::write(path, contents).expect("fixture file");
}

/// A tree with `count` flat crates plus whatever `extra` adds, so a fixture can
/// clear the fail-closed floor without hand-writing twenty manifests.
fn fixture_root(extra: impl FnOnce(&Path)) -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().expect("tempdir");
    let root = temporary.path().to_path_buf();
    write(&root.join("Cargo.toml"), "[workspace]\nmembers = []\n");
    for index in 0..MIN_CRATE_DIRECTORIES {
        write(
            &root.join(format!("crates/ironclaw_filler_{index}/Cargo.toml")),
            &format!("[package]\nname = \"ironclaw_filler_{index}\"\n"),
        );
    }
    extra(&root);
    (temporary, root)
}

#[test]
fn reborn_crate_moved_into_a_family_directory_still_resolves() {
    let (_guard, root) = fixture_root(|root| {
        write(
            &root.join("crates/substrates/ironclaw_llm/Cargo.toml"),
            "[package]\nname = \"ironclaw_llm\"\n",
        );
        write(&root.join("crates/substrates/ironclaw_llm/src/lib.rs"), "");
        // Already-nested today, and NOT moved by the family pass: it must keep
        // resolving to where it really sits rather than being collapsed.
        write(
            &root.join("crates/extensions/ironclaw_extension_support/Cargo.toml"),
            "[package]\nname = \"ironclaw_extension_support\"\n",
        );
    });

    assert_eq!(
        resolve_crate_relative(&root, "crates/ironclaw_llm/src/lib.rs"),
        "crates/substrates/ironclaw_llm/src/lib.rs",
        "a gate spelling the crate flatly must follow it into its family directory"
    );
    assert_eq!(
        crate_path(&root, "crates/ironclaw_llm/src/lib.rs"),
        root.join("crates/substrates/ironclaw_llm/src/lib.rs")
    );
    assert_eq!(
        resolve_crate_relative(&root, "crates/ironclaw_llm/src/"),
        "crates/substrates/ironclaw_llm/src/",
        "a trailing slash must survive, or `contains` fragments stop matching directories"
    );
    assert_eq!(
        crate_dir(&root, "ironclaw_llm"),
        root.join("crates/substrates/ironclaw_llm")
    );
    assert_eq!(
        resolve_crate_relative(&root, "crates/extensions/ironclaw_extension_support/src"),
        "crates/extensions/ironclaw_extension_support/src",
        "an entry already spelled at its real location must not be rewritten"
    );
    assert_eq!(
        resolve_crate_relative(&root, "crates/ironclaw_extension_support"),
        "crates/extensions/ironclaw_extension_support",
        "…and the flat spelling of that same crate must still resolve to it"
    );
}

#[test]
fn reborn_crate_that_no_longer_exists_is_refused_not_answered() {
    let (_guard, root) = fixture_root(|_| {});

    let error = try_resolve_crate_relative(&root, "crates/ironclaw_deleted/src/lib.rs")
        .expect_err("a spec naming a crate that is not there must refuse");
    assert!(
        error.contains("ironclaw_deleted") && error.contains("repoint"),
        "the refusal must name the unresolvable crate and say to repoint it, got: {error}"
    );

    let error = try_crate_directory(&root, "ironclaw_deleted")
        .expect_err("resolving a missing crate by name must refuse");
    assert!(
        error.contains("found 0"),
        "the refusal must report how many candidates it found, got: {error}"
    );
}

#[test]
fn reborn_ambiguous_crate_name_is_refused_not_picked() {
    let (_guard, root) = fixture_root(|root| {
        write(
            &root.join("crates/substrates/ironclaw_llm/Cargo.toml"),
            "[package]\nname = \"ironclaw_llm\"\n",
        );
        write(
            &root.join("crates/lanes/ironclaw_llm/Cargo.toml"),
            "[package]\nname = \"ironclaw_llm\"\n",
        );
    });

    let error = try_crate_directory(&root, "ironclaw_llm")
        .expect_err("two directories with the same basename must refuse, not resolve to one");
    assert!(
        error.contains("found 2"),
        "the refusal must say the name is ambiguous, got: {error}"
    );
}

#[test]
fn reborn_truncated_tree_refuses_rather_than_reporting_an_empty_inventory() {
    let temporary = tempfile::tempdir().expect("tempdir");
    let root = temporary.path();
    write(&root.join("Cargo.toml"), "[workspace]\n");
    write(
        &root.join("crates/ironclaw_only/Cargo.toml"),
        "[package]\nname = \"ironclaw_only\"\n",
    );

    let error = try_crate_directories(root)
        .expect_err("an inventory below the floor must refuse, not be returned");
    assert!(
        error.contains(&MIN_CRATE_DIRECTORIES.to_string()) && error.contains("Failing closed"),
        "the refusal must cite the floor and say it is failing closed, got: {error}"
    );

    let missing = tempfile::tempdir().expect("tempdir");
    let error = try_crate_directories(missing.path())
        .expect_err("a tree with no crates/ directory must refuse");
    assert!(
        error.contains("crate discovery cannot run"),
        "the refusal must say discovery could not run, got: {error}"
    );
}

#[test]
fn reborn_separate_workspaces_nested_manifests_and_build_output_are_excluded() {
    let (_guard, root) = fixture_root(|root| {
        // A guest component: its own workspace, never built here.
        write(
            &root.join("crates/extensions/packages/slack/wasm-src/Cargo.toml"),
            "[workspace]\n[package]\nname = \"slack_guest\"\n",
        );
        // A package directory that IS a crate of this workspace, with a guest
        // nested inside it — the guest must not shadow or split the package.
        write(
            &root.join("crates/extensions/packages/slack/Cargo.toml"),
            "[package]\nname = \"slack\"\n",
        );
        // Outermost wins: a fuzz manifest inside a crate is not a second crate.
        write(
            &root.join("crates/ironclaw_safety/Cargo.toml"),
            "[package]\nname = \"ironclaw_safety\"\n",
        );
        write(
            &root.join("crates/ironclaw_safety/fuzz/Cargo.toml"),
            "[package]\nname = \"ironclaw_safety_fuzz\"\n",
        );
        // Build output and dotted directories are not crates.
        write(
            &root.join("crates/ironclaw_safety/target/debug/build/x/Cargo.toml"),
            "[package]\nname = \"stale\"\n",
        );
        write(
            &root.join("crates/.cargo_vendor/thing/Cargo.toml"),
            "[package]\nname = \"vendored\"\n",
        );
    });

    let inventory = crate_directories(&root);
    assert!(inventory.contains(&"crates/ironclaw_safety".to_string()));
    assert!(inventory.contains(&"crates/extensions/packages/slack".to_string()));
    for excluded in [
        "crates/extensions/packages/slack/wasm-src",
        "crates/ironclaw_safety/fuzz",
        "crates/ironclaw_safety/target/debug/build/x",
        "crates/.cargo_vendor/thing",
    ] {
        assert!(
            !inventory.contains(&excluded.to_string()),
            "{excluded} must not be inventoried as a crate, got {inventory:?}"
        );
    }
    assert_eq!(
        nested_workspace_roots(&root),
        vec!["crates/extensions/packages/slack/wasm-src".to_string()],
    );
}
