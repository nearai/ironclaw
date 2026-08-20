use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
        RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
    },
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::ContainerInspectResponse,
};
use futures_util::StreamExt;
use ironclaw_host_api::{
    ids::{TenantId, UserId},
    process::{CommandExecutionOutput, RuntimeProcessError},
};

use super::{
    ContainerWorkdir, RebornScopedSandboxCommandTransport, append_with_limit,
    registry::{
        ExistingContainerDecision, SandboxActivityRegistry, existing_container_decision,
        label_tenant, label_user,
    },
    user_key::RebornSandboxUserKey,
};

pub(super) const LABEL_PREFIX: &str = super::registry::USER_CONTAINER_LABEL_PREFIX;
const EXEC_HELPER: &str = "/usr/local/bin/ironclaw-exec";
const HOST_TIMEOUT_GRACE: Duration = Duration::from_secs(5);
const CONTAINER_STOP_TIMEOUT_SECS: i64 = 10;
/// Mirrors the worker helper's hard maximum.
const MAX_EXEC_TIMEOUT_SECS: u64 = 86_400;

pub(super) struct UserContainerLaunch {
    pub(super) config: Config<String>,
    pub(super) labels: HashMap<String, String>,
}

/// Transport-local idle Docker cleanup.
///
/// This never owns durable run state; `ironclaw_processes` remains the process
/// lifecycle authority. Reconciliation after a host restart is demand-driven
/// by the next command, not by this in-memory task.
pub(super) struct UserContainerSweeper {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl UserContainerSweeper {
    pub(super) fn spawn(
        docker: Docker,
        registry: Arc<SandboxActivityRegistry>,
        idle_timeout: Duration,
    ) -> Arc<Self> {
        let (shutdown, mut receiver) = tokio::sync::watch::channel(false);
        let interval = idle_timeout
            .checked_div(2)
            .unwrap_or(Duration::from_millis(50))
            .clamp(Duration::from_millis(50), Duration::from_secs(60));
        let task = tokio::spawn(async move {
            reconcile_labeled_user_containers(&docker, &registry).await;
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = ticker.tick() => sweep_idle_user_containers(&docker, &registry, idle_timeout).await,
                    changed = receiver.changed() => {
                        if changed.is_err() || *receiver.borrow() {
                            break;
                        }
                    }
                }
            }
        });
        Arc::new(Self {
            shutdown,
            task: std::sync::Mutex::new(Some(task)),
        })
    }

    fn take_task(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.task
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }

    pub(super) async fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.take_task() {
            let _ = task.await;
        }
    }
}

impl Drop for UserContainerSweeper {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(task) = self.take_task() {
            task.abort();
        }
    }
}

#[cfg(test)]
pub(super) mod test_support {
    use super::*;

    pub(in crate::sandbox_process) fn disabled_sweeper() -> Arc<UserContainerSweeper> {
        let (shutdown, _receiver) = tokio::sync::watch::channel(false);
        Arc::new(UserContainerSweeper {
            shutdown,
            task: std::sync::Mutex::new(None),
        })
    }
}

/// Ensures the stable container is ready while its user's lifecycle gate is held.
///
/// The caller must retain that gate until the command exec and any recycle finish.
pub(super) async fn ensure_user_container(
    transport: &RebornScopedSandboxCommandTransport,
    key: &RebornSandboxUserKey,
    launch: UserContainerLaunch,
) -> Result<String, RuntimeProcessError> {
    let name = key.container_name();
    transport
        .activity
        .set_expected_labels(key, launch.labels.clone());

    if transport.activity.recycle_required(key) {
        remove_user_container(&transport.docker, &name).await?;
        transport.activity.clear_recycle_required(key);
    }

    for attempt in 0..2 {
        if let Some(inspected) = inspect_user_container(&transport.docker, &name).await? {
            match user_container_adoption_decision(&inspected, &launch.labels) {
                ExistingContainerDecision::ReuseRunning => return Ok(name),
                ExistingContainerDecision::StartStopped => {
                    start_user_container(&transport.docker, &name).await?;
                    return Ok(name);
                }
                ExistingContainerDecision::Recreate => {
                    remove_user_container(&transport.docker, &name).await?;
                }
            }
        }

        match transport
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: name.clone(),
                    platform: None,
                }),
                launch.config.clone(),
            )
            .await
        {
            Ok(_) => {
                start_user_container(&transport.docker, &name).await?;
                return Ok(name);
            }
            Err(error) if docker_status(&error) == Some(409) && attempt == 0 => continue,
            Err(error) => {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox container create failed: {error}"
                )));
            }
        }
    }

    Err(RuntimeProcessError::ExecutionFailed(
        "sandbox container name collision did not converge".to_string(),
    ))
}

