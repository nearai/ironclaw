//! Real-Docker security boundary check for the sandbox worker image.
//!
//! Both preconditions — daemon reachable, image built — go through
//! `tests/support/docker_gate.rs` rather than being open-coded here, so this
//! test participates in the crate's single fail-closed switch: under
//! `IRONCLAW_REQUIRE_DOCKER_TESTS=1` a missing daemon or a missing image is a
//! hard failure instead of a skip. **Nothing in the repository sets that
//! variable yet** (issue #7081), so with it unset the observable behavior is
//! unchanged from the open-coded version: skip, with a visible `SKIP:` line as
//! the gate's module doc requires.

use std::process::Command;

use ironclaw_sandbox::DEFAULT_PROCESS_SANDBOX_IMAGE;

#[path = "support/docker_gate.rs"]
mod docker_gate;

#[test]
fn docker_image_enforces_basic_security_boundary_when_available() {
    if !docker_gate::docker_available() {
        eprintln!("SKIP: Docker security boundary test — no Docker daemon is reachable");
        return;
    }
    if !docker_gate::docker_image_available(DEFAULT_PROCESS_SANDBOX_IMAGE) {
        eprintln!(
            "SKIP: Docker security boundary test — {DEFAULT_PROCESS_SANDBOX_IMAGE} is not built"
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
