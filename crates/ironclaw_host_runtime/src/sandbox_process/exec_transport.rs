//! Exec-based lifecycle for the persistent per-user sandbox container.
//!
//! Replaces the ephemeral per-command create/run/remove model entirely —
//! there is no fallback path, per the design's "Relation to ephemeral
//! model: Replace" decision. [`ensure_container`] reuses (or transparently
//! restarts) the one container that already exists for a `{tenant, user}`
//! pair, keyed by the Docker labels [`super::registry`] attaches; every
//! individual shell command then runs as a fresh `docker exec` via
//! [`exec_in_container`] rather than a fresh container.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use bollard::{
    Docker,
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions, LogOutput,
        StartContainerOptions,
    },
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::HostConfig,
};
use futures_util::StreamExt;
use ironclaw_host_api::{TenantId, UserId};

use crate::{CommandExecutionOutput, RuntimeProcessError};

use super::{
    ContainerWorkdir, LABEL_PREFIX, RebornSandboxConfig, RebornSandboxUserKey,
    registry::{self, build_user_container_labels, user_container_label_filter},
    shell_single_quote,
};

/// Finds the one container already labeled for `{tenant_id, user_id}` and
/// makes sure it is running (creating or restarting it as needed), or
/// creates a fresh one if none exists yet. Returns the container ID a
/// subsequent [`exec_in_container`] call can target.
pub(super) async fn ensure_container(
    docker: &Docker,
    config: &RebornSandboxConfig,
    key: &RebornSandboxUserKey,
    tenant_id: &TenantId,
    user_id: &UserId,
    workspace: &Path,
) -> Result<String, RuntimeProcessError> {
    let filters = user_container_label_filter(LABEL_PREFIX, tenant_id, user_id);
    let found = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox container lookup failed: {error}"
            ))
        })?;

    match found.as_slice() {
        [] => {
            create_and_start_user_container(docker, config, key, tenant_id, user_id, workspace)
                .await
        }
        [existing] => {
            let container_id = existing.id.clone().ok_or_else(|| {
                RuntimeProcessError::ExecutionFailed(
                    "sandbox container lookup returned an unnamed container".to_string(),
                )
            })?;
            ensure_running(docker, &container_id).await?;
            Ok(container_id)
        }
        multiple => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox container registry has {} containers for one user; expected at most one",
            multiple.len()
        ))),
    }
}

async fn ensure_running(docker: &Docker, container_id: &str) -> Result<(), RuntimeProcessError> {
    let inspected = docker
        .inspect_container(container_id, None::<InspectContainerOptions>)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox container inspect failed: {error}"
            ))
        })?;
    let running = inspected
        .state
        .as_ref()
        .and_then(|state| state.running)
        .unwrap_or(false);
    if !running {
        docker
            .start_container(container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox container restart failed: {error}"
                ))
            })?;
    }
    Ok(())
}

async fn create_and_start_user_container(
    docker: &Docker,
    config: &RebornSandboxConfig,
    key: &RebornSandboxUserKey,
    tenant_id: &TenantId,
    user_id: &UserId,
    workspace: &Path,
) -> Result<String, RuntimeProcessError> {
    let launch = user_container_launch_config(config, tenant_id, user_id, workspace).await?;
    let created = docker
        .create_container(
            Some(CreateContainerOptions {
                name: key.container_name(),
                platform: None,
            }),
            launch,
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox container create failed: {error}"
            ))
        })?;
    docker
        .start_container(&created.id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox container start failed: {error}"))
        })?;
    Ok(created.id)
}

