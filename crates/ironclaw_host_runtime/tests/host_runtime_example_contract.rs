use std::path::Path;

#[test]
fn host_runtime_live_example_is_non_docker_and_uses_composition_root() {
    let example_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("reborn_host_runtime.rs");
    let source = std::fs::read_to_string(&example_path)
        .unwrap_or_else(|error| panic!("failed to read {example_path:?}: {error}"));

    assert!(source.contains("HostRuntimeServices"));
    assert!(source.contains("ProcessServices::in_memory"));
    assert!(source.contains("capability_host_for_runtime_dispatcher"));
    assert!(source.contains("spawn_json"));
    assert!(source.contains("await_result"));
    assert!(source.contains("output"));
    assert!(source.contains("InProcessEchoBackend"));
    assert!(!source.contains("DockerScriptBackend"));
    assert!(!source.contains("docker run"));
}
