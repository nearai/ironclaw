//! Real-Docker security boundary check for the sandbox worker image.
//!
//! Both preconditions — daemon reachable, configured Reborn worker image built
//! — go through `tests/support/docker_gate.rs`. Under
//! `IRONCLAW_REQUIRE_DOCKER_TESTS=1`, a missing daemon or image is a hard
//! failure instead of a skip. Without that switch, local runs keep the same
//! visible `SKIP:` behavior.

use std::process::Command;

#[path = "support/docker_gate.rs"]
mod docker_gate;

#[test]
fn docker_image_enforces_basic_security_boundary_when_available() {
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_available() {
        eprintln!("SKIP: Docker security boundary test — no Docker daemon is reachable");
        return;
    }
    if !docker_gate::docker_image_available(&image) {
        eprintln!("SKIP: Docker security boundary test — {image} is not built");
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
            &image,
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
