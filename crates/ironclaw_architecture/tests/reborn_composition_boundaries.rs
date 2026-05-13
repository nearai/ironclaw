use std::{collections::HashMap, path::PathBuf, process::Command};

use serde_json::Value;

const COMPOSITION_CRATE: &str = "ironclaw_reborn_composition";

const SUBSTRATE_CRATES: &[&str] = &[
    "ironclaw_host_api",
    "ironclaw_storage",
    "ironclaw_filesystem",
    "ironclaw_events",
    "ironclaw_event_projections",
    "ironclaw_extensions",
    "ironclaw_authorization",
    "ironclaw_run_state",
    "ironclaw_approvals",
    "ironclaw_resources",
    "ironclaw_trust",
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
    "ironclaw_turns",
    "ironclaw_threads",
    "ironclaw_loop_support",
    "ironclaw_reborn",
    "ironclaw_product_adapters",
    "ironclaw_product_workflow",
    "ironclaw_wasm_product_adapters",
];

#[test]
fn no_substrate_crate_depends_on_composition_root() {
    let dependencies = workspace_dependencies();
    for substrate in SUBSTRATE_CRATES {
        let Some(actual) = dependencies.get(*substrate) else {
            continue;
        };
        assert!(
            !actual.iter().any(|dep| dep == COMPOSITION_CRATE),
            "{substrate} must not depend on {COMPOSITION_CRATE}; actual deps: {actual:?}"
        );
    }
}

#[test]
fn composition_root_is_workspace_member() {
    let dependencies = workspace_dependencies();
    assert!(dependencies.contains_key(COMPOSITION_CRATE));
}

#[test]
fn composition_public_api_is_facade_shaped() {
    let lib = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_reborn_composition/src/lib.rs"),
    )
    .expect("composition lib readable");
    let input = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_reborn_composition/src/input.rs"),
    )
    .expect("composition input readable");
    let factory = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_reborn_composition/src/factory.rs"),
    )
    .expect("composition factory readable");
    let public_surface = format!("{lib}\n{input}\n{factory}");

    assert!(
        !lib.contains("RebornStorageInput") && !lib.contains("RebornEventStoreConfig"),
        "composition crate must not re-export raw storage/event-store input types"
    );
    assert!(
        !input.contains("pub enum RebornStorageInput"),
        "RebornStorageInput must stay crate-private"
    );
    assert!(
        !input.contains("pub db:") && !input.contains("pub pool:"),
        "raw database handles must not be public struct/enum fields"
    );

    for forbidden in [
        "pub run_state_store",
        "pub approval_request_store",
        "pub capability_lease_store",
        "pub event_log",
        "pub audit_log",
        "pub secret_store",
        "pub network_enforcer",
        "pub process_services",
        "pub filesystem_root",
        "pub resource_governor",
        "LegacyBridgeMode",
    ] {
        assert!(
            !public_surface.contains(forbidden),
            "composition root public API must not expose `{forbidden}`"
        );
    }
}

fn workspace_dependencies() -> HashMap<String, Vec<String>> {
    cargo_metadata()["packages"]
        .as_array()
        .expect("packages")
        .iter()
        .filter_map(package_dependencies)
        .collect()
}

fn cargo_metadata() -> Value {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .expect("cargo metadata");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("metadata json")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("architecture crate under crates")
        .to_path_buf()
}

fn package_dependencies(package: &Value) -> Option<(String, Vec<String>)> {
    let name = package["name"].as_str()?.to_string();
    let dependencies = package["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|dependency| {
            dependency
                .get("kind")
                .and_then(Value::as_str)
                .is_none_or(|kind| kind == "normal")
        })
        .filter_map(|dependency| dependency["name"].as_str())
        .filter(|name| name.starts_with("ironclaw_"))
        .map(ToString::to_string)
        .collect();
    Some((name, dependencies))
}
