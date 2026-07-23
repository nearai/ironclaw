//! Docker daemon / sandbox-image availability gate for real-Docker crate-tier
//! tests in this crate.
//!
//! Real-Docker tests run only where a daemon (and the locally-built
//! `ironclaw-worker` image) is reachable — CI/hosted Docker runners, not this
//! development machine. Callers MUST skip with a visible "SKIP: ..." line on
//! `eprintln!` when either check fails, never a silent pass — a quietly
//! vanishing assertion is indistinguishable from a real green run.

use std::process::Command;

/// True iff the `docker` CLI can reach a live daemon (`docker version`
/// succeeds only against a running daemon). Mirrors the gate
/// `ironclaw_process_sandbox/tests/docker_security.rs` already uses.
pub(crate) fn docker_available() -> bool {
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
pub(crate) fn docker_image_available(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Resolve the sandbox worker image name the same way
/// `RebornSandboxConfig::new` does, so the gate checks the image the test
/// will actually launch.
pub(crate) fn configured_sandbox_image() -> String {
    std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
        .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
        .unwrap_or_else(|_| "ironclaw-worker:latest".to_string())
}
