use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};

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

/// The third-party hook-projection path MUST install installed-tier bindings
/// EXCLUSIVELY through `HookRegistrar::install`, never any lower-level
/// `HookDispatcherBuilder` primitive that could mint an `Installed`-tier binding
/// with a caller-chosen `owning_extension`. The registrar is the single seam
/// that (a) enforces the Installed-tier ceiling and the per-extension caps, and
/// (b) derives `owning_extension` from the installer argument (spoof-blocked).
///
/// # Why scan the WHOLE composition crate, not just `hooks.rs`
///
/// A substring scan limited to `hooks.rs` is evadable by a future refactor that
/// moves a bypass into a sibling helper module, or that hand-builds a
/// `HookBinding { trust_class: Installed, .. }` and calls `insert_binding`, or
/// that calls the generic `install_observer(.. HookTrustClass::Installed ..)`.
/// This test therefore scans EVERY non-test source line of
/// `ironclaw_reborn_composition` (the crate that owns the untrusted projection
/// path) and forbids ALL of those Installed-tier-minting primitives crate-wide.
/// The only sanctioned way for this crate to install an installed-tier binding
/// is `HookRegistrar::install`.
#[test]
fn composition_crate_installs_installed_tier_only_through_registrar() {
    let crate_src = workspace_root().join("crates/ironclaw_reborn_composition/src");
    let sources = rust_sources(&crate_src);
    assert!(
        !sources.is_empty(),
        "expected to find composition crate source files under {crate_src:?}"
    );

    // Direct builder/dispatcher primitives that can mint an Installed-tier
    // binding while accepting `owning_extension` (or a hand-built trust class)
    // as a free parameter — every one of these bypasses the registrar's
    // ceiling + spoof-blocked attribution.
    const FORBIDDEN_INSTALLED_PRIMITIVES: &[&str] = &[
        "install_installed_before_capability",
        "install_installed_before_prompt",
        "install_installed_observer",
        "install_installed_event_triggered",
        "install_installed_wasm_before_capability",
        "install_installed_wasm_before_prompt",
        "install_installed_wasm_observer",
        // Generic, trust-class-parameterized installer + the raw binding
        // insertion path: either could carry `HookTrustClass::Installed`.
        "install_observer(",
        "insert_binding(",
        // A hand-constructed installed-tier binding is the lowest-level bypass.
        "HookTrustClass::Installed",
    ];

    let mut saw_registrar_install = false;
    for (path, contents) in &sources {
        let production = strip_test_module(contents);
        if production.contains("registrar.install(") {
            saw_registrar_install = true;
        }
        for forbidden in FORBIDDEN_INSTALLED_PRIMITIVES {
            assert!(
                !production.contains(forbidden),
                "{path:?}: third-party hook projection must install installed-tier \
                 bindings ONLY through HookRegistrar::install, never the direct \
                 primitive `{forbidden}` (registrar-only invariant: ceiling + \
                 spoof-blocked owning_extension). Move this into the registrar or \
                 use HookRegistrar::install."
            );
        }
    }

    // Positive anchor: the projection path DOES route through the registrar, so
    // the negative assertions above are not vacuously true.
    assert!(
        saw_registrar_install,
        "the projection path must install through HookRegistrar::install \
         somewhere in the composition crate"
    );
}

/// Strip a trailing `#[cfg(test)] mod tests { .. }` unit-test module so the
/// architecture invariant applies to production code only (tests legitimately
/// exercise builder APIs directly). Matches the module attribute line
/// specifically — a bare `#[cfg(test)]` substring also appears in doc comments.
fn strip_test_module(contents: &str) -> &str {
    match contents.find("#[cfg(test)]\nmod tests") {
        Some(idx) => &contents[..idx],
        None => contents,
    }
}

/// Recursively collect `(path, contents)` for every `.rs` file under `dir`.
fn rust_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let contents = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("readable rust source {path:?}: {error}"));
                out.push((path, contents));
            }
        }
    }
    out
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