/// The persistent container's own launch `cmd` is a no-op long-lived
/// process (`sleep infinity`) — the container never runs the model's
/// command directly; every command arrives later via `docker exec`.
pub(super) async fn user_container_launch_config(
    config: &RebornSandboxConfig,
    tenant_id: &TenantId,
    user_id: &UserId,
    workspace: &Path,
) -> Result<Config<String>, RuntimeProcessError> {
    let labels = build_user_container_labels(LABEL_PREFIX, tenant_id, user_id);
    let mut env = config.command_env(std::collections::HashMap::new())?;
    env.push("HOME=/workspace/.home".to_string());
    let container_user = config.container_identity.container_user()?;
    let mut binds = config
        .mount_sources
        .prepare_container_binds(workspace, None)
        .await?
        .into_iter()
        .map(|bind| bind.into_docker_bind())
        .collect::<Vec<_>>();
    config.append_broker_binds(&mut binds)?;
    let host_config = HostConfig {
        binds: Some(binds),
        memory: Some(config.memory_bytes as i64),
        cpu_shares: Some(config.cpu_shares as i64),
        auto_remove: Some(false),
        network_mode: config.container_network_mode(),
        cap_drop: Some(vec!["ALL".to_string()]),
        security_opt: Some(vec!["no-new-privileges:true".to_string()]),
        readonly_rootfs: Some(true),
        tmpfs: Some(
            [("/tmp".to_string(), "size=512M".to_string())]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    };
    Ok(Config {
        image: Some(config.image.clone()),
        cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
        env: Some(env),
        labels: Some(labels),
        host_config: Some(host_config),
        user: container_user,
        attach_stdout: Some(false),
        attach_stderr: Some(false),
        ..Default::default()
    })
}

/// `setsid` creates a new session AND process group whose pgid equals its
/// own pid. `exec` replaces the current process image in place, so the pid
/// docker reports via `inspect_exec` stays the same value and now doubles
/// as the whole job's pgid — `kill -KILL -<pid>` on timeout reaches every
/// descendant without touching the container's own PID 1.
fn wrap_command_for_pgid_isolation(command: &str) -> String {
    format!("exec setsid sh -c {}", shell_single_quote(command))
}

pub(super) async fn exec_in_container(
    docker: &Docker,
    container_id: &str,
    workdir: ContainerWorkdir,
    env: Vec<String>,
    command: String,
    timeout: Duration,
    output_limit: usize,
) -> Result<CommandExecutionOutput, RuntimeProcessError> {
    let wrapped = wrap_command_for_pgid_isolation(&command);
    let exec = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["sh".to_string(), "-c".to_string(), wrapped]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                working_dir: Some(workdir.into_string()),
                env: Some(env),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox exec create failed: {error}"))
        })?;
    let started_at = Instant::now();

    let run = async {
        match docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!("sandbox exec start failed: {error}"))
            })? {
            StartExecResults::Attached { output, .. } => {
                collect_exec_output(output, output_limit).await
            }
            StartExecResults::Detached => Err(RuntimeProcessError::ExecutionFailed(
                "sandbox exec unexpectedly detached".to_string(),
            )),
        }
    };

    let output = match tokio::time::timeout(timeout, run).await {
        Ok(result) => result?,
        Err(_) => {
            kill_exec_process_group(docker, container_id, &exec.id).await;
            return Err(RuntimeProcessError::Timeout(timeout));
        }
    };

    let exit_code = docker
        .inspect_exec(&exec.id)
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("sandbox exec inspect failed: {error}"))
        })?
        .exit_code
        .unwrap_or(-1);

    Ok(CommandExecutionOutput {
        output,
        saved_output: None,
        exit_code,
        sandboxed: true,
        duration: started_at.elapsed(),
    })
}

/// Result of launching a detached (`background: true`) command inside the
/// container: the pid the launch script reported (which is also its
/// `setsid`-created pgid, per [`wrap_command_for_pgid_isolation`]) and the
/// container-local log path its stdout/stderr were redirected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BackgroundLaunch {
    pub(super) pid: u32,
    pub(super) log_path: String,
}

