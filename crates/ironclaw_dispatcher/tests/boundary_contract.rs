#[test]
fn dispatcher_crate_does_not_depend_on_higher_level_workflow_crates() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));

    for forbidden in [
        "ironclaw_authorization",
        "ironclaw_capabilities",
        "ironclaw_wasm",
        "ironclaw_scripts",
        "ironclaw_mcp",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "ironclaw_dispatcher examples/tests should exercise the dispatcher boundary directly, not depend on higher-level workflow crate {forbidden}"
        );
    }
}
