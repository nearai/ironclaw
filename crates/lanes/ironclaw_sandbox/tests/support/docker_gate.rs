//! Docker daemon availability gate for real-Docker crate-tier tests in this
//! crate.
//!
//! This module only gates on daemon reachability ([`docker_available`]) —
//! whether an image the test needs is present is each caller's own concern.
//! `attribution.rs`'s real-Docker test pulls `busybox:1.36` itself before
//! using it; a caller that instead needs the locally-built `ironclaw-worker`
//! image (e.g. `exec_transport`, which declares this same file via `#[path]`
//! — see `attribution.rs`'s module-level comment on its own declaration) is
//! responsible for ensuring that image is present. Callers MUST skip with a
//! visible "SKIP: ..." line on `eprintln!` when either the daemon or their
//! own image precondition fails, never a silent pass — a quietly vanishing
//! assertion is indistinguishable from a real green run.

use std::process::Command;

/// True iff `IRONCLAW_REQUIRE_DOCKER_TESTS=1` is set. CI sets this so a
/// missing daemon/image is a hard test failure instead of a silent skip (the
/// exact gap that let sandbox security bugs ship unnoticed — docker-gated
/// tests only ran on one developer's laptop). Unset locally, so dev machines
/// keep today's skip-and-pass behavior.
pub(crate) fn docker_tests_required() -> bool {
    std::env::var("IRONCLAW_REQUIRE_DOCKER_TESTS").as_deref() == Ok("1")
}

/// True iff the `docker` CLI can reach a live daemon (`docker version`
/// succeeds only against a running daemon). This is the daemon gate
/// `ironclaw_sandbox/tests/docker_security.rs` uses; it open-coded an
/// equivalent check until 2026-08-03, which left it outside the fail-closed
/// switch below.
///
/// When `IRONCLAW_REQUIRE_DOCKER_TESTS=1` and no daemon is reachable, this
/// panics rather than returning `false` — callers gate on this function
/// returning `false` to print `SKIP: ...` and return early, which must not
/// happen in CI.
pub(crate) fn docker_available() -> bool {
    let available = Command::new("docker")
        .arg("version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !available && docker_tests_required() {
        panic!(
            "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but no Docker daemon is reachable \
             (`docker version` failed) — docker-gated tests must not silently \
             skip in CI"
        );
    }
    available
}

/// True iff `image` is present in the local Docker image store (i.e. it was
/// built, not just referenced). The Reborn sandbox worker image
/// (`ironclaw-worker:latest` by default, `IRONCLAW_REBORN_SANDBOX_IMAGE` /
/// `IRONCLAW_SANDBOX_IMAGE` override) is never pulled automatically — a
/// daemon can be reachable with the image still missing.
///
/// Same "fail instead of skip" behavior as [`docker_available`] under
/// `IRONCLAW_REQUIRE_DOCKER_TESTS=1`.
// Loaded into several test modules whose individual consumers use different
// subsets of this shared gate.
#[allow(dead_code)]
pub(crate) fn docker_image_available(image: &str) -> bool {
    let available = Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !available && docker_tests_required() {
        panic!(
            "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but sandbox image {image:?} is not \
             present locally — docker-gated tests must not silently skip in CI"
        );
    }
    available
}

/// Resolve the sandbox worker image name the same way
/// `RebornSandboxConfig::new` does, so the gate checks the image the test
/// will actually launch.
// Some consumers use a fixed image while the per-user lifecycle and
// attribution proofs use the configured worker image.
#[allow(dead_code)]
pub(crate) fn configured_sandbox_image() -> String {
    std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
        .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
        .unwrap_or_else(|_| "ironclaw-worker:latest".to_string())
}