/// Launches `command` detached inside the container (`setsid ... &`),
/// redirecting its output to a per-pid log under `/workspace/.ironclaw/`,
/// and returns immediately with the launched pid instead of waiting for
/// completion.
pub(super) async fn exec_background_in_container(
    docker: &Docker,
    container_id: &str,
    workdir: ContainerWorkdir,
    env: Vec<String>,
    command: String,
) -> Result<BackgroundLaunch, RuntimeProcessError> {
    let launch_script = format!(
        "mkdir -p /workspace/.ironclaw && {} >>/workspace/.ironclaw/bg-$$.log 2>&1 & echo $!",
        wrap_command_for_pgid_isolation(&command),
    );
    let exec = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["sh".to_string(), "-c".to_string(), launch_script]),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                working_dir: Some(workdir.into_string()),
                env: Some(env),
                ..Default::default()
            },
        )
        .await
        .map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "sandbox background launch failed: {error}"
            ))
        })?;
    let launch_timeout = Duration::from_secs(10);
    let pid_output = tokio::time::timeout(launch_timeout, async {
        match docker
            .start_exec(
                &exec.id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox background launch start failed: {error}"
                ))
            })? {
            StartExecResults::Attached { output, .. } => collect_exec_output(output, 256).await,
            StartExecResults::Detached => Ok(String::new()),
        }
    })
    .await
    .map_err(|_| RuntimeProcessError::Timeout(launch_timeout))??;
    let pid: u32 = pid_output.trim().parse().map_err(|_| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox background launch did not report a pid: {pid_output:?}"
        ))
    })?;
    Ok(BackgroundLaunch {
        pid,
        log_path: format!("/workspace/.ironclaw/bg-{pid}.log"),
    })
}

/// Renders the "still-live background processes" footer appended to every
/// foreground shell result. Empty when there are no tracked survivors.
pub(super) fn render_background_footer(jobs: &[registry::BackgroundJob]) -> String {
    if jobs.is_empty() {
        return String::new();
    }
    let mut footer = String::from("\n\nLive background processes:");
    for job in jobs {
        footer.push_str(&format!("\n  pid {} ({})", job.pid, job.command_preview));
    }
    footer
}

#[cfg(test)]
mod footer_tests {
    use super::*;

    #[test]
    fn empty_job_list_renders_no_footer() {
        assert_eq!(render_background_footer(&[]), "");
    }

    #[test]
    fn footer_lists_every_survivor_with_pid_and_command_preview() {
        let jobs = vec![
            registry::BackgroundJob {
                pid: 101,
                command_preview: "npm run dev".to_string(),
            },
            registry::BackgroundJob {
                pid: 202,
                command_preview: "python -m http.server".to_string(),
            },
        ];
        assert_eq!(
            render_background_footer(&jobs),
            "\n\nLive background processes:\n  pid 101 (npm run dev)\n  pid 202 (python -m http.server)",
        );
    }
}

/// Best-effort: kills the whole process group the timed-out exec started
/// (see [`wrap_command_for_pgid_isolation`]), but never fails the caller
/// over it — the caller already treats the command as having timed out
/// regardless of whether this cleanup exec itself succeeds.
async fn kill_exec_process_group(docker: &Docker, container_id: &str, exec_id: &str) {
    let Ok(inspected) = docker.inspect_exec(exec_id).await else {
        return;
    };
    let Some(pid) = inspected.pid else { return };
    let kill_cmd = format!("kill -KILL -{pid} 2>/dev/null || true");
    if let Ok(kill_exec) = docker
        .create_exec(
            container_id,
            CreateExecOptions {
                cmd: Some(vec!["sh".to_string(), "-c".to_string(), kill_cmd]),
                attach_stdout: Some(false),
                attach_stderr: Some(false),
                ..Default::default()
            },
        )
        .await
        && let Err(error) = docker
            .start_exec(
                &kill_exec.id,
                Some(StartExecOptions {
                    detach: true,
                    ..Default::default()
                }),
            )
            .await
    {
        tracing::debug!(?error, "best-effort sandbox exec timeout kill failed");
    }
}

