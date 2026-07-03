#[test]
fn authorization_crate_stays_below_workflow_and_runtime_crates() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));
    let dependencies = dependencies_section(&manifest);

    for forbidden in [
        "ironclaw_approvals",
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
            "ironclaw_authorization should evaluate grants/leases without depending on {forbidden}"
        );
    }
}

#[test]
fn filesystem_lease_store_does_not_block_on_async_filesystem_calls() {
    let lib_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let lib = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|error| panic!("failed to read {lib_path:?}: {error}"));

    assert!(
        !lib.contains("block_on"),
        "filesystem-backed lease persistence should be async instead of blocking on RootFilesystem futures"
    );
}

fn dependencies_section(manifest: &str) -> &str {
    manifest
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]").map(|(deps, _)| deps))
        .expect("Cargo.toml must contain [dependencies] before [dev-dependencies]")
}
