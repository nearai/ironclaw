//! Real-Docker tests for [`ironclaw_host_runtime::SandboxReaper`]'s two-stage
//! per-user lifecycle.
//!
//! `sandbox_process/reaper.rs`'s no-Docker unit tests already pin the pure
//! stage-decision logic (`decide_reap_action` on a fake clock: idle→stop,
//! retention→remove, forced-recycle by age, and "never reap on uncertainty").
//! These tests prove the Docker-facing half: that a reaper pointed at a real
//! daemon actually stops a container it decides to stop, actually removes one
//! it decides to remove, and actually leaves alone one it decides to keep.
//!
//! The in-memory [`SandboxActivityRegistry`]'s `touch` is crate-private, so an
//! integration test can only hand the reaper an *empty* registry (every
//! container reads back `idle == None`). The three cases below therefore drive
//! the reaper through all three [`ReapAction`]s using the wall-clock
//! `ironclaw.created_at` label — a container whose label places its age past
//! `forced_recycle_after` is stopped (if running) or removed (if stopped),
//! while a young running container survives. The idle-stop-by-activity path is
//! covered by the crate's fake-clock unit tests, which can touch the registry.
//!
//! Requires a reachable Docker daemon AND a locally-built `ironclaw-worker`
//! image, same gate as `sandbox_cross_tenant_escape.rs`. Neither is available
//! on this development machine — the tests are authored to run for real in
//! CI/hosted Docker lanes and skip cleanly (a visible `SKIP: ...` line, never
//! a silent pass) everywhere else.

#[path = "support/docker_gate.rs"]
mod docker_gate;

use std::{collections::HashMap, sync::Arc, time::Duration};

use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, RemoveContainerOptions,
        StartContainerOptions,
    },
    models::HostConfig,
};
use chrono::Utc;
use ironclaw_host_api::{TenantId, UserId};
use ironclaw_host_runtime::{SandboxActivityRegistry, SandboxReaper, SandboxReaperConfig};

// The persistent-container identity labels the production launch config
// attaches (see `sandbox_process/registry.rs`). Written as literals here
// because those helper fns are `pub(crate)` and unreachable from an
// integration test — the reaper's real listing filter keys on
// `ironclaw.created_at`, and it rebuilds the per-user key from
// `ironclaw.tenant`/`ironclaw.user`.
const LABEL_TENANT: &str = "ironclaw.tenant";
const LABEL_USER: &str = "ironclaw.user";
const LABEL_CREATED_AT: &str = "ironclaw.created_at";

fn user_labels(created_at: chrono::DateTime<Utc>) -> HashMap<String, String> {
    let tenant = TenantId::new("reaper-docker-tenant").unwrap();
    let user = UserId::new("reaper-docker-user").unwrap();
    HashMap::from([
        (LABEL_TENANT.to_string(), tenant.as_str().to_string()),
        (LABEL_USER.to_string(), user.as_str().to_string()),
        (LABEL_CREATED_AT.to_string(), created_at.to_rfc3339()),
    ])
}

/// A test config whose `forced_recycle_after` is short enough that a
/// container carrying a `created_at` label a couple of minutes in the past is
/// past the recycle age, without any real waiting. `idle_stop_after` and
/// `remove_stopped_after` are left large so the *only* trigger these tests
/// exercise is age-based forced recycle (the one axis an integration test can
/// drive without touching the crate-private activity registry).
fn forced_recycle_config() -> SandboxReaperConfig {
    SandboxReaperConfig {
        scan_interval: Duration::from_secs(300),
        idle_stop_after: Duration::from_secs(900),
        remove_stopped_after: Duration::from_secs(7 * 24 * 3600),
        forced_recycle_after: Duration::from_secs(60),
        label_prefix: "ironclaw".to_string(),
    }
}

/// Starts a long-running labeled container (bypassing the command transport,
/// which blocks until its command exits — these tests need a container that is
/// still *running* when the reaper scans it).
async fn start_running_container(
    docker: &Docker,
    image: &str,
    labels: HashMap<String, String>,
) -> String {
    let container_id = create_labeled_container(docker, image, labels, "sleep 300").await;
    docker
        .start_container(&container_id, None::<StartContainerOptions<String>>)
        .await
        .expect("container start should succeed against a reachable daemon");
    container_id
}

