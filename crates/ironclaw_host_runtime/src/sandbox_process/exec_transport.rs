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
    errors::Error as DockerError,
    exec::{CreateExecOptions, StartExecOptions, StartExecResults},
    models::{HostConfig, Ipam, IpamConfig},
    network::CreateNetworkOptions,
};
use futures_util::StreamExt;
use ironclaw_host_api::{TenantId, UserId};

use crate::{CommandExecutionOutput, RuntimeProcessError};

use super::{
    ContainerWorkdir, LABEL_PREFIX, RebornSandboxConfig, RebornSandboxUserKey,
    broker::{
        SANDBOX_EGRESS_NETWORK_GATEWAY, SANDBOX_EGRESS_NETWORK_NAME, SANDBOX_EGRESS_NETWORK_SUBNET,
    },
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
    network_ready: &tokio::sync::OnceCell<()>,
) -> Result<String, RuntimeProcessError> {
    ensure_egress_network_once(docker, config, network_ready).await?;
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

/// Idempotently creates the pinned internal egress network (E1) before a
/// container that needs it joins. A no-op unless `config` actually resolves
/// to [`SANDBOX_EGRESS_NETWORK_NAME`] (no-net and fully-open-bridge configs
/// never call the Docker network API here).
///
/// Docker network creation is not atomic-on-conflict the way `CREATE TABLE
/// IF NOT EXISTS` is, so a losing racer against a concurrent create (e.g.
/// two users' first sandbox commands landing at once) gets a server error
/// back instead of success — [`is_network_already_exists_error`] treats that
/// as success too, since the end state (the network exists) is what this
/// function promises.
async fn ensure_egress_network(
    docker: &Docker,
    config: &RebornSandboxConfig,
) -> Result<(), RuntimeProcessError> {
    if config.container_network_mode().as_deref() != Some(SANDBOX_EGRESS_NETWORK_NAME) {
        return Ok(());
    }
    match docker
        .create_network(sandbox_egress_network_create_options())
        .await
    {
        Ok(_) => Ok(()),
        Err(error) if is_network_already_exists_error(&error) => Ok(()),
        Err(error) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "sandbox egress network ensure failed: {error}"
        ))),
    }
}

/// Gates [`ensure_egress_network`] behind `network_ready` so the (already
/// idempotent, per [`is_network_already_exists_error`]) create attempt only
/// actually rounds-trips to Docker once per process instead of on every
/// [`ensure_container`] call — `ensure_container` runs once per command
/// dispatch, so without this every command after the first pays a wasted
/// create-network round trip that Docker always 409s. `OnceCell` keeps this
/// correct under a race: concurrent callers before the first success share
/// the same in-flight attempt, and a failed attempt leaves the cell
/// uninitialized so the next call retries rather than wedging forever.
async fn ensure_egress_network_once(
    docker: &Docker,
    config: &RebornSandboxConfig,
    network_ready: &tokio::sync::OnceCell<()>,
) -> Result<(), RuntimeProcessError> {
    network_ready
        .get_or_try_init(|| ensure_egress_network(docker, config))
        .await
        .map(|_| ())
}