pub(super) fn exec_helper_timeout_secs(timeout: Duration) -> Result<u64, RuntimeProcessError> {
    let seconds = timeout
        .as_secs()
        .saturating_add(u64::from(timeout.subsec_nanos() > 0))
        .max(1);
    if seconds > MAX_EXEC_TIMEOUT_SECS {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox command timeout must not exceed {MAX_EXEC_TIMEOUT_SECS} seconds"
        )));
    }
    Ok(seconds)
}

/// Executes a command while its user's lifecycle gate is held.
///
/// The caller must retain that gate through any error or timeout recycle.
pub(super) async fn execute_in_user_container(
    transport: &RebornScopedSandboxCommandTransport,
    key: &RebornSandboxUserKey,
    container_name: &str,
    command: String,
    workdir: ContainerWorkdir,
    timeout: Duration,
) -> Result<CommandExecutionOutput, RuntimeProcessError> {
    let started_at = Instant::now();
    let helper_timeout_secs = exec_helper_timeout_secs(timeout)?;
    let outcome_nonce = uuid::Uuid::new_v4().to_string();
    let created = transport
        .docker
        .create_exec(
            container_name,
            CreateExecOptions {
                attach_stdin: Some(false),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(false),
                cmd: Some(vec![
                    EXEC_HELPER.to_string(),
                    helper_timeout_secs.to_string(),
                    outcome_nonce.clone(),
                    command,
                ]),
                privileged: Some(false),
                working_dir: Some(workdir.into_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox exec create failed: {error}"))
        })?;

    let exec_id = created.id;
    let run = async {
        let started = transport
            .docker
            .start_exec(
                &exec_id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: Some(
                        transport.config.max_output_bytes.clamp(8 * 1024, 64 * 1024),
                    ),
                }),
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!("sandbox exec start failed: {error}"))
            })?;
        let StartExecResults::Attached { mut output, input } = started else {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox exec unexpectedly detached".to_string(),
            ));
        };
        drop(input);

        let mut captured = BoundedExecOutput::new(transport.config.max_output_bytes);
        let mut outcome_tail = String::new();
        while let Some(frame) = output.next().await {
            match frame {
                Ok(bollard::container::LogOutput::StdOut { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    captured.push_stdout(&text);
                    append_tail(&mut outcome_tail, &text, 512);
                }
                Ok(bollard::container::LogOutput::StdErr { message }) => {
                    captured.push_stderr(&String::from_utf8_lossy(&message));
                }
                Ok(_) => {}
                Err(error) => {
                    return Err(RuntimeProcessError::ExecutionFailed(format!(
                        "sandbox exec output failed: {error}"
                    )));
                }
            }
        }

        let inspected = transport
            .docker
            .inspect_exec(&exec_id)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox exec inspect failed: {error}"
                ))
            })?;
        let helper_exit = inspected.exit_code.ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "sandbox exec completed without an exit code".to_string(),
            )
        })?;
        if helper_exit != 0 {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "sandbox exec supervisor failed with exit code {helper_exit}"
            )));
        }
        let outcome = parse_exec_outcome_trailer(&outcome_tail, &outcome_nonce)?;
        match outcome {
            ExecOutcome::Timeout => Err(RuntimeProcessError::Timeout(timeout)),
            ExecOutcome::Exit(exit_code) => {
                strip_exec_outcome_trailer(&mut captured.stdout, &outcome_nonce);
                let output = captured.finish();
                Ok(CommandExecutionOutput {
                    output,
                    saved_output: None,
                    exit_code,
                    sandboxed: true,
                    duration: started_at.elapsed(),
                })
            }
        }
    };

    match tokio::time::timeout(timeout.saturating_add(HOST_TIMEOUT_GRACE), run).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error @ RuntimeProcessError::Timeout(_))) => Err(error),
        Ok(Err(error)) => {
            recycle_untrusted_user_container(transport, key, container_name).await;
            Err(error)
        }
        Err(_) => {
            recycle_untrusted_user_container(transport, key, container_name).await;
            Err(RuntimeProcessError::Timeout(timeout))
        }
    }
}

