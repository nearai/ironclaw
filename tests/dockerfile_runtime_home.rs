use std::path::PathBuf;

fn runtime_dockerfile() -> String {
    let repo_root = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .expect("repo root should be discoverable");
    let path = repo_root.join("Dockerfile");
    std::fs::read_to_string(path).expect("Dockerfile should be readable")
}

#[test]
fn runtime_image_declares_and_prepares_ironclaw_home() {
    let dockerfile = runtime_dockerfile();

    assert!(
        dockerfile.contains("useradd -m -d /home/ironclaw -u 1000 ironclaw"),
        "runtime image must create the ironclaw user with the expected home directory",
    );
    assert!(
        dockerfile.contains("ENV HOME=/home/ironclaw"),
        "runtime image must set HOME to /home/ironclaw for ~/.ironclaw state",
    );
    assert!(
        dockerfile.contains("WORKDIR /home/ironclaw"),
        "runtime image must start in the ironclaw home directory",
    );
    assert!(
        dockerfile.contains("mkdir -p /home/ironclaw/.ironclaw"),
        "runtime image must pre-create ~/.ironclaw before dropping privileges",
    );
}
