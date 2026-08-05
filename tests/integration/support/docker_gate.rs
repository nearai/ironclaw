//! Docker availability gate for the full-turn sandbox integration test.

use std::time::Duration;

use tokio::process::Command;

const DOCKER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

fn required() -> bool {
    std::env::var("IRONCLAW_REQUIRE_DOCKER_TESTS").as_deref() == Ok("1")
}

pub async fn docker_available() -> bool {
    let available = docker_command_succeeds(["version"]).await;
    assert!(
        available || !required(),
        "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but no Docker daemon is reachable"
    );
    available
}

pub async fn docker_image_available(image: &str) -> bool {
    let available = docker_command_succeeds(["image", "inspect", image]).await;
    assert!(
        available || !required(),
        "IRONCLAW_REQUIRE_DOCKER_TESTS=1 but sandbox image {image:?} is unavailable"
    );
    available
}

async fn docker_command_succeeds<const N: usize>(args: [&str; N]) -> bool {
    let mut command = Command::new("docker");
    command.args(args).kill_on_drop(true);
    matches!(
        tokio::time::timeout(DOCKER_PROBE_TIMEOUT, command.output()).await,
        Ok(Ok(output)) if output.status.success()
    )
}

pub fn configured_sandbox_image() -> String {
    std::env::var("IRONCLAW_REBORN_SANDBOX_IMAGE")
        .or_else(|_| std::env::var("IRONCLAW_SANDBOX_IMAGE"))
        .unwrap_or_else(|_| "ironclaw-worker:latest".to_string())
}