/// Pure builder for the `internal: true`, pinned-subnet network Docker
/// creates for [`SANDBOX_EGRESS_NETWORK_NAME`] — kept as a standalone
/// function so its shape is unit-testable without a Docker daemon (mirrors
/// how [`user_container_launch_config`] separates config assembly from the
/// `docker.create_container` call).
fn sandbox_egress_network_create_options() -> CreateNetworkOptions<String> {
    CreateNetworkOptions {
        name: SANDBOX_EGRESS_NETWORK_NAME.to_string(),
        check_duplicate: true,
        driver: "bridge".to_string(),
        // The load-bearing setting: no default route off-host, so the
        // egress proxy (reached at the pinned gateway, see
        // `SANDBOX_EGRESS_NETWORK_GATEWAY`) is the only way out.
        internal: true,
        ipam: Ipam {
            config: Some(vec![IpamConfig {
                subnet: Some(SANDBOX_EGRESS_NETWORK_SUBNET.to_string()),
                gateway: Some(SANDBOX_EGRESS_NETWORK_GATEWAY.to_string()),
                ..Default::default()
            }]),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// True when `error` indicates the network already exists (a prior boot, or
/// a concurrent racer, created it first) — the outcome
/// [`ensure_egress_network`] wants, not a failure. Matches on Docker's
/// typical 409-conflict status as well as the "already exists" message
/// text, since different Docker/DinD versions have been observed to surface
/// this either way.
fn is_network_already_exists_error(error: &DockerError) -> bool {
    match error {
        DockerError::DockerResponseServerError {
            status_code,
            message,
        } => *status_code == 409 || message.to_lowercase().contains("already exists"),
        _ => false,
    }
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
    // npm's global install prefix defaults to `/usr` on this Debian-apt
    // install (Dockerfile.process-sandbox), which is NOT `$HOME`-relative —
    // unlike cargo/rustup, `HOME=/workspace/.home` alone does not rescue it.
    // Under `readonly_rootfs: Some(true)` a bare `npm install -g` then fails
    // with EROFS. Redirect npm's prefix into the writable, persistent
    // workspace HOME instead.
    env.push("NPM_CONFIG_PREFIX=/workspace/.home/.npm-global".to_string());
    // Setting `Config.env`'s PATH here REPLACES the image-baked `ENV PATH`
    // for the whole container (and therefore every subsequent `docker exec`)
    // rather than extending it, so every directory a sandboxed command needs
    // must be restated explicitly: the new npm global bin dir, `pip
    // --user`'s console-script directory (installed but otherwise not on
    // PATH — those CLIs would be present but not invokable), the image's
    // baked cargo bin dir, and the standard system dirs.
    env.push(
        "PATH=/workspace/.home/.npm-global/bin:/workspace/.home/.local/bin:/home/sandbox/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
            .to_string(),
    );
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

/// Directory (inside the container) background job logs are written under.
const BACKGROUND_LOG_DIR: &str = "/workspace/.ironclaw";

/// Builds the launch script for a detached (`background: true`) command,
/// pure so the pid-agreement invariant below is unit-testable without a
/// Docker daemon.
///
/// The log filename must be derived from the exact same pid `$!` reports
/// back to the launching (outer) shell — but in POSIX sh, `$$` evaluated
/// *inside* a backgrounded compound command still expands to the INVOKING
/// shell's pid, not the forked job's; only `$!`, read by the shell that did
/// the forking, names the actual child. Redirecting via `>>bg-$$.log`
/// *outside* the wrapped command (as this used to) therefore wrote to a
/// different file than the one `$!` names, so the reported `log_path` never
/// matched the file the job actually wrote.
///
/// Fix: fold the redirect INSIDE the `setsid`-wrapped inner `sh -c` instead.
/// That inner shell is reached only through a chain of `exec`s (never a bare
/// fork) starting from the process `$!` names, and `exec` never changes a
/// process's pid — so by the time this inner shell starts up fresh and reads
/// its own `$$`, that value is the real pid of the process `$!` already
/// reported, and the two agree.
fn background_launch_script(command: &str) -> String {
    let logging_command = format!("exec >>{BACKGROUND_LOG_DIR}/bg-$$.log 2>&1; {command}");
    format!(
        "mkdir -p {BACKGROUND_LOG_DIR} && {} & echo $!",
        wrap_command_for_pgid_isolation(&logging_command),
    )
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
    let launch_script = background_launch_script(&command);
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
        log_path: format!("{BACKGROUND_LOG_DIR}/bg-{pid}.log"),
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

    /// Pins the pid-agreement invariant `background_launch_script` exists
    /// for: the `bg-$$.log` redirect must live strictly inside the
    /// single-quoted, freshly-`exec`'d inner `sh -c` — not in the outer,
    /// unquoted portion of the script — because only there does `$$` share
    /// its value with the `$!` the outer shell reports back to Rust. Before
    /// the fix, the redirect sat outside the wrap (`{wrapped} >>bg-$$.log`),
    /// so `$$` resolved to the wrong (invoking) shell's pid.
    #[test]
    fn background_launch_script_puts_the_dollar_dollar_log_redirect_inside_the_inner_shell() {
        let script = background_launch_script("echo hi");
        assert_eq!(
            script,
            "mkdir -p /workspace/.ironclaw && exec setsid sh -c 'exec >>/workspace/.ironclaw/bg-$$.log 2>&1; echo hi' & echo $!"
        );

        let inner_quote_start = script.find("sh -c '").unwrap() + "sh -c '".len();
        let redirect_pos = script
            .find("bg-$$.log")
            .expect("script must still redirect via a $$-derived filename");
        assert!(
            redirect_pos > inner_quote_start,
            "the bg-$$.log redirect must be inside the inner sh -c's quoted body, \
             where $$ agrees with the $! the outer shell reports: {script}"
        );

        // The final `echo $!` — read by Rust to build `BackgroundLaunch.pid`
        // and therefore `log_path` — must stay in the OUTER, unquoted part
        // of the script (it reports the outer shell's view of the forked
        // job, which is what the inner shell's own pid actually equals).
        assert!(
            script.ends_with("& echo $!"),
            "the outer shell must still report the launched job's pid via $!: {script}"
        );
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
        assert!(
            env.iter()
                .any(|e| e == "NPM_CONFIG_PREFIX=/workspace/.home/.npm-global"),
            "npm's global prefix is not $HOME-relative, so it must be redirected explicitly \
             to a writable path under readonly_rootfs: {env:?}"
        );
        let path_entry = env
            .iter()
            .find(|e| e.starts_with("PATH="))
            .unwrap_or_else(|| panic!("launch env must set an explicit PATH: {env:?}"));
        for expected in [
            "/workspace/.home/.npm-global/bin",
            "/workspace/.home/.local/bin",
            "/home/sandbox/.cargo/bin",
            "/usr/bin",
        ] {
            assert!(
                path_entry.contains(expected),
                "PATH must include {expected} (setting Config.env PATH replaces the \
                 image-baked ENV PATH for every docker exec): {path_entry:?}"
            );
        }
        let host_config = launch.host_config.unwrap();
        assert_eq!(host_config.auto_remove, Some(false));
    }

    #[test]
    fn sandbox_egress_network_create_options_pins_internal_subnet_and_gateway() {
        let options = sandbox_egress_network_create_options();

        assert_eq!(options.name, SANDBOX_EGRESS_NETWORK_NAME);
        assert!(
            options.internal,
            "must have no default route off-host (E1) — that's what makes the proxy the only way out"
        );
        let ipam_config = options
            .ipam
            .config
            .as_ref()
            .and_then(|configs| configs.first())
            .expect("network create options must pin an IPAM config");
        assert_eq!(
            ipam_config.subnet.as_deref(),
            Some(SANDBOX_EGRESS_NETWORK_SUBNET)
        );
        assert_eq!(
            ipam_config.gateway.as_deref(),
            Some(SANDBOX_EGRESS_NETWORK_GATEWAY)
        );
    }

    #[test]
    fn already_exists_network_error_is_treated_as_idempotent_success() {
        let conflict = DockerError::DockerResponseServerError {
            status_code: 409,
            message: format!("network with name {SANDBOX_EGRESS_NETWORK_NAME} already exists"),
        };
        assert!(is_network_already_exists_error(&conflict));

        let message_only = DockerError::DockerResponseServerError {
            status_code: 500,
            message: "Error: network with name ironclaw-sandbox-egress already exists".to_string(),
        };
        assert!(is_network_already_exists_error(&message_only));

        let unrelated = DockerError::DockerResponseServerError {
            status_code: 500,
            message: "internal server error".to_string(),
        };
        assert!(!is_network_already_exists_error(&unrelated));
    }

    #[tokio::test]
    async fn ensure_egress_network_is_a_no_op_for_none_network_configs() {
        // No live Docker daemon needed: `ensure_egress_network` must return
        // early (never issue a `docker.create_network` call) for a config
        // whose `container_network_mode()` isn't the egress network, so an
        // unreachable `Docker` handle is never actually used. Building an
        // HTTP-transport `Docker` client is lazy (no connection attempt
        // until a request is sent), unlike `connect_with_local_defaults`,
        // which stats the Unix socket path at construction and fails
        // immediately in this sandboxed environment (no
        // `/var/run/docker.sock`) — this exercises the guard clause without
        // either.
        let docker =
            Docker::connect_with_http("http://127.0.0.1:0", 120, bollard::API_DEFAULT_VERSION)
                .expect("HTTP-transport client construction performs no I/O");
        let temp = tempfile::tempdir().unwrap();

        let none_network_config = RebornSandboxConfig::new(temp.path().join("workspaces"));
        assert_eq!(
            none_network_config.container_network_mode(),
            Some("none".to_string())
        );
        ensure_egress_network(&docker, &none_network_config)
            .await
            .expect("no-net config must skip the network API entirely");
    }

    /// Once `network_ready` is initialized (as a prior `ensure_container`
    /// call would have left it after a successful ensure), a second call
    /// must short-circuit past `ensure_egress_network` entirely — proven
    /// here by pointing at an unreachable Docker transport with a config
    /// that *does* require the egress network: if the gate failed to
    /// short-circuit, this would try to reach Docker and return `Err`.
    #[tokio::test]
    async fn ensure_egress_network_once_short_circuits_once_already_initialized() {
        let docker =
            Docker::connect_with_http("http://127.0.0.1:0", 120, bollard::API_DEFAULT_VERSION)
                .expect("HTTP-transport client construction performs no I/O");
        let temp = tempfile::tempdir().unwrap();
        let egress_config = RebornSandboxConfig::new(temp.path().join("workspaces"))
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .expect("valid proxy url");
        assert_eq!(
            egress_config.container_network_mode(),
            Some(SANDBOX_EGRESS_NETWORK_NAME.to_string()),
            "test config must actually require the egress network for this test to be meaningful"
        );

        let network_ready = tokio::sync::OnceCell::new();
        network_ready
            .set(())
            .expect("freshly constructed OnceCell always accepts the first set");

        ensure_egress_network_once(&docker, &egress_config, &network_ready)
            .await
            .expect(
                "an already-initialized gate must short-circuit past the unreachable docker call",
            );
    }

    /// Sanity check paired with the short-circuit test above: an
    /// UNinitialized gate must still actually attempt the ensure (and thus
    /// surface the unreachable-Docker error) — otherwise the short-circuit
    /// test would pass vacuously regardless of whether gating works.
    #[tokio::test]
    async fn ensure_egress_network_once_attempts_the_ensure_when_not_yet_initialized() {
        let docker =
            Docker::connect_with_http("http://127.0.0.1:0", 120, bollard::API_DEFAULT_VERSION)
                .expect("HTTP-transport client construction performs no I/O");
        let temp = tempfile::tempdir().unwrap();
        let egress_config = RebornSandboxConfig::new(temp.path().join("workspaces"))
            .with_network_broker_proxy_url("http://broker.internal:8181")
            .expect("valid proxy url");

        let network_ready = tokio::sync::OnceCell::new();
        let result = ensure_egress_network_once(&docker, &egress_config, &network_ready).await;
        assert!(
            result.is_err(),
            "a not-yet-initialized gate must still attempt the ensure and surface the \
             unreachable-docker failure, not silently succeed"
        );
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

/// Real-Docker tests for the exec-based persistent container lifecycle that
/// genuinely need crate-private data. The rest of this module's former
/// coverage moved to
/// `crates/ironclaw_host_runtime/tests/sandbox_exec_transport_docker.rs`,
/// driven through the public `RuntimeProcessPort::run_command` surface —
/// this one test stays inline because it asserts the applied Docker
/// `HostConfig` against `RebornSandboxConfig`'s private `memory_bytes` /
/// `cpu_shares` fields, which have no public accessor and aren't worth
/// adding solely to relocate a test. Gated the way `sandbox_reaper_docker.rs`
/// already gates its tests: a visible `SKIP: ...` line, never a silent
/// `#[ignore]` vanish.
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

    /// Guards against the applied container's `HostConfig` diverging from
    /// `RebornSandboxConfig` — a limit can be coded into
    /// `user_container_launch_config` and unit-tested against the Rust
    /// `Config` struct while never actually taking effect against the real
    /// Docker daemon (e.g. a field docker silently ignores or overrides).
    /// This asserts against `docker inspect`'s own view, not the struct we
    /// built.
    #[tokio::test]
    async fn applied_container_limits_match_config_via_docker_inspect() {
        if !docker_gate::docker_available() {
            eprintln!(
                "SKIP: no docker daemon reachable — applied_container_limits_match_config_via_docker_inspect requires a real Docker daemon (CI/hosted Docker lane only)"
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
        let tenant = ironclaw_host_api::TenantId::new("limits-tenant").unwrap();
        let user = ironclaw_host_api::UserId::new("limits-user").unwrap();
        let key = RebornSandboxUserKey::from_tenant_user(&tenant, &user);
        let workspace = key.workspace_path(temp.path());
        std::fs::create_dir_all(&workspace).unwrap();

        let network_ready = tokio::sync::OnceCell::new();
        let container_id = ensure_container(
            &docker,
            &config,
            &key,
            &tenant,
            &user,
            &workspace,
            &network_ready,
        )
        .await
        .expect("ensure_container succeeds");

        let inspected = docker
            .inspect_container(&container_id, None::<InspectContainerOptions>)
            .await
            .expect("inspect succeeds");
        let host_config = inspected
            .host_config
            .expect("inspected container has a host config");

        assert_eq!(
            host_config.memory,
            Some(config.memory_bytes as i64),
            "applied memory limit must match config: {host_config:?}"
        );
        assert_eq!(
            host_config.cpu_shares,
            Some(config.cpu_shares as i64),
            "applied cpu_shares must match config: {host_config:?}"
        );
        assert_eq!(
            host_config.readonly_rootfs,
            Some(true),
            "applied container must have a readonly rootfs: {host_config:?}"
        );
        let cap_drop = host_config.cap_drop.unwrap_or_default();
        assert!(
            cap_drop.iter().any(|cap| cap == "ALL"),
            "applied container must drop ALL capabilities: {cap_drop:?}"
        );
        let security_opt = host_config.security_opt.unwrap_or_default();
        assert!(
            security_opt
                .iter()
                .any(|opt| opt == "no-new-privileges:true"),
            "applied container must set no-new-privileges: {security_opt:?}"
        );

        best_effort_remove(&docker, &container_id).await;
    }
}
