use std::fs;
use std::path::PathBuf;

#[test]
fn capabilities_crate_does_not_depend_on_concrete_runtime_or_dispatcher_crates() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = fs::read_to_string(manifest_path).unwrap();
    let production_dependencies = manifest
        .split("\n[dev-dependencies]")
        .next()
        .unwrap_or(&manifest);
    for forbidden in [
        "ironclaw_dispatcher",
        "ironclaw_host_runtime",
        "ironclaw_mcp",
        "ironclaw_scripts",
        "ironclaw_wasm",
        "ironclaw_secrets",
        "ironclaw_network",
    ] {
        assert!(
            !production_dependencies.contains(forbidden),
            "ironclaw_capabilities production code must use neutral ports and must not depend on {forbidden}"
        );
    }
}
