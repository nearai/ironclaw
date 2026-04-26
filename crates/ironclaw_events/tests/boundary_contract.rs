#[test]
fn events_crate_stays_below_workflow_and_runtime_crates() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));
    let dependencies = dependencies_section(&manifest);

    for forbidden in [
        "ironclaw_authorization",
        "ironclaw_approvals",
        "ironclaw_run_state",
        "ironclaw_capabilities",
        "ironclaw_dispatcher",
        "ironclaw_processes",
        "ironclaw_host_runtime",
        "ironclaw_resources",
        "ironclaw_extensions",
        "ironclaw_wasm",
        "ironclaw_scripts",
        "ironclaw_mcp",
    ] {
        assert!(
            !dependencies.contains(forbidden),
            "ironclaw_events must remain a low-level sink/envelope crate and not depend on {forbidden}"
        );
    }
}

fn dependencies_section(manifest: &str) -> &str {
    manifest
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]").map(|(deps, _)| deps))
        .expect("Cargo.toml must contain [dependencies] before [dev-dependencies]")
}