/// Creates, starts, and waits for a labeled container to exit, leaving it in
/// the `exited` state with a real `finished_at` — the shape the reaper's
/// remove branch requires (a stopped container with a known stop time).
async fn start_then_exit_container(
    docker: &Docker,
    image: &str,
    labels: HashMap<String, String>,
) -> String {
    let container_id = create_labeled_container(docker, image, labels, "exit 0").await;
    docker
        .start_container(&container_id, None::<StartContainerOptions<String>>)
        .await
        .expect("container start should succeed against a reachable daemon");
    for _ in 0..50 {
        if !container_running(docker, &container_id).await {
            return container_id;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("container did not reach the exited state within the poll window");
}

async fn create_labeled_container(
    docker: &Docker,
    image: &str,
    labels: HashMap<String, String>,
    command: &str,
) -> String {
    let name = format!("ironclaw-reaper-docker-test-{}", uuid::Uuid::new_v4());
    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            command.to_string(),
        ]),
        labels: Some(labels),
        host_config: Some(HostConfig {
            auto_remove: Some(false),
            network_mode: Some("none".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    docker
        .create_container(Some(CreateContainerOptions { name, platform: None }), config)
        .await
        .expect("container create should succeed against a reachable daemon")
        .id
}

async fn container_exists(docker: &Docker, container_id: &str) -> bool {
    docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
        .is_ok()
}

async fn container_running(docker: &Docker, container_id: &str) -> bool {
    docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
        .ok()
        .and_then(|c| c.state.and_then(|s| s.running))
        .unwrap_or(false)
}

async fn best_effort_remove(docker: &Docker, container_id: &str) {
    let _ = docker
        .remove_container(
            container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await;
}

#[tokio::test]
async fn young_running_container_survives_scan() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — young_running_container_survives_scan requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — young_running_container_survives_scan requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let docker = Docker::connect_with_local_defaults().unwrap();
    // Freshly created: not past forced-recycle age, and the empty activity
    // registry reports `idle == None` — "never reap on uncertainty" keeps it.
    let labels = user_labels(Utc::now());
    let container_id = start_running_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(SandboxActivityRegistry::new()),
        forced_recycle_config(),
    );
    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed");

    let survived = container_exists(&docker, &container_id).await;
    best_effort_remove(&docker, &container_id).await;
    assert!(
        survived,
        "a young running container with no idle record must survive the scan"
    );
}

#[tokio::test]
async fn running_container_past_forced_recycle_age_is_stopped_not_removed() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — running_container_past_forced_recycle_age_is_stopped_not_removed requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — running_container_past_forced_recycle_age_is_stopped_not_removed requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let docker = Docker::connect_with_local_defaults().unwrap();
    // Age (via the created_at label) is past forced_recycle_after while the
    // container is still running: forced recycle stops it first (stop, not
    // remove — the user's workspace bind mount is preserved for restart).
    let labels = user_labels(Utc::now() - chrono::Duration::seconds(120));
    let container_id = start_running_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(SandboxActivityRegistry::new()),
        forced_recycle_config(),
    );
    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed");

    let still_exists = container_exists(&docker, &container_id).await;
    let still_running = container_running(&docker, &container_id).await;
    best_effort_remove(&docker, &container_id).await;
    assert!(
        still_exists,
        "a running container past forced-recycle age must be stopped, not removed"
    );
    assert!(
        !still_running,
        "a running container past forced-recycle age must be stopped"
    );
}

#[tokio::test]
async fn stopped_container_past_forced_recycle_age_is_removed() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — stopped_container_past_forced_recycle_age_is_removed requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — stopped_container_past_forced_recycle_age_is_removed requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let docker = Docker::connect_with_local_defaults().unwrap();
    // Already stopped (known finished_at) and past forced-recycle age: removed
    // outright even though it is still within the normal retention window.
    let labels = user_labels(Utc::now() - chrono::Duration::seconds(120));
    let container_id = start_then_exit_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(SandboxActivityRegistry::new()),
        forced_recycle_config(),
    );
    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed");

    let survived = container_exists(&docker, &container_id).await;
    if survived {
        best_effort_remove(&docker, &container_id).await;
    }
    assert!(
        !survived,
        "a stopped container past forced-recycle age must be removed"
    );
}
