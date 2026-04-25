use std::path::Path;

#[test]
fn host_runtime_filesystem_example_uses_scoped_output_refs() {
    let example_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("reborn_host_runtime_filesystem.rs");
    let source = std::fs::read_to_string(&example_path)
        .unwrap_or_else(|error| panic!("failed to read {example_path:?}: {error}"));

    assert!(source.contains("HostRuntimeServices"));
    assert!(source.contains("ProcessServices::filesystem"));
    assert!(source.contains("mount_local"));
    assert!(source.contains("/engine"));
    assert!(source.contains("output_ref"));
    assert!(source.contains("process-results"));
    assert!(source.contains("process-outputs"));
    assert!(source.contains("await_result"));
    assert!(source.contains("output"));
    assert!(!source.contains("DockerScriptBackend"));
    assert!(!source.contains("docker run"));
}
