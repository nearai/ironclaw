use std::{collections::HashMap, path::PathBuf, process::Command};

use serde_json::Value;

const COMPOSITION_CRATE: &str = "ironclaw_reborn_composition";

const SUBSTRATE_CRATES: &[&str] = &[
    "ironclaw_auth",
    "ironclaw_host_api",
    "ironclaw_storage",
    "ironclaw_filesystem",
    "ironclaw_events",
    "ironclaw_event_projections",
    "ironclaw_event_streams",
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
        !lib.contains("pub use input::RebornStorageInput"),
        "composition facade API must not re-export raw storage input types"
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

/// The third-party hook-projection path in `hooks.rs` MUST install exclusively
/// through `HookRegistrar::install`, never the lower-level
/// `HookDispatcherBuilder::install_installed_*` methods. The registrar is the
/// single seam that (a) enforces the Installed-tier ceiling and the
/// per-extension caps, and (b) derives `owning_extension` from the installer
/// argument (spoof-blocked). The direct builder installers accept
/// `owning_extension` as a free parameter and would bypass both. This source
/// assertion pins the registrar-only invariant so a future refactor cannot
/// silently introduce a bypass for untrusted extensions.
#[test]
fn hook_projection_path_never_calls_direct_installed_builder_api() {
    let hooks = std::fs::read_to_string(
        workspace_root().join("crates/ironclaw_reborn_composition/src/hooks.rs"),
    )
    .expect("composition hooks.rs readable");

    // Strip the unit-test module: tests legitimately exercise builder APIs
    // directly, and the architecture invariant is about the production
    // projection path only. Match the module attribute line specifically (a
    // bare `#[cfg(test)]` substring also appears in a module doc comment).
    let production = match hooks.find("#[cfg(test)]\nmod tests") {
        Some(idx) => &hooks[..idx],
        None => hooks.as_str(),
    };

    for forbidden in [
        "install_installed_before_capability",
        "install_installed_before_prompt",
        "install_installed_observer",
        "install_installed_event_triggered",
        "install_installed_wasm_before_capability",
        "install_installed_wasm_before_prompt",
        "install_installed_wasm_observer",
    ] {
        assert!(
            !production.contains(forbidden),
            "third-party hook projection in hooks.rs must go through \
             HookRegistrar::install, never the direct builder installer \
             `{forbidden}` (registrar-only invariant: ceiling + spoof-blocked \
             owning_extension)"
        );
    }

    // Positive anchor: the projection path DOES route through the registrar, so
    // the negative assertions above are not vacuously true.
    assert!(
        production.contains("registrar.install("),
        "the projection path must install through HookRegistrar::install"
    );
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
