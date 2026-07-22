//! Real-Docker tests for [`ironclaw_host_runtime::SandboxReaper`].
//!
//! `sandbox_process.rs`'s no-Docker unit tests (in `sandbox_process/reaper.rs`)
//! already pin the liveness-decision logic (headline: a transient run-state
//! query error is never treated as "the run is gone"). These tests prove the
//! Docker-facing half: that a reaper pointed at a real daemon actually removes
//! a container it has decided is orphaned, and actually leaves alone a
//! container it has decided is alive (or uncertain).
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
    container::{Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions},
    models::HostConfig,
};
use chrono::Utc;
use ironclaw_host_api::{
    AgentId, ApprovalRequest, InvocationId, ProjectId, ResourceScope, TenantId, UserId,
};
use ironclaw_host_runtime::{SandboxReaper, SandboxReaperConfig};
use ironclaw_run_state::{RunRecord, RunStart, RunStateError, RunStateStore};

const LABEL_INVOCATION_ID: &str = "ironclaw.invocation_id";
const LABEL_RESOURCE_SCOPE: &str = "ironclaw.resource_scope";
const LABEL_CREATED_AT: &str = "ironclaw.created_at";

fn test_scope() -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("reaper-docker-tenant").unwrap(),
        user_id: UserId::new("reaper-docker-user").unwrap(),
        agent_id: Some(AgentId::new("agent").unwrap()),
        project_id: Some(ProjectId::new("project").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

/// Builds the same `ironclaw.*` labels `container_launch_config` attaches in
/// production, so these tests exercise the reaper's real listing filter
/// (`ironclaw.invocation_id`) and label parsing, not a test-only shortcut.
/// `created_at` is caller-supplied so tests can place a container on either
/// side of `orphan_threshold` without sleeping.
fn ironclaw_labels(
    scope: &ResourceScope,
    created_at: chrono::DateTime<Utc>,
) -> HashMap<String, String> {
    HashMap::from([
        (
            LABEL_INVOCATION_ID.to_string(),
            scope.invocation_id.to_string(),
        ),
        (
            LABEL_RESOURCE_SCOPE.to_string(),
            serde_json::to_string(scope).unwrap(),
        ),
        (LABEL_CREATED_AT.to_string(), created_at.to_rfc3339()),
    ])
}

/// Starts a long-running, labeled container directly (bypassing
/// `SandboxCommandTransport::run_command`, which blocks until the command
/// exits — these tests need a container that is still *running* when the
/// reaper scans it, simulating a host crash mid-command).
async fn start_labeled_container(
    docker: &Docker,
    image: &str,
    labels: HashMap<String, String>,
) -> String {
    let name = format!("ironclaw-reaper-docker-test-{}", uuid::Uuid::new_v4());
    let config = Config {
        image: Some(image.to_string()),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            "sleep 300".to_string(),
        ]),
        labels: Some(labels),
        host_config: Some(HostConfig {
            auto_remove: Some(false),
            network_mode: Some("none".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let created = docker
        .create_container(
            Some(CreateContainerOptions {
                name,
                platform: None,
            }),
            config,
        )
        .await
        .expect("container create should succeed against a reachable daemon");
    docker
        .start_container(&created.id, None::<StartContainerOptions<String>>)
        .await
        .expect("container start should succeed against a reachable daemon");
    created.id
}

async fn container_exists(docker: &Docker, container_id: &str) -> bool {
    docker.inspect_container(container_id, None).await.is_ok()
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

/// Always reports "no record" — mirrors an invocation whose run-state has
/// genuinely aged out or was never recorded (definitive orphan).
struct AlwaysAbsentRunStateStore;

/// Always reports a transient backend failure — mirrors a run-state store
/// that is momentarily unreachable, not evidence the invocation is gone.
struct AlwaysErrorRunStateStore;

// The reaper only ever calls `get`; the writer methods must exist to satisfy
// the trait but are never invoked. They're written inline (not via a
// declarative macro) because `#[async_trait]` sees a macro *invocation*, not
// the expanded `async fn`s, and cannot rewrite them — which produces E0195.
#[async_trait::async_trait]
impl RunStateStore for AlwaysAbsentRunStateStore {
    async fn start(&self, _start: RunStart) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls start()")
    }
    async fn block_approval(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls block_approval()")
    }
    async fn block_auth(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls block_auth()")
    }
    async fn complete(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls complete()")
    }
    async fn fail(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls fail()")
    }
    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        unreachable!("AlwaysAbsentRunStateStore never calls records_for_scope()")
    }
    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl RunStateStore for AlwaysErrorRunStateStore {
    async fn start(&self, _start: RunStart) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls start()")
    }
    async fn block_approval(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _approval: ApprovalRequest,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls block_approval()")
    }
    async fn block_auth(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls block_auth()")
    }
    async fn complete(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls complete()")
    }
    async fn fail(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
        _error_kind: String,
    ) -> Result<RunRecord, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls fail()")
    }
    async fn records_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<RunRecord>, RunStateError> {
        unreachable!("AlwaysErrorRunStateStore never calls records_for_scope()")
    }
    async fn get(
        &self,
        _scope: &ResourceScope,
        _invocation_id: InvocationId,
    ) -> Result<Option<RunRecord>, RunStateError> {
        Err(RunStateError::Backend(
            "simulated transient run-state backend failure".to_string(),
        ))
    }
}

#[tokio::test]
async fn kill_mid_command_orphan_past_threshold_is_reaped() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — kill_mid_command_orphan_past_threshold_is_reaped requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — kill_mid_command_orphan_past_threshold_is_reaped requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let docker = Docker::connect_with_local_defaults().unwrap();
    let scope = test_scope();
    // Simulates "host crashed mid-command": container is still running, its
    // created_at label is already older than orphan_threshold, and no
    // run-state record exists for its invocation (definitive orphan).
    let created_at = Utc::now() - chrono::Duration::seconds(120);
    let labels = ironclaw_labels(&scope, created_at);
    let container_id = start_labeled_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(AlwaysAbsentRunStateStore),
        SandboxReaperConfig {
            scan_interval: Duration::from_secs(300),
            orphan_threshold: Duration::from_secs(60),
            label_prefix: "ironclaw".to_string(),
        },
    );

    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed");

    assert!(
        !container_exists(&docker, &container_id).await,
        "orphaned container past the threshold must be reaped"
    );
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
    let scope = test_scope();
    // Freshly created: even though no run-state record exists either, the
    // container has not yet crossed orphan_threshold and must survive.
    let labels = ironclaw_labels(&scope, Utc::now());
    let container_id = start_labeled_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(AlwaysAbsentRunStateStore),
        SandboxReaperConfig {
            scan_interval: Duration::from_secs(300),
            orphan_threshold: Duration::from_secs(600),
            label_prefix: "ironclaw".to_string(),
        },
    );

    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed");

    let survived = container_exists(&docker, &container_id).await;
    best_effort_remove(&docker, &container_id).await;
    assert!(survived, "young container under the threshold must survive");
}

