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
    "ironclaw_reborn_openai_compat",
    "ironclaw_product_adapters",
    "ironclaw_product_workflow",
    "ironclaw_triggers",
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

#[test]
fn legacy_main_does_not_compose_reborn_runtime() {
    let root = workspace_root();
    let legacy_main =
        std::fs::read_to_string(root.join("src/main.rs")).expect("legacy main.rs readable");

    for forbidden in [
        "ironclaw_reborn_composition",
        "ironclaw_reborn_cli",
        "build_reborn_runtime",
        "build_reborn_services",
        "RebornBuildInput",
        "RebornRuntimeInput",
        "RebornCompositionProfile",
    ] {
        assert!(
            !legacy_main.contains(forbidden),
            "legacy src/main.rs must stay on the v1/AppBuilder path and must not compose \
             Reborn runtime startup directly; found `{forbidden}`"
        );
    }
}

#[test]
fn reborn_binary_main_is_thin_bootstrap() {
    let root = workspace_root();
    let reborn_main = std::fs::read_to_string(root.join("crates/ironclaw_reborn_cli/src/main.rs"))
        .expect("reborn cli main.rs readable");

    assert!(
        reborn_main.contains("cli::run()"),
        "ironclaw-reborn main.rs should delegate to the clap command root"
    );
    for forbidden in [
        "build_reborn_runtime",
        "build_reborn_services",
        "axum::serve",
        "TcpListener::bind",
        "src/channels/web",
        "ironclaw::channels::web",
    ] {
        assert!(
            !reborn_main.contains(forbidden),
            "ironclaw-reborn main.rs must stay a thin bootstrap over Reborn-owned command \
             modules and factories; found `{forbidden}`"
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
        // Dedicated unit-test module files (e.g. `hooks/tests.rs`, declared via
        // `#[cfg(test)] mod tests;`) legitimately exercise builder APIs directly
        // and are not production code. Skip them wholesale — the inline
        // `#[cfg(test)] mod tests { .. }` stripping below covers same-file test
        // modules. This keeps the invariant robust across the #3951 finding-#4
        // decomposition (which moved the test matrix into its own file).
        if is_test_module_file(path) {
            continue;
        }
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

/// Strip trailing `#[cfg(test)] mod <name> { .. }` unit-test module(s) so the
/// architecture invariant applies to production code only (tests legitimately
/// exercise builder APIs directly). Matches the `#[cfg(test)]` attribute line
/// immediately followed by a `mod ` declaration of ANY name (so a refactor that
/// renames `mod tests` or adds a second `#[cfg(test)] mod` block is still fully
/// stripped), and cuts from the FIRST such occurrence onward — a bare
/// `#[cfg(test)]` substring also appears in doc comments, hence the `\nmod `
/// anchor rather than a bare match.
fn strip_test_module(contents: &str) -> &str {
    match contents.find("#[cfg(test)]\nmod ") {
        Some(idx) => &contents[..idx],
        None => contents,
    }
}

/// A dedicated unit-test module file: a `tests.rs` module file, or any source
/// under a `tests/` directory. These are test-only and may use builder APIs
/// directly, so the production-only invariant scan skips them.
fn is_test_module_file(path: &Path) -> bool {
    path.file_name().and_then(|name| name.to_str()) == Some("tests.rs")
        || path
            .components()
            .any(|component| component.as_os_str() == "tests")
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
