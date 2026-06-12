use std::path::PathBuf;

fn worker_dockerfile() -> String {
    let repo_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repo root should be discoverable");
    let path = repo_root.join("Dockerfile.worker");
    std::fs::read_to_string(path).expect("Dockerfile.worker should be readable")
}

#[test]
fn worker_image_disables_ansi_colored_output() {
    let dockerfile = worker_dockerfile();

    assert!(
        dockerfile.contains("NO_COLOR=1"),
        "worker image must request colorless output so journald keeps MESSAGE as a string",
    );
    assert!(
        dockerfile.contains("RUST_LOG_STYLE=never"),
        "worker image must disable tracing-subscriber ANSI escapes",
    );
    assert!(
        dockerfile.contains("FORCE_COLOR=0"),
        "worker image must disable forced color output from JS and CLI tools",
    );
}
