#[test]
fn capabilities_crate_does_not_depend_on_concrete_dispatcher() {
    let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("failed to read {manifest_path:?}: {error}"));

    let dependencies = manifest
        .split_once("[dependencies]")
        .and_then(|(_, rest)| rest.split_once("[dev-dependencies]").map(|(deps, _)| deps))
        .expect("Cargo.toml must contain [dependencies] before [dev-dependencies]");

    assert!(
        !dependencies.contains("ironclaw_dispatcher"),
        "ironclaw_capabilities production code must depend on the neutral host-api dispatch port, not the concrete dispatcher crate"
    );
}