/// Recycles a container that cannot safely accept another command.
///
/// The caller must already hold this user's lifecycle gate for the whole call.
/// Acquiring it here would self-deadlock the serialized command path.
async fn recycle_untrusted_user_container(
    transport: &RebornScopedSandboxCommandTransport,
    key: &RebornSandboxUserKey,
    container_name: &str,
) {
    transport.activity.mark_recycle_required(key);
    match tokio::time::timeout(
        HOST_TIMEOUT_GRACE,
        remove_user_container(&transport.docker, container_name),
    )
    .await
    {
        Ok(Ok(())) => transport.activity.clear_recycle_required(key),
        Ok(Err(error)) => tracing::warn!(
            ?error,
            container_name,
            "untrusted sandbox user container could not be recycled"
        ),
        Err(_) => tracing::warn!(
            container_name,
            "untrusted sandbox user container recycle request exceeded its grace period"
        ),
    }
}

async fn inspect_user_container(
    docker: &Docker,
    name: &str,
) -> Result<Option<ContainerInspectResponse>, RuntimeProcessError> {
    match docker
        .inspect_container(name, None::<InspectContainerOptions>)
        .await
    {
        Ok(container) => Ok(Some(container)),
        Err(error) if docker_status(&error) == Some(404) => Ok(None),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container inspect failed: {error}"
        ))),
    }
}

async fn start_user_container(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    match docker
        .start_container(name, None::<StartContainerOptions<String>>)
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if docker_status(&error) == Some(304) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container start failed: {error}"
        ))),
    }
}

fn user_container_adoption_decision(
    inspected: &ContainerInspectResponse,
    expected_labels: &HashMap<String, String>,
) -> ExistingContainerDecision {
    let state = inspected.state.as_ref();
    let unsafe_state = state.is_none_or(|state| {
        state.running.is_none()
            || state.paused.unwrap_or(false)
            || state.restarting.unwrap_or(false)
            || state.dead.unwrap_or(false)
    });
    let resolved_image = inspected.image.as_deref();
    let expected_image = expected_labels
        .get(&super::registry::label_image(LABEL_PREFIX))
        .map(String::as_str);
    if unsafe_state || resolved_image != expected_image {
        return ExistingContainerDecision::Recreate;
    }
    let labels = inspected
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref());
    existing_container_decision(
        labels,
        state.and_then(|state| state.running).unwrap_or(false),
        expected_labels,
    )
}

/// Force-removes a user container.
///
/// The caller must hold that user's lifecycle gate for the whole operation.
async fn remove_user_container(docker: &Docker, name: &str) -> Result<(), RuntimeProcessError> {
    match docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if docker_status(&error) == Some(404) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container recycle failed: {error}"
        ))),
    }
}

fn docker_status(error: &bollard::errors::Error) -> Option<u16> {
    match error {
        bollard::errors::Error::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    }
}