#[tokio::test]
async fn transient_liveness_error_container_survives_scan() {
    if !docker_gate::docker_available() {
        eprintln!(
            "SKIP: no docker daemon reachable — transient_liveness_error_container_survives_scan requires a real Docker daemon (CI/hosted Docker lane only)"
        );
        return;
    }
    let image = docker_gate::configured_sandbox_image();
    if !docker_gate::docker_image_available(&image) {
        eprintln!(
            "SKIP: sandbox worker image {image:?} is not built locally — transient_liveness_error_container_survives_scan requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
        );
        return;
    }

    let docker = Docker::connect_with_local_defaults().unwrap();
    let scope = test_scope();
    // Old enough to be reap-eligible by age alone, but the run-state store
    // fails the liveness query — the headline behavior (never reap on
    // uncertainty) must keep this container alive.
    let created_at = Utc::now() - chrono::Duration::seconds(120);
    let labels = ironclaw_labels(&scope, created_at);
    let container_id = start_labeled_container(&docker, &image, labels).await;

    let reaper = SandboxReaper::new(
        docker.clone(),
        Arc::new(AlwaysErrorRunStateStore),
        SandboxReaperConfig {
            scan_interval: Duration::from_secs(300),
            orphan_threshold: Duration::from_secs(60),
            label_prefix: "ironclaw".to_string(),
        },
    );

    reaper
        .scan_and_reap()
        .await
        .expect("scan against a reachable daemon should succeed even when run-state queries fail");

    let survived = container_exists(&docker, &container_id).await;
    best_effort_remove(&docker, &container_id).await;
    assert!(
        survived,
        "a transient run-state query error must never cause a reap"
    );
}