async fn collect_exec_output(
    mut stream: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>,
    >,
    limit: usize,
) -> Result<String, RuntimeProcessError> {
    let mut stdout = String::new();
    let mut stderr = String::new();
    let half_limit = limit / 2;
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(LogOutput::StdOut { message }) => {
                super::append_with_limit(
                    &mut stdout,
                    &String::from_utf8_lossy(&message),
                    half_limit,
                );
            }
            Ok(LogOutput::StdErr { message }) => {
                super::append_with_limit(
                    &mut stderr,
                    &String::from_utf8_lossy(&message),
                    half_limit,
                );
            }
            Ok(_) => {}
            Err(error) => {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "sandbox exec output collection failed: {error}"
                )));
            }
        }
    }
    if stderr.is_empty() {
        Ok(stdout)
    } else if stdout.is_empty() {
        Ok(stderr)
    } else {
        Ok(format!("{stdout}\n\n--- stderr ---\n{stderr}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_command_runs_under_setsid_for_process_group_isolation() {
        let wrapped = wrap_command_for_pgid_isolation("echo hi && sleep 1");
        assert_eq!(wrapped, "exec setsid sh -c 'echo hi && sleep 1'");
    }

    #[tokio::test]
    async fn user_container_launch_config_uses_persistent_cmd_and_user_labels() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let config = RebornSandboxConfig::new(temp.path().join("workspaces"));
        let tenant = ironclaw_host_api::TenantId::new("tenant-a").unwrap();
        let user = ironclaw_host_api::UserId::new("user-a").unwrap();

        let launch = user_container_launch_config(&config, &tenant, &user, &workspace)
            .await
            .unwrap();

        assert_eq!(
            launch.cmd,
            Some(vec!["sleep".to_string(), "infinity".to_string()])
        );
        let labels = launch.labels.unwrap();
        assert_eq!(labels.get("ironclaw.tenant").unwrap(), "tenant-a");
        assert_eq!(labels.get("ironclaw.user").unwrap(), "user-a");
        let env = launch.env.unwrap();
        assert!(env.iter().any(|e| e == "HOME=/workspace/.home"));
        let host_config = launch.host_config.unwrap();
        assert_eq!(host_config.auto_remove, Some(false));
    }
}

// `#[path]` resolution for a module declared inline inside another inline
// module is relative to a *fictitious* per-module directory chain (here
// `src/sandbox_process/exec_transport/docker_tests/`, which does not exist
// on disk) — verified empirically, since none of those intermediate
// directories are real. Declaring `docker_gate` at THIS file's top level
// instead resolves relative to `exec_transport.rs`'s own real directory
// (`src/sandbox_process/`, two levels above the crate root), matching the
// convention `sandbox_reaper_docker.rs` uses one level up (that file sits
// directly in `tests/`, so it only needs `"support/docker_gate.rs"`).
#[cfg(test)]
#[path = "../../tests/support/docker_gate.rs"]
mod docker_gate;

/// Real-Docker tests for the exec-based persistent container lifecycle.
/// Gated the way `sandbox_reaper_docker.rs` already gates its tests: a
/// visible `SKIP: ...` line, never a silent `#[ignore]` vanish.
#[cfg(test)]
mod docker_tests {
    use super::*;
    use bollard::container::RemoveContainerOptions;