async fn reconcile_labeled_user_containers(
    docker: &Docker,
    registry: &Arc<SandboxActivityRegistry>,
) {
    let tenant_label = label_tenant(LABEL_PREFIX);
    let user_label = label_user(LABEL_PREFIX);
    let filters = HashMap::from([(
        "label".to_string(),
        vec![tenant_label.clone(), user_label.clone()],
    )]);
    let containers = match docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await
    {
        Ok(containers) => containers,
        Err(error) => {
            tracing::warn!(?error, "sandbox user-container reconciliation failed");
            return;
        }
    };
    for container in containers {
        let Some(labels) = container.labels else {
            continue;
        };
        let (Some(tenant), Some(user)) = (labels.get(&tenant_label), labels.get(&user_label))
        else {
            continue;
        };
        let (Ok(tenant), Ok(user)) = (TenantId::new(tenant), UserId::new(user)) else {
            tracing::warn!(
                container_id = container.id.as_deref().unwrap_or("<unknown>"),
                "sandbox user-container reconciliation ignored invalid identity labels"
            );
            continue;
        };
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        if let Err(error) = registry.register_discovered_container(key, labels) {
            tracing::warn!(
                ?error,
                container_id = container.id.as_deref().unwrap_or("<unknown>"),
                "sandbox user-container reconciliation reached registry capacity"
            );
        }
    }
}

async fn sweep_idle_user_containers(
    docker: &Docker,
    registry: &Arc<SandboxActivityRegistry>,
    idle_timeout: Duration,
) {
    for (key, expected_labels) in registry.sweep_candidates(Instant::now(), idle_timeout) {
        let Some(gate) = registry.gate(&key) else {
            continue;
        };
        let Ok(_gate) = gate.try_lock() else {
            continue;
        };
        if !registry.sweep_eligible(&key, Instant::now(), idle_timeout) {
            continue;
        }
        let name = key.container_name();
        let inspected = match inspect_user_container(docker, &name).await {
            Ok(inspected) => inspected,
            Err(error) => {
                tracing::debug!(?error, container_name = name, "idle sandbox inspect failed");
                continue;
            }
        };
        let Some(inspected) = inspected else {
            registry.forget_if_inactive(&key);
            continue;
        };
        match user_container_adoption_decision(&inspected, &expected_labels) {
            ExistingContainerDecision::ReuseRunning => {
                if let Err(error) = docker
                    .stop_container(
                        &name,
                        Some(StopContainerOptions {
                            t: CONTAINER_STOP_TIMEOUT_SECS,
                        }),
                    )
                    .await
                {
                    tracing::debug!(?error, container_name = name, "idle sandbox stop failed");
                    continue;
                }
                registry.forget_if_inactive(&key);
            }
            ExistingContainerDecision::StartStopped => {
                registry.forget_if_inactive(&key);
            }
            ExistingContainerDecision::Recreate => {
                if let Err(error) = remove_user_container(docker, &name).await {
                    tracing::debug!(?error, container_name = name, "idle sandbox recycle failed");
                    continue;
                }
                registry.forget_if_inactive(&key);
            }
        }
    }
}

fn append_tail(target: &mut String, text: &str, limit: usize) {
    target.push_str(text);
    if target.len() <= limit {
        return;
    }
    let mut start = target.len() - limit;
    while !target.is_char_boundary(start) {
        start += 1;
    }
    target.drain(..start);
}

fn outcome_prefix(nonce: &str) -> String {
    format!("__IRONCLAW_EXEC_OUTCOME_{nonce}=")
}

fn parse_exec_outcome_trailer(tail: &str, nonce: &str) -> Result<ExecOutcome, RuntimeProcessError> {
    let prefix = outcome_prefix(nonce);
    let Some(start) = tail.rfind(&prefix) else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox exec outcome trailer is missing".to_string(),
        ));
    };
    let value = tail[start + prefix.len()..]
        .split_once('\n')
        .map_or(&tail[start + prefix.len()..], |(value, _)| value);
    parse_exec_outcome(value)
}

