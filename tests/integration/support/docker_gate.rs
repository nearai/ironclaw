//! Docker availability gate for the full-turn sandbox integration test.

use std::process::Command;

fn required() -> bool {
    std::env::var("IRONCLAW_REQUIRE_DOCKER_TESTS").as_deref() == Ok("1")
}

pub fn docker_available() -> bool {
    let available = Command::new("docker")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || !required(),
        "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but no Docker daemon is reachable"
    );
    available
}

pub fn docker_image_available(image: &str) -> bool {
    let available = Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        available || !required(),
        "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but sandbox image {image:?} is unavailable"
    );
    available
}

pub fn configured_sandbox_image() -> String {
    std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
        .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
        .unwrap_or_else(|_| "ironclaw-worker:latest".to_string())
}