    fn docker_tests_config(workspaces_root: &Path) -> RebornSandboxConfig {
        RebornSandboxConfig::new(workspaces_root.to_path_buf())
            .with_image(docker_gate::configured_sandbox_image())
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
    async fn exec_reuses_container_across_commands_file_persists_env_does_not() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — exec_reuses_container_across_commands_file_persists_env_does_not requires a real Docker daemon (CI/hosted Docker lane only)"
            );
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image) {
            eprintln!(
                "SKIP: sandbox worker image {image:?} is not built locally — requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
            );
            return;
        }

        let docker = Docker::connect_with_local_defaults().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config = docker_tests_config(temp.path());
        let tenant = ironclaw_host_api::TenantId::new("exec-reuse-tenant").unwrap();
        let user = ironclaw_host_api::UserId::new("exec-reuse-user").unwrap();
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        let workspace = key.workspace_path(temp.path());
        std::fs::create_dir_all(&workspace).unwrap();

        let container_id = ensure_container(&docker, &config, &key, &tenant, &user, &workspace)
            .await
            .expect("first ensure_container creates the container");

        exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "echo persisted > /workspace/marker.txt".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("write exec succeeds");

        let read = exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "cat /workspace/marker.txt".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("read exec succeeds against the SAME container");
        assert!(
            read.output.contains("persisted"),
            "file written in one exec must be visible to the next: {read:?}"
        );

        let with_env = exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            vec!["PROBE_VAR=set".to_string()],
            "echo $PROBE_VAR".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("env-setting exec succeeds");
        assert!(with_env.output.contains("set"));

        let without_env = exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "echo [$PROBE_VAR]".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("later exec succeeds");
        assert!(
            without_env.output.contains("[]"),
            "env set in one exec must NOT bleed into the next (stateless exec): {without_env:?}"
        );

        best_effort_remove(&docker, &container_id).await;
    }

    #[tokio::test]
    async fn stopped_container_restarts_transparently_on_next_exec() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — stopped_container_restarts_transparently_on_next_exec requires a real Docker daemon (CI/hosted Docker lane only)"
            );
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image) {
            eprintln!(
                "SKIP: sandbox worker image {image:?} is not built locally — requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
            );
            return;
        }

        let docker = Docker::connect_with_local_defaults().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config = docker_tests_config(temp.path());
        let tenant = ironclaw_host_api::TenantId::new("restart-tenant").unwrap();
        let user = ironclaw_host_api::UserId::new("restart-user").unwrap();
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        let workspace = key.workspace_path(temp.path());
        std::fs::create_dir_all(&workspace).unwrap();

        let container_id = ensure_container(&docker, &config, &key, &tenant, &user, &workspace)
            .await
            .expect("first ensure_container creates the container");
        docker
            .stop_container(&container_id, None)
            .await
            .expect("stop out of band");

        let reused_id = ensure_container(&docker, &config, &key, &tenant, &user, &workspace)
            .await
            .expect("ensure_container transparently restarts a stopped container");
        assert_eq!(
            reused_id, container_id,
            "restart must reuse the same container, not recreate one"
        );

        let output = exec_in_container(
            &docker,
            &reused_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "echo alive".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("exec against the restarted container succeeds");
        assert!(output.output.contains("alive"));

        best_effort_remove(&docker, &container_id).await;
    }

    #[tokio::test]
    async fn timeout_kills_process_group_but_container_survives() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — timeout_kills_process_group_but_container_survives requires a real Docker daemon (CI/hosted Docker lane only)"
            );
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image) {
            eprintln!(
                "SKIP: sandbox worker image {image:?} is not built locally — requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
            );
            return;
        }

        let docker = Docker::connect_with_local_defaults().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config = docker_tests_config(temp.path());
        let tenant = ironclaw_host_api::TenantId::new("timeout-tenant").unwrap();
        let user = ironclaw_host_api::UserId::new("timeout-user").unwrap();
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        let workspace = key.workspace_path(temp.path());
        std::fs::create_dir_all(&workspace).unwrap();
        let container_id = ensure_container(&docker, &config, &key, &tenant, &user, &workspace)
            .await
            .expect("ensure_container succeeds");

        let timed_out = exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "sleep 100".to_string(),
            Duration::from_secs(1),
            4096,
        )
        .await;
        assert!(
            matches!(timed_out, Err(RuntimeProcessError::Timeout(_))),
            "long-running exec must time out: {timed_out:?}"
        );

        let still_alive = exec_in_container(
            &docker,
            &container_id,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "echo alive".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .expect("the container itself must survive a timeout kill of the exec'd process group");
        assert!(still_alive.output.contains("alive"));

        best_effort_remove(&docker, &container_id).await;
    }

    #[tokio::test]
    async fn cross_user_containers_and_workspaces_are_isolated() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — cross_user_containers_and_workspaces_are_isolated requires a real Docker daemon (CI/hosted Docker lane only)"
            );
            return;
        }
        let image = docker_gate::configured_sandbox_image();
        if !docker_gate::docker_image_available(&image) {
            eprintln!(
                "SKIP: sandbox worker image {image:?} is not built locally — requires a locally-built ironclaw-worker image (CI/hosted Docker lane only)"
            );
            return;
        }

        let docker = Docker::connect_with_local_defaults().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let config = docker_tests_config(temp.path());

        let tenant = ironclaw_host_api::TenantId::new("isolation-tenant").unwrap();
        let user_a = ironclaw_host_api::UserId::new("isolation-user-a").unwrap();
        let user_b = ironclaw_host_api::UserId::new("isolation-user-b").unwrap();
        let key_a = RebornSandboxUserKey::from_tenant_user(&tenant, &user_a);
        let key_b = RebornSandboxUserKey::from_tenant_user(&tenant, &user_b);
        let workspace_a = key_a.workspace_path(temp.path());
        let workspace_b = key_b.workspace_path(temp.path());
        std::fs::create_dir_all(&workspace_a).unwrap();
        std::fs::create_dir_all(&workspace_b).unwrap();

        let container_a =
            ensure_container(&docker, &config, &key_a, &tenant, &user_a, &workspace_a)
                .await
                .unwrap();
        let container_b =
            ensure_container(&docker, &config, &key_b, &tenant, &user_b, &workspace_b)
                .await
                .unwrap();
        assert_ne!(
            container_a, container_b,
            "distinct users must get distinct containers"
        );

        exec_in_container(
            &docker,
            &container_a,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "echo user-a-secret > /workspace/user-a-only.txt".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .unwrap();

        let leak_check = exec_in_container(
            &docker,
            &container_b,
            ContainerWorkdir::workspace_root(),
            Vec::new(),
            "cat /workspace/user-a-only.txt 2>&1 || echo NOT_FOUND".to_string(),
            Duration::from_secs(10),
            4096,
        )
        .await
        .unwrap();
        assert!(
            leak_check.output.contains("NOT_FOUND"),
            "user B's container must not see user A's workspace file: {leak_check:?}"
        );

        // The design's hard invariant: user B's workspace host path must
        // not appear ANYWHERE in user A's container mount table, and vice
        // versa — a bind-mount-source leak would be a full sandbox escape.
        let inspected_a = docker.inspect_container(&container_a, None).await.unwrap();
        let binds_a = inspected_a.host_config.unwrap().binds.unwrap_or_default();
        let workspace_b_str = workspace_b.to_string_lossy().to_string();
        assert!(
            binds_a.iter().all(|bind| !bind.contains(&workspace_b_str)),
            "user B's workspace path must not appear in user A's mount table: {binds_a:?}"
        );

        let inspected_b = docker.inspect_container(&container_b, None).await.unwrap();
        let binds_b = inspected_b.host_config.unwrap().binds.unwrap_or_default();
        let workspace_a_str = workspace_a.to_string_lossy().to_string();
        assert!(
            binds_b.iter().all(|bind| !bind.contains(&workspace_a_str)),
            "user A's workspace path must not appear in user B's mount table: {binds_b:?}"
        );

        best_effort_remove(&docker, &container_a).await;
        best_effort_remove(&docker, &container_b).await;
    }
}
