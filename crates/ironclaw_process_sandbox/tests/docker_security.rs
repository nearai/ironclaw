use std::process::Command;

use ironclaw_process_sandbox::DEFAULT_PROCESS_SANDBOX_IMAGE;

#[test]
fn docker_image_enforces_basic_security_boundary_when_available() {
    if Command::new("docker").arg("version").output().is_err() {
        eprintln!("skipping Docker security boundary test: docker CLI is unavailable");
        return;
    }
    let Ok(inspect) = Command::new("docker")
        .args(["image", "inspect", DEFAULT_PROCESS_SANDBOX_IMAGE])
        .output()
    else {
        eprintln!("skipping Docker security boundary test: docker image inspect failed");
        return;
    };
    if !inspect.status.success() {
        eprintln!(
            "skipping Docker security boundary test: {DEFAULT_PROCESS_SANDBOX_IMAGE} is not built"
        );
        return;
    }

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--network",
            "none",
            "--security-opt",
            "no-new-privileges",
            "--cap-drop",
            "ALL",
            "--cap-add",
            "SETPCAP",
            "--cap-add",
            "SETUID",
            "--cap-add",
            "SETGID",
            "--memory",
            "128m",
            "--memory-swap",
            "128m",
            "--pids-limit",
            "64",
            DEFAULT_PROCESS_SANDBOX_IMAGE,
            "sh",
            "-c",
            "test \"$(id -u)\" != 0 && test -z \"$(ip route 2>/dev/null | awk '/default/ {print $0}')\"",
        ])
        .output()
        .expect("docker run should start when image is available");

    assert!(
        output.status.success(),
        "sandbox image should run as non-root without default network route; stdout={}; stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
