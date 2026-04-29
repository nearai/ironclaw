//! Architecture boundary tests for the Reborn composition root.
//!
//! `ironclaw_reborn_composition` sits at the top of the substrate stack: it
//! is allowed to depend on every substrate crate (that's the point — it
//! composes them) but **no substrate crate may depend on it**. Reversing the
//! direction would make the composition root a load-bearing piece of the
//! kernel boundary, which #3026 explicitly forbids.
//!
//! These rules complement the per-substrate rules in
//! `reborn_dependency_boundaries.rs`. They live in a separate file so that
//! the composition crate can be added to the workspace incrementally without
//! editing the broader rule list.

use std::{collections::HashMap, path::PathBuf, process::Command};

use serde_json::Value;

const COMPOSITION_CRATE: &str = "ironclaw_reborn_composition";

/// Every Reborn substrate crate currently in the workspace, plus every
/// substrate crate the landing plan reserves names for. Listing both keeps
/// the test useful as future crates merge — the rule fails the moment a
/// substrate crate accidentally imports the composition root.
const SUBSTRATE_CRATES: &[&str] = &[
    "ironclaw_host_api",
    "ironclaw_filesystem",
    "ironclaw_events",
    "ironclaw_extensions",
    "ironclaw_authorization",
    "ironclaw_run_state",
    "ironclaw_approvals",
    "ironclaw_resources",
    // Crates not yet in the workspace; rule is conservative — when each
    // lands, this test will start enforcing the same direction without an
    // edit here.
    "ironclaw_capabilities",
    "ironclaw_dispatcher",
    "ironclaw_processes",
    "ironclaw_secrets",
    "ironclaw_network",
    "ironclaw_memory",
    "ironclaw_host_runtime",
    "ironclaw_mcp",
    "ironclaw_scripts",
    "ironclaw_wasm",
];

#[test]
fn no_substrate_crate_depends_on_composition_root() {
    let dependencies = workspace_dependencies();

    for substrate in SUBSTRATE_CRATES {
        let Some(actual) = dependencies.get(*substrate) else {
            // Substrate not yet in the workspace — rule activates when it
            // lands. Mirrors the policy in `reborn_dependency_boundaries.rs`.
            continue;
        };
        assert!(
            !actual.iter().any(|dep| dep == COMPOSITION_CRATE),
            "{substrate} must not depend on {COMPOSITION_CRATE}; \
             the composition root composes substrate, not the other way around. \
             actual normal ironclaw deps: {actual:?}"
        );
    }
}

#[test]
fn composition_root_is_present_in_workspace() {
    // Defensive: this test is the only thing that breaks if the crate is
    // accidentally removed from the workspace `members` list. Substrate
    // production code never imports it, so a missing entry would otherwise
    // fail silently for everyone except the binary crate that wires it in.
    let dependencies = workspace_dependencies();
    assert!(
        dependencies.contains_key(COMPOSITION_CRATE),
        "{COMPOSITION_CRATE} must be a workspace member"
    );
}

fn workspace_dependencies() -> HashMap<String, Vec<String>> {
    let metadata = cargo_metadata();
    let packages = metadata["packages"]
        .as_array()
        .expect("cargo metadata must include packages");
    packages
        .iter()
        .filter_map(package_dependencies)
        .collect::<HashMap<_, _>>()
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate must live under crates/ironclaw_architecture")
        .to_path_buf()
}

fn package_dependencies(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| is_normal_dependency(dependency))
        .filter_map(|dependency| dependency["name"].as_str())
        .filter(|name| name.starts_with("ironclaw_"))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some((name, dependencies))
}

fn is_normal_dependency(dependency: &Value) -> bool {
    dependency
        .get("kind")
        .and_then(Value::as_str)
        .is_none_or(|kind| kind == "normal")
}
