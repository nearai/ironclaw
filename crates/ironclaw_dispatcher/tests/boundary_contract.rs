#[test]
fn dispatcher_crate_is_only_a_compatibility_reexport() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));
    let lib_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("lib.rs");
    let lib = std::fs::read_to_string(&lib_path)
        .unwrap_or_else(|error| panic!("failed to read {lib_path:?}: {error}"));

    assert!(
        manifest.contains("ironclaw_capabilities"),
        "ironclaw_dispatcher is now a compatibility shim over ironclaw_capabilities"
    );
    for forbidden in [
        "ironclaw_authorization",
        "ironclaw_wasm",
        "ironclaw_scripts",
        "ironclaw_mcp",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "ironclaw_dispatcher must not regain direct runtime/workflow dependencies on {forbidden}"
        );
    }
    assert!(
        lib.contains("pub use ironclaw_capabilities::{"),
        "ironclaw_dispatcher should stay a re-export shim with no local dispatcher implementation"
    );
}
