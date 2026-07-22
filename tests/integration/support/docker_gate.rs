//! Docker daemon / sandbox-image availability gate for real-Docker
//! integration-tier tests under `tests/integration/`.
//!
//! Real-Docker tests run only where a daemon (and the locally-built
//! `ironclaw-worker` image) is reachable — CI/hosted Docker runners, not a
//! typical dev machine. Callers MUST skip with a visible "SKIP: ..." line on
//! `eprintln!` when either check fails, never a silent pass — a quietly
//! vanishing assertion is indistinguishable from a real green run.
//!
//! Deliberately duplicated (not `#[path]`-shared) from
//! `crates/ironclaw_host_runtime/tests/support/docker_gate.rs`: that file
//! lives inside a different crate's private test tree, and reaching into it
//! via a cross-crate relative `#[path]` would couple this integration suite
//! to `ironclaw_host_runtime`'s test directory layout. This copy is the
//! `tests/integration/`-local instance of the same tiny gate.

use std::process::Command;

/// True iff the `docker` CLI can reach a live daemon (`docker version`
/// succeeds only against a running daemon).
pub fn docker_available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// True iff `image` is present in the local Docker image store (i.e. it was
/// built, not just referenced). The Reborn sandbox worker image
/// (`ironclaw-worker:latest` by default, `IRONCLAW_REBORN_SANDBOX_IMAGE` /
/// `IRONCLAW_SANDBOX_IMAGE` override) is never pulled automatically — a
/// daemon can be reachable with the image still missing.
pub fn docker_image_available(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Resolve the sandbox worker image name the same way
/// `RebornSandboxConfig::new` does, so the gate checks the image the test
/// will actually launch.
pub fn configured_sandbox_image() -> String {
    std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
        .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
        .unwrap_or_else(|_| "ironclaw-worker:latest".to_string())
}
