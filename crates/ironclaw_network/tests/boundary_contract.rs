#[test]
fn network_crate_does_not_depend_on_workflow_runtime_secret_or_observability_crates() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));

    for forbidden in [
        "ironclaw_authorization",
        "ironclaw_approvals",
        "ironclaw_capabilities",
        "ironclaw_dispatcher",
        "ironclaw_events",
        "ironclaw_extensions",
        "ironclaw_filesystem",
        "ironclaw_host_runtime",
        "ironclaw_mcp",
        "ironclaw_processes",
        "ironclaw_resources",
        "ironclaw_run_state",
        "ironclaw_scripts",
        "ironclaw_secrets",
        "ironclaw_wasm",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "ironclaw_network must stay a low-level scoped network policy service, not depend on {forbidden}"
        );
    }
}