fn strip_exec_outcome_trailer(output: &mut String, nonce: &str) {
    let marker = format!("\n{}", outcome_prefix(nonce));
    if let Some(start) = output.rfind(&marker) {
        output.truncate(start);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecOutcome {
    Timeout,
    Exit(i64),
}

fn parse_exec_outcome(value: &str) -> Result<ExecOutcome, RuntimeProcessError> {
    let value = value.trim();
    if value == "timeout" {
        return Ok(ExecOutcome::Timeout);
    }
    let Some(exit) = value.strip_prefix("exit:") else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox exec outcome marker is malformed".to_string(),
        ));
    };
    let exit = exit.parse::<u8>().map_err(|_| {
        RuntimeProcessError::ExecutionFailed(
            "sandbox exec outcome marker has an invalid exit code".to_string(),
        )
    })?;
    Ok(ExecOutcome::Exit(i64::from(exit)))
}

struct BoundedExecOutput {
    stdout: String,
    stderr: String,
    stream_limit: usize,
    total_limit: usize,
}

impl BoundedExecOutput {
    fn new(total_limit: usize) -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            stream_limit: total_limit / 2,
            total_limit,
        }
    }

    fn push_stdout(&mut self, text: &str) {
        append_with_limit(&mut self.stdout, text, self.stream_limit);
    }

    fn push_stderr(&mut self, text: &str) {
        append_with_limit(&mut self.stderr, text, self.stream_limit);
    }

    fn finish(self) -> String {
        let mut combined = String::new();
        append_with_limit(&mut combined, &self.stdout, self.total_limit);
        if !self.stderr.is_empty() {
            let separator = if self.stdout.is_empty() {
                ""
            } else {
                "\n\n--- stderr ---\n"
            };
            append_with_limit(&mut combined, separator, self.total_limit);
            append_with_limit(&mut combined, &self.stderr, self.total_limit);
        }
        combined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_bounded_across_stdout_stderr_and_separator() {
        let mut output = BoundedExecOutput::new(16);
        output.push_stdout("abcdefghijk");
        output.push_stderr("0123456789");
        let combined = output.finish();

        assert!(combined.len() <= 16);
        assert!(combined.starts_with("abcdefgh"));
    }

    #[test]
    fn output_truncation_keeps_utf8_boundaries() {
        let mut output = BoundedExecOutput::new(7);
        output.push_stdout("éééé");
        let combined = output.finish();

        assert!(combined.is_char_boundary(combined.len()));
        assert!(combined.len() <= 7);
    }

    #[test]
    fn outcome_marker_distinguishes_timeout_from_every_exit_code() {
        assert_eq!(
            parse_exec_outcome("timeout\n").unwrap(),
            ExecOutcome::Timeout
        );
        assert_eq!(parse_exec_outcome("exit:0").unwrap(), ExecOutcome::Exit(0));
        assert_eq!(
            parse_exec_outcome("exit:124").unwrap(),
            ExecOutcome::Exit(124)
        );
        assert_eq!(
            parse_exec_outcome("exit:255").unwrap(),
            ExecOutcome::Exit(255)
        );
        assert!(parse_exec_outcome("exit:256").is_err());
        assert!(parse_exec_outcome("unknown").is_err());
    }

    #[test]
    fn bounded_tail_retains_final_trailer_and_utf8_boundaries() {
        let nonce = "abc-123";
        let trailer = format!("\n{}exit:124\n", outcome_prefix(nonce));
        let mut tail = String::new();
        append_tail(&mut tail, &format!("{}{}", "é".repeat(400), trailer), 512);

        assert!(tail.is_char_boundary(tail.len()));
        assert!(tail.len() <= 512);
        assert_eq!(
            parse_exec_outcome_trailer(&tail, nonce).unwrap(),
            ExecOutcome::Exit(124)
        );
    }

    #[test]
    fn outcome_trailer_uses_last_exact_nonce_and_strips_only_supervisor_line() {
        let nonce = "abc-123";
        let mut output = format!(
            "command wrote {}exit:7\nlater output\n{}timeout\n",
            outcome_prefix(nonce),
            outcome_prefix(nonce)
        );
        assert_eq!(
            parse_exec_outcome_trailer(&output, nonce).unwrap(),
            ExecOutcome::Timeout
        );
        strip_exec_outcome_trailer(&mut output, nonce);
        assert!(output.contains("command wrote"));
        assert!(!output.ends_with("timeout\n"));
        assert!(parse_exec_outcome_trailer("ordinary output", nonce).is_err());
    }

    #[test]
    fn adoption_recycles_user_container_on_image_or_posture_mismatch() {
        let expected = HashMap::from([
            (
                super::super::registry::label_image(LABEL_PREFIX),
                "sha256:image-v1".to_string(),
            ),
            (
                super::super::registry::label_security_posture(LABEL_PREFIX),
                "posture-v1".to_string(),
            ),
        ]);
        let compatible = ContainerInspectResponse {
            state: Some(bollard::models::ContainerState {
                running: Some(true),
                ..Default::default()
            }),
            image: Some("sha256:image-v1".to_string()),
            config: Some(bollard::models::ContainerConfig {
                image: Some("image:latest".to_string()),
                labels: Some(expected.clone()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            user_container_adoption_decision(&compatible, &expected),
            ExistingContainerDecision::ReuseRunning
        );

        let mut wrong_image = compatible.clone();

        wrong_image.image = Some("sha256:image-v2".to_string());
        assert_eq!(
            user_container_adoption_decision(&wrong_image, &expected),
            ExistingContainerDecision::Recreate
        );

        let mut wrong_posture = compatible;
        wrong_posture
            .config
            .as_mut()
            .unwrap()
            .labels
            .as_mut()
            .unwrap()
            .insert(
                super::super::registry::label_security_posture(LABEL_PREFIX),
                "posture-v2".to_string(),
            );
        assert_eq!(
            user_container_adoption_decision(&wrong_posture, &expected),
            ExistingContainerDecision::Recreate
        );
    }

    #[tokio::test]
    async fn failed_container_removal_is_reported_and_retained_for_retry() {
        let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", unavailable.local_addr().unwrap());
        drop(unavailable);
        let docker = Docker::connect_with_http(&endpoint, 1, bollard::API_DEFAULT_VERSION).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let transport = super::super::test_support::transport(
            docker,
            super::super::RebornSandboxConfig::new(temp.path().join("workspaces")),
        );
        let tenant = TenantId::new("recycle-failure-tenant").unwrap();
        let user = UserId::new("recycle-failure-user").unwrap();
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        let _activity = transport.activity.begin(&key).unwrap();
        let container_name = key.container_name();

        let error = remove_user_container(&transport.docker, &container_name)
            .await
            .expect_err("Docker removal failure must reach the caller");
        assert!(
            error
                .to_string()
                .contains("sandbox container recycle failed"),
            "removal error lost its operation context: {error}"
        );

        recycle_untrusted_user_container(&transport, &key, &container_name).await;
        assert!(
            transport.activity.recycle_required(&key),
            "failed cleanup must remain marked so the next serialized command retries it"
        );
    }

    #[tokio::test]
    async fn dropping_last_sweeper_owner_aborts_its_task() {
        let registry = Arc::new(SandboxActivityRegistry::new());
        let registry_weak = Arc::downgrade(&registry);
        let task_registry = Arc::clone(&registry);
        let (shutdown, _receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(task_registry);
        });
        let sweeper = Arc::new(UserContainerSweeper {
            shutdown,
            task: std::sync::Mutex::new(Some(task)),
        });

        drop(registry);
        drop(sweeper);
        tokio::task::yield_now().await;

        assert!(
            registry_weak.upgrade().is_none(),
            "dropping the final sweeper owner must release task captures"
        );
    }
}
