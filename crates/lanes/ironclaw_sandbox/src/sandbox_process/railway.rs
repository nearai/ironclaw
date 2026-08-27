//! Railway-backed preview provider for persistent user sandboxes.
//!
//! Railway is the trusted outer runtime. The model never receives Railway CLI
//! access: this transport invokes a scrubbed local CLI and the outer VM owns a
//! fixed inner Python image against a per-user directory in Railway's
//! checkpointed workspace. Inner containers are intentionally ephemeral:
//! Railway does not preserve their mount namespaces between outer exec calls.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use tokio::{io::AsyncReadExt, process::Command, sync::Mutex};

use ironclaw_host_api::process::{
    CommandExecutionOutput, CommandExecutionRequest, RuntimeProcessError, SandboxCommandTransport,
};

use crate::sandbox_process::RebornSandboxUserKey;

use super::worker_spec::DOCKER_WORKER_USER as WORKER_USER;

const DEFAULT_IDLE_TIMEOUT_MINUTES: u16 = 5;
// Pin the exact multi-platform manifest exercised by the Railway preview
// canary. Operators may override this explicitly, but the default must not
// drift between review and deployment.
const DEFAULT_WORKER_IMAGE: &str =
    "python:3.12-slim@sha256:57cd7c3a7a273101a6485ba99423ee568157882804b1124b4dd04266317710de";
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_OUTPUT_LIMIT: usize = 16 * 1024;
const REMOTE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const REMOTE_CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(120);
// A checkpoint list is provider metadata rather than model output. Keep it
// bounded, but large enough for the maximum tracked-user registry. Truncation
// produces invalid JSON and therefore fails closed instead of treating an
// existing durable checkpoint as absent.
const PROVIDER_LIST_OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const MAX_TRACKED_USERS: usize = 4_096;
const WORKSPACE_ROOT: &str = "/workspace";
const RAILWAY_WORKSPACES_ROOT: &str = "/workspace/ironclaw-users";
const EXIT_SENTINEL: &str = "__IRONCLAW_RAILWAY_PRIVATE_EXIT_CODE__=";
const WORKSPACE_BOOTSTRAP_MARKER: &str = "ironclaw-railway-workspace-bootstrap";
const CHECKPOINT_FAILURE_WARNING: &str = "[IronClaw warning: command completed, but the Railway workspace checkpoint failed; recent state may not survive sandbox restart.]";
const RAILWAY_PROXY_IP_TOKEN: &str = "__IRONCLAW_PROXY_IP__";
const RAILWAY_PROXY_BOOTSTRAP_CONFIG: &str = "dns:\n  listen: \"127.0.0.1:53\"\n  proxy_ip: \"127.0.0.1\"\nproxy:\n  http_listen: \"127.0.0.1:80\"\n  https_listen: \"127.0.0.1:443\"\n  tunnel_listen: \"127.0.0.1:3128\"\ntls:\n  mode: \"sni-only\"\ntransforms:\n  - name: allowlist\n    config:\n      domains: []\n";

// The complete Docker command arrives as distinct argv values. In particular,
// the model command is the final argv value and is never interpolated into
// this shell program.
const OUTER_EXEC_WRAPPER: &str = "set +e\n\"$@\"\nstatus=$?\nprintf '\\n__IRONCLAW_RAILWAY_PRIVATE_EXIT_CODE__=%s\\n' \"$status\"\nexit 0";
const RAILWAY_MANAGED_EGRESS_WRAPPER: &str = r#"set -eu
proxy_image=$1
proxy_posture=$2
config_template=$3
bootstrap_config=$4
invocation_id=$5
worker_image=$6
shift 6

network=ironclaw-reborn-private
upstream=ironclaw-reborn-upstream
proxy=ironclaw-reborn-proxy
material=/run/ironclaw-reborn-proxy
config_path=$material/proxy.yaml
invocation_path=$material/invocation-id

mkdir -p "$material"
chmod 700 "$material"
printf %s "$invocation_id" > "$invocation_path"
# The proxy drops DAC_OVERRIDE and can run under a different host UID.
# This is attribution metadata, not a credential.
chmod 644 "$invocation_path"

mkdir -p "$material/audit"
chmod 700 "$material/audit"
audit_log="$material/audit/proxy.log"
audit_capture="$material/audit/.capture"
audit_cursor="$material/audit/.cursor"
audit_append="$material/audit/.append"

# Appends only records strictly newer than the cursor. The cursor is the
# last drained record's own RFC3339Nano timestamp (Docker emits fixed-width
# UTC, so lexicographic comparison is chronological); the strict `$1 > c`
# filter makes the boundary exclusive, so back-to-back drains
# (post-command and next-invocation) never duplicate audit entries.
drain_proxy_audit() {
  if ! docker inspect "$proxy" >/dev/null 2>&1; then
    proxy_matches=$(docker ps --all --filter "name=^/${proxy}$" --format '{{.Names}}') || return 1
    [ -z "$proxy_matches" ] || return 1
    return 0
  fi
  if [ -f "$audit_log" ] && [ "$(wc -c < "$audit_log")" -gt 8388608 ]; then
    mv "$audit_log" "$audit_log.$(date +%s).$$"
  fi
  if [ -s "$audit_cursor" ]; then
    cursor=$(cat "$audit_cursor")
    docker logs --timestamps --since "$cursor" "$proxy" >"$audit_capture.raw" 2>&1 || return 1
    awk -v c="$cursor" '$1 > c' "$audit_capture.raw" > "$audit_capture"
    rm -f "$audit_capture.raw"
  else
    docker logs --timestamps "$proxy" >"$audit_capture" 2>&1 || return 1
  fi
  last_record=$(tail -n 1 "$audit_capture" | cut -d' ' -f1)
  tail -c 4194304 "$audit_capture" > "$audit_append" || return 1
  audit_total=$(wc -c < "$audit_append")
  for audit_file in "$material"/audit/proxy.log*; do
    [ -f "$audit_file" ] || continue
    audit_total=$((audit_total + $(wc -c < "$audit_file")))
  done
  if [ "$audit_total" -gt 268435456 ]; then
    rm -f "$audit_append" "$audit_capture"
    return 1
  fi
  cat "$audit_append" >> "$audit_log" || return 1
  chmod 600 "$audit_log"
  rm -f "$audit_append" "$audit_capture"
  if [ -n "$last_record" ]; then
    printf %s "$last_record" > "$audit_cursor"
  fi
  return 0
}

if ! drain_proxy_audit; then
  printf '%s\n' "Railway sandbox proxy audit capture failed; leaving prior proxy for retry" >&2
  exit 1
fi
docker rm --force "$proxy" >/dev/null 2>&1 || true
if docker network inspect "$network" >/dev/null 2>&1; then
  internal=$(docker network inspect --format '{{.Internal}}' "$network")
  gateway_mode=$(docker network inspect --format '{{index .Options "com.docker.network.bridge.gateway_mode_ipv4"}}' "$network")
  if [ "$internal" != true ] || [ "$gateway_mode" != isolated ]; then
    docker rm --force "$proxy" >/dev/null 2>&1 || true
    docker network rm "$network" >/dev/null
  fi
fi
if ! docker network inspect "$network" >/dev/null 2>&1; then
  if ! network_error=$(docker network create --internal --opt com.docker.network.bridge.gateway_mode_ipv4=isolated "$network" 2>&1); then
    case "$network_error" in
      *non-overlapping*address*pool*|*predefined*address*pools*subnetted*)
        printf '%s\n' "Railway sandbox private network address pool is exhausted; configure Docker default-address-pools in /etc/docker/daemon.json: $network_error" >&2
        ;;
      *)
        printf '%s\n' "Railway sandbox private network creation failed: $network_error" >&2
        ;;
    esac
    exit 1
  fi
fi

if docker network inspect "$upstream" >/dev/null 2>&1; then
  internal=$(docker network inspect --format '{{.Internal}}' "$upstream")
  if [ "$internal" != false ]; then
    docker network rm "$upstream" >/dev/null
  fi
fi
if ! docker network inspect "$upstream" >/dev/null 2>&1; then
  if ! network_error=$(docker network create "$upstream" 2>&1); then
    case "$network_error" in
      *non-overlapping*address*pool*|*predefined*address*pools*subnetted*)
        printf '%s\n' "Railway sandbox upstream network address pool is exhausted; configure Docker default-address-pools in /etc/docker/daemon.json: $network_error" >&2
        ;;
      *)
        printf '%s\n' "Railway sandbox upstream network creation failed: $network_error" >&2
        ;;
    esac
    exit 1
  fi
fi
printf %s "$bootstrap_config" > "$config_path"
chmod 600 "$config_path"
docker create \
  --name "$proxy" \
  --network "$network" \
  --label "ironclaw.proxy.posture=$proxy_posture" \
  --read-only \
  --cap-drop ALL \
  --cap-add NET_BIND_SERVICE \
  --security-opt no-new-privileges:true \
  --pids-limit 256 \
  --memory 128m \
  --tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m \
  --log-driver json-file \
  --log-opt max-size=1m \
  --log-opt max-file=1 \
  --mount "type=bind,src=$material,dst=/run/ironclaw-proxy,readonly" \
  "$proxy_image" -config /run/ironclaw-proxy/proxy.yaml >/dev/null
docker network connect "$upstream" "$proxy"
docker start "$proxy" >/dev/null
proxy_ip=$(docker inspect --format '{{(index .NetworkSettings.Networks "ironclaw-reborn-private").IPAddress}}' "$proxy")
docker stop --time 10 "$proxy" >/dev/null
printf %s "$config_template" | \
  sed "s|__IRONCLAW_PROXY_IP__|$proxy_ip|g" \
  > "$config_path"
docker start "$proxy" >/dev/null

attempt=0
until docker exec "$proxy" nc -z "$proxy_ip" 3128 >/dev/null 2>&1
do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 20 ]; then
    printf '%s\n' "Railway sandbox managed proxy failed readiness" >&2
    exit 1
  fi
  sleep 0.1
done

proxy_url="http://${proxy_ip}:3128"
set +e
docker run \
  --network "$network" \
  --dns "$proxy_ip" \
  --env IRONCLAW_REBORN_NETWORK_MODE=brokered \
  --env "IRONCLAW_REBORN_HTTP_PROXY=$proxy_url" \
  --env "http_proxy=$proxy_url" \
  --env "https_proxy=$proxy_url" \
  --env "HTTP_PROXY=$proxy_url" \
  --env "HTTPS_PROXY=$proxy_url" \
  --env NO_PROXY= \
  --env no_proxy= \
  "$@"
worker_status=$?
set -e
if ! drain_proxy_audit; then
  # The proxy and its Docker log survive; the next invocation's
  # fail-closed drain retries before any removal can destroy evidence.
  printf '%s\n' "Railway sandbox proxy audit capture failed after command exit" >&2
  exit 1
fi
exit "$worker_status"
"#;

/// Explicit, host-only configuration for the preview transport.
///
/// Railway auth is not stored here. Each CLI child copies only a selected
/// `RAILWAY_TOKEN` or `RAILWAY_API_TOKEN` plus minimal CLI runtime variables.
#[derive(Clone, PartialEq, Eq)]
/// Configuration for the experimental Railway sandbox transport.
///
/// # Deployment constraint
///
/// PR1 requires exactly one IronClaw application replica. Per-user command
/// serialization is process-local; Railway checkpoints are durable, but they
/// are not a distributed lease. Running multiple IronClaw replicas can let two
/// transports restore and replace the same per-user checkpoint concurrently.
pub struct RailwayPreviewSandboxConfig {
    project_id: String,
    environment_id: String,
    cli_path: PathBuf,
    idle_timeout_minutes: u16,
    worker_image: String,
    proxy_image: String,
    proxy_posture: String,
    proxy_config_template: String,
}

impl RailwayPreviewSandboxConfig {
    pub fn new(
        project_id: impl Into<String>,
        environment_id: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        let policy = super::network_allowlist::sandbox_network_policy().map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "Railway sandbox managed egress policy is invalid: {error}"
            ))
        })?;
        let proxy_image = super::managed_egress::configured_proxy_image()?;
        let proxy_posture = super::managed_egress::proxy_posture(
            &proxy_image,
            &policy,
            b"railway-preview-sni-only",
        )?;
        let proxy_config_template =
            super::managed_egress::render_proxy_config(&policy, RAILWAY_PROXY_IP_TOKEN)?;
        Ok(Self {
            project_id: required_config_value("project identifier", project_id.into())?,
            environment_id: required_config_value("environment identifier", environment_id.into())?,
            cli_path: PathBuf::from("railway"),
            idle_timeout_minutes: DEFAULT_IDLE_TIMEOUT_MINUTES,
            worker_image: DEFAULT_WORKER_IMAGE.to_string(),
            proxy_image,
            proxy_posture,
            proxy_config_template,
        })
    }

    pub fn with_cli_path(mut self, cli_path: impl Into<PathBuf>) -> Self {
        self.cli_path = cli_path.into();
        self
    }

    pub fn with_idle_timeout_minutes(mut self, minutes: u16) -> Result<Self, RuntimeProcessError> {
        if minutes == 0 {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview configuration requires a non-zero idle timeout".to_string(),
            ));
        }
        self.idle_timeout_minutes = minutes;
        Ok(self)
    }

    pub fn with_worker_image(
        mut self,
        image: impl Into<String>,
    ) -> Result<Self, RuntimeProcessError> {
        self.worker_image = required_config_value("worker image", image.into())?;
        Ok(self)
    }
}

impl std::fmt::Debug for RailwayPreviewSandboxConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RailwayPreviewSandboxConfig")
            .field("project_id", &self.project_id)
            .field("environment_id", &self.environment_id)
            .field("cli_path", &self.cli_path)
            .field("idle_timeout_minutes", &self.idle_timeout_minutes)
            .field("worker_image", &self.worker_image)
            .field("proxy_image", &self.proxy_image)
            .field("proxy_posture", &self.proxy_posture)
            .finish_non_exhaustive()
    }
}

/// Railway preview transport for a Docker daemon hosted inside a Railway
/// sandbox VM. Live IDs are process-local; deterministic per-user checkpoints
/// restore the outer workspace when a VM expires or IronClaw restarts. Each
/// command receives a fresh inner container so no mounted inner container has
/// to survive across Railway exec calls. Network access follows the explicit
/// configuration posture; construction defaults to `--network none`.
#[derive(Clone)]
pub struct RailwayPreviewSandboxTransport {
    config: RailwayPreviewSandboxConfig,
    cli: Arc<dyn RailwayCli>,
    users: Arc<Mutex<HashMap<RebornSandboxUserKey, TrackedUserState>>>,
    max_tracked_users: usize,
}

impl std::fmt::Debug for RailwayPreviewSandboxTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RailwayPreviewSandboxTransport")
            .field("project_id", &self.config.project_id)
            .field("environment_id", &self.config.environment_id)
            .field("cli_path", &self.config.cli_path)
            .field("idle_timeout_minutes", &self.config.idle_timeout_minutes)
            .field("worker_image", &self.config.worker_image)
            .finish_non_exhaustive()
    }
}

impl RailwayPreviewSandboxTransport {
    pub fn new(config: RailwayPreviewSandboxConfig) -> Self {
        Self {
            config,
            cli: Arc::new(SystemRailwayCli),
            users: Arc::new(Mutex::new(HashMap::new())),
            max_tracked_users: MAX_TRACKED_USERS,
        }
    }

    async fn state_for(
        &self,
        key: RebornSandboxUserKey,
    ) -> Result<Arc<Mutex<UserSandboxState>>, RuntimeProcessError> {
        let mut users = self.users.lock().await;
        if let Some(tracked) = users.get_mut(&key) {
            tracked.last_touched = Instant::now();
            return Ok(tracked.state.clone());
        }
        if users.len() >= self.max_tracked_users {
            let idle_lru = users
                .iter()
                .filter(|(_, tracked)| Arc::strong_count(&tracked.state) == 1)
                // Losing a pending ID could allow replacement provisioning
                // while cleanup of the previous sandbox remains unconfirmed.
                .filter(|(_, tracked)| {
                    tracked.state.try_lock().is_ok_and(|state| {
                        !matches!(state.lifecycle, UserSandboxLifecycle::CleanupPending(_))
                    })
                })
                .min_by_key(|(_, tracked)| tracked.last_touched)
                .map(|(key, _)| key.clone())
                .ok_or_else(|| {
                    RuntimeProcessError::ExecutionFailed(
                        "Railway preview user-state capacity is exhausted".to_string(),
                    )
                })?;
            users.remove(&idle_lru);
        }
        let state = Arc::new(Mutex::new(UserSandboxState::default()));
        users.insert(
            key,
            TrackedUserState {
                state: state.clone(),
                last_touched: Instant::now(),
            },
        );
        Ok(state)
    }

    async fn provision(
        &self,
        key: &RebornSandboxUserKey,
        state: &mut UserSandboxState,
        deadline: Instant,
        output_limit: usize,
    ) -> Result<String, RuntimeProcessError> {
        let checkpoint_name = checkpoint_name(key);
        let checkpoints = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ListCheckpoints,
                    checkpoint_list_argv(&self.config),
                    PROVIDER_LIST_OUTPUT_LIMIT,
                ),
                deadline,
            )
            .await?;
        let restore_checkpoint = checkpoint_exists(&checkpoints.stdout, &checkpoint_name)?;
        let created = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::CreateSandbox,
                    sandbox_create_argv(
                        &self.config,
                        restore_checkpoint.then_some(checkpoint_name.as_str()),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await?;
        let sandbox_id = parse_sandbox_id(&created.stdout)?;
        state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
        let bootstrap = async {
            let timeout = deadline_remaining(deadline)?;
            self.execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ExecuteSandboxCommand,
                    sandbox_exec_argv(
                        &self.config,
                        &sandbox_id,
                        timeout,
                        workspace_bootstrap_argv(&railway_workspace_path(key)),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await
        }
        .await;
        if let Err(bootstrap_error) = bootstrap {
            self.destroy_after_failed_execution(&sandbox_id, output_limit, &bootstrap_error)
                .await?;
            state.lifecycle = UserSandboxLifecycle::Absent;
            return Err(bootstrap_error);
        }
        state.lifecycle = UserSandboxLifecycle::Live(sandbox_id.clone());
        Ok(sandbox_id)
    }

    async fn sandbox_is_live(
        &self,
        sandbox_id: &str,
        deadline: Instant,
    ) -> Result<bool, RuntimeProcessError> {
        let sandboxes = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ListSandboxes,
                    sandbox_list_argv(&self.config),
                    PROVIDER_LIST_OUTPUT_LIMIT,
                ),
                deadline,
            )
            .await?;
        sandbox_exists(&sandboxes.stdout, sandbox_id)
    }

    async fn checkpoint(
        &self,
        key: &RebornSandboxUserKey,
        sandbox_id: &str,
        output_limit: usize,
    ) -> Result<(), RuntimeProcessError> {
        // Railway checkpoint names are unique within the environment and the
        // CLI's documented create semantics replace an existing checkpoint of
        // the same name synchronously. One deterministic name per user bounds
        // retention to the latest workspace snapshot; the live canary writes,
        // restores, and replaces that same name across transport restart.
        let result = self
            .cli
            .execute(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::CreateCheckpoint,
                    checkpoint_create_argv(&self.config, sandbox_id, &checkpoint_name(key)),
                    output_limit,
                ),
                REMOTE_CHECKPOINT_TIMEOUT,
            )
            .await;
        result.map(|_| ()).map_err(|error| {
            tracing::error!(
                ?error,
                sandbox_id,
                "Railway command completed but durable checkpoint creation failed"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview command completed but its workspace checkpoint failed".to_string(),
            )
        })
    }

    async fn destroy_after_failed_execution(
        &self,
        sandbox_id: &str,
        output_limit: usize,
        execution_error: &RuntimeProcessError,
    ) -> Result<(), RuntimeProcessError> {
        self.salvage_proxy_audit(sandbox_id, output_limit).await;
        if let Err(cleanup_error) = self.destroy_sandbox(sandbox_id, output_limit).await {
            tracing::error!(
                ?execution_error,
                ?cleanup_error,
                "Railway sandbox execution failed and remote cleanup was not confirmed"
            );
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox execution failed and remote cleanup could not be confirmed"
                    .to_string(),
            ));
        }
        Ok(())
    }

    async fn destroy_sandbox(
        &self,
        sandbox_id: &str,
        output_limit: usize,
    ) -> Result<(), RuntimeProcessError> {
        self.cli
            .execute(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::DestroySandbox,
                    sandbox_destroy_argv(&self.config, sandbox_id),
                    output_limit,
                ),
                REMOTE_CLEANUP_TIMEOUT,
            )
            .await
            .map(|_| ())
    }

    /// Best-effort: pull the egress audit tail out of a sandbox that is about
    /// to be destroyed after a transport failure, and record it in host logs
    /// (the only durable location once the outer sandbox is gone). Salvage
    /// failure never blocks destruction — the sandbox is already suspect.
    async fn salvage_proxy_audit(&self, sandbox_id: &str, output_limit: usize) {
        const SALVAGE_SCRIPT: &str = "docker logs --timestamps ironclaw-reborn-proxy 2>&1 | tail -c 65536; for audit_file in /run/ironclaw-reborn-proxy/audit/proxy.log.* /run/ironclaw-reborn-proxy/audit/proxy.log; do [ ! -f \"$audit_file\" ] || tail -c 65536 \"$audit_file\"; done";
        let salvage = self
            .cli
            .execute(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ExecuteSandboxCommand,
                    sandbox_exec_argv(
                        &self.config,
                        sandbox_id,
                        REMOTE_CLEANUP_TIMEOUT,
                        vec!["sh".into(), "-c".into(), SALVAGE_SCRIPT.into()],
                    ),
                    output_limit,
                ),
                REMOTE_CLEANUP_TIMEOUT,
            )
            .await;
        match salvage {
            Ok(output) => tracing::warn!(
                sandbox_id,
                audit_tail = %output.stdout,
                "Railway sandbox egress audit salvaged before destruction"
            ),
            Err(error) => tracing::warn!(
                ?error,
                sandbox_id,
                "Railway sandbox egress audit salvage failed before destruction"
            ),
        }
    }

    async fn shutdown_owned_sandboxes(&self) -> Result<(), RuntimeProcessError> {
        let users = {
            let users = self.users.lock().await;
            users
                .iter()
                .map(|(key, tracked)| (key.clone(), tracked.state.clone()))
                .collect::<Vec<_>>()
        };
        let mut first_error = None;
        for (key, state) in users {
            let mut state = state.lock().await;
            let sandbox_id = match state.lifecycle.clone() {
                UserSandboxLifecycle::Absent => continue,
                UserSandboxLifecycle::Live(sandbox_id) => {
                    if !state.checkpoint_current {
                        if let Err(error) = self
                            .checkpoint(&key, &sandbox_id, DEFAULT_OUTPUT_LIMIT)
                            .await
                        {
                            tracing::error!(
                                ?error,
                                sandbox_id,
                                "Railway sandbox shutdown left a live sandbox because its final checkpoint failed"
                            );
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                            continue;
                        }
                        state.checkpoint_current = true;
                    }
                    sandbox_id
                }
                UserSandboxLifecycle::CleanupPending(sandbox_id) => sandbox_id,
            };
            state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
            match self
                .destroy_sandbox(&sandbox_id, DEFAULT_OUTPUT_LIMIT)
                .await
            {
                Ok(()) => state.lifecycle = UserSandboxLifecycle::Absent,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        sandbox_id,
                        "Railway sandbox shutdown could not confirm remote cleanup"
                    );
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn execute_cli(
        &self,
        invocation: RailwayCliInvocation,
        deadline: Instant,
    ) -> Result<RailwayCliOutput, RuntimeProcessError> {
        self.cli
            .execute(invocation, deadline_remaining(deadline)?)
            .await
    }
}

#[async_trait]
impl SandboxCommandTransport for RailwayPreviewSandboxTransport {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        reject_request_environment(&request)?;
        reject_nul("command", &request.command)?;
        let workdir = validated_workdir(request.workdir.as_deref())?;
        let timeout = request_timeout(request.timeout_secs);
        let output_limit = DEFAULT_OUTPUT_LIMIT;
        let started = Instant::now();
        let deadline = started + timeout;
        let key = RebornSandboxUserKey::from_scope(&request.scope);
        let state = self.state_for(key.clone()).await?;

        // Per-user lifecycle gate: first provision, worker bootstrap, and all
        // use of that worker serialize without blocking other tenant/users.
        let mut state = state.lock().await;
        if let UserSandboxLifecycle::CleanupPending(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            let prior_error = RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox cleanup was previously unconfirmed".to_string(),
            );
            self.destroy_after_failed_execution(&sandbox_id, output_limit, &prior_error)
                .await?;
            state.lifecycle = UserSandboxLifecycle::Absent;
        }
        if let UserSandboxLifecycle::Live(sandbox_id) = &state.lifecycle {
            let sandbox_id = sandbox_id.clone();
            if !self.sandbox_is_live(&sandbox_id, deadline).await? {
                state.lifecycle = UserSandboxLifecycle::Absent;
            }
        }
        let sandbox_id = match &state.lifecycle {
            UserSandboxLifecycle::Live(id) => id.clone(),
            UserSandboxLifecycle::Absent => {
                self.provision(&key, &mut state, deadline, output_limit)
                    .await?
            }
            UserSandboxLifecycle::CleanupPending(_) => {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "Railway preview sandbox cleanup remains pending".to_string(),
                ));
            }
        };
        // The remote command may mutate `/workspace` even when its transport
        // response is lost or malformed. Mark the prior checkpoint stale
        // before dispatch so graceful shutdown cannot destroy those writes
        // without first retrying a checkpoint.
        state.checkpoint_current = false;
        let output = self
            .execute_cli(
                RailwayCliInvocation::new(
                    self.config.cli_path.clone(),
                    RailwayCliOperation::ExecuteSandboxCommand,
                    sandbox_exec_argv(
                        &self.config,
                        &sandbox_id,
                        deadline_remaining(deadline)?,
                        ephemeral_worker_argv(
                            &self.config,
                            &railway_workspace_path(&key),
                            &workdir,
                            &request.command,
                            &request.scope.invocation_id,
                        ),
                    ),
                    output_limit,
                ),
                deadline,
            )
            .await;
        let output = match output {
            Ok(output) => output,
            Err(execution_error) => {
                // Killing a local Railway CLI process does not prove that its
                // remote command stopped. Destroy the whole outer sandbox on
                // any worker-exec transport failure, then force reprovisioning
                // from the last durable checkpoint on the next request.
                state.lifecycle = UserSandboxLifecycle::CleanupPending(sandbox_id.clone());
                self.destroy_after_failed_execution(&sandbox_id, output_limit, &execution_error)
                    .await?;
                state.lifecycle = UserSandboxLifecycle::Absent;
                return Err(execution_error);
            }
        };
        let (stdout, exit_code) = parse_exit_sentinel(output.stdout)?;
        let checkpoint_failed = self
            .checkpoint(&key, &sandbox_id, output_limit)
            .await
            .is_err();
        state.checkpoint_current = !checkpoint_failed;
        let mut raw_output = combine_output(stdout, output.stderr);
        if checkpoint_failed {
            if !raw_output.is_empty() && !raw_output.ends_with('\n') {
                raw_output.push('\n');
            }
            raw_output.push_str(CHECKPOINT_FAILURE_WARNING);
            raw_output.push('\n');
        }
        let output = redact_command_output(raw_output);
        Ok(CommandExecutionOutput {
            output,
            saved_output: None,
            exit_code: i64::from(exit_code),
            sandboxed: true,
            duration: started.elapsed(),
        })
    }

    async fn shutdown(&self) -> Result<(), RuntimeProcessError> {
        self.shutdown_owned_sandboxes().await
    }
}

#[cfg(test)]
mod constructor_test_support {
    use super::*;

    impl RailwayPreviewSandboxTransport {
        pub(super) fn with_cli(
            config: RailwayPreviewSandboxConfig,
            cli: Arc<dyn RailwayCli>,
        ) -> Self {
            Self::with_cli_and_capacity(config, cli, MAX_TRACKED_USERS)
        }

        pub(super) fn with_cli_and_capacity(
            config: RailwayPreviewSandboxConfig,
            cli: Arc<dyn RailwayCli>,
            max_tracked_users: usize,
        ) -> Self {
            Self {
                config,
                cli,
                users: Arc::new(Mutex::new(HashMap::new())),
                max_tracked_users,
            }
        }
    }
}

#[derive(Debug)]
struct TrackedUserState {
    state: Arc<Mutex<UserSandboxState>>,
    last_touched: Instant,
}

#[derive(Debug, Default)]
struct UserSandboxState {
    lifecycle: UserSandboxLifecycle,
    checkpoint_current: bool,
}

#[derive(Debug, Clone, Default)]
enum UserSandboxLifecycle {
    #[default]
    Absent,
    Live(String),
    CleanupPending(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RailwayCliInvocation {
    program: PathBuf,
    operation: RailwayCliOperation,
    args: Vec<String>,
    output_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailwayCliOperation {
    ListCheckpoints,
    ListSandboxes,
    CreateSandbox,
    ExecuteSandboxCommand,
    CreateCheckpoint,
    DestroySandbox,
}

impl RailwayCliOperation {
    fn label(self) -> &'static str {
        match self {
            Self::ListCheckpoints => "list checkpoints",
            Self::ListSandboxes => "list sandboxes",
            Self::CreateSandbox => "create sandbox",
            Self::ExecuteSandboxCommand => "execute sandbox command",
            Self::CreateCheckpoint => "create checkpoint",
            Self::DestroySandbox => "destroy sandbox",
        }
    }
}

impl RailwayCliInvocation {
    fn new(
        program: PathBuf,
        operation: RailwayCliOperation,
        args: Vec<String>,
        output_limit: usize,
    ) -> Self {
        // Leave enough headroom for the fixed remote exit sentinel even when
        // the model command emits exactly its requested output cap.
        Self {
            program,
            operation,
            args,
            output_limit: output_limit.saturating_add(EXIT_SENTINEL.len() + 32),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RailwayCliOutput {
    stdout: String,
    stderr: String,
}

#[async_trait]
trait RailwayCli: Send + Sync {
    async fn execute(
        &self,
        invocation: RailwayCliInvocation,
        timeout: Duration,
    ) -> Result<RailwayCliOutput, RuntimeProcessError>;
}

#[derive(Debug)]
struct SystemRailwayCli;

#[derive(Debug)]
struct PrivateRailwayCliHome {
    path: Option<PathBuf>,
}

impl PrivateRailwayCliHome {
    async fn create() -> Result<Self, RuntimeProcessError> {
        tokio::task::spawn_blocking(Self::create_blocking)
            .await
            .map_err(|error| {
                tracing::error!(?error, "Railway CLI private state setup task failed");
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not create private CLI state".to_string(),
                )
            })?
    }

    fn create_blocking() -> Result<Self, RuntimeProcessError> {
        let path =
            std::env::temp_dir().join(format!("ironclaw-railway-cli-{}", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700).create(&path).map_err(|error| {
                tracing::error!(
                    ?error,
                    "Railway CLI private state directory creation failed"
                );
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not create private CLI state".to_string(),
                )
            })?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    tracing::error!(?error, "Railway CLI private state permission update failed");
                    let _ = std::fs::remove_dir_all(&path);
                    RuntimeProcessError::ExecutionFailed(
                        "Railway preview could not secure private CLI state".to_string(),
                    )
                },
            )?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir(&path).map_err(|error| {
            tracing::error!(
                ?error,
                "Railway CLI private state directory creation failed"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview could not create private CLI state".to_string(),
            )
        })?;
        Ok(Self { path: Some(path) })
    }

    fn path(&self) -> Result<&Path, RuntimeProcessError> {
        self.path.as_deref().ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview private CLI state is unavailable".to_string(),
            )
        })
    }

    async fn cleanup(mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        match tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup failed"
            ),
            Err(error) => tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup task failed"
            ),
        }
    }
}

impl Drop for PrivateRailwayCliHome {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if let Err(error) = std::fs::remove_dir_all(path) {
            tracing::debug!(
                ?error,
                "best-effort Railway CLI private state cleanup failed"
            );
        }
    }
}

#[async_trait]
impl RailwayCli for SystemRailwayCli {
    async fn execute(
        &self,
        invocation: RailwayCliInvocation,
        timeout: Duration,
    ) -> Result<RailwayCliOutput, RuntimeProcessError> {
        let environment = railway_cli_environment()?;
        let cli_home = PrivateRailwayCliHome::create().await?;
        let operation = invocation.operation;
        let mut command = Command::new(&invocation.program);
        command
            .args(&invocation.args)
            .env_clear()
            .envs(&environment)
            .env("HOME", cli_home.path()?)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                tracing::error!(?error, "trusted Railway CLI process could not be started");
                cli_home.cleanup().await;
                return Err(RuntimeProcessError::ExecutionFailed(
                    "Railway preview could not start the trusted Railway CLI".to_string(),
                ));
            }
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let result = tokio::time::timeout(timeout, async {
            let stdout = async {
                match stdout {
                    Some(stream) => read_stream_bounded(stream, invocation.output_limit).await,
                    None => Ok(String::new()),
                }
            };
            let stderr = async {
                match stderr {
                    Some(stream) => read_stream_bounded(stream, invocation.output_limit).await,
                    None => Ok(String::new()),
                }
            };
            let (stdout, stderr, status) = tokio::join!(stdout, stderr, child.wait());
            let status = status.map_err(|error| {
                tracing::error!(?error, "trusted Railway CLI process wait failed");
                RuntimeProcessError::ExecutionFailed(
                    "Railway preview CLI command did not complete".to_string(),
                )
            })?;
            let stdout = stdout?;
            let stderr = stderr?;
            if !status.success() {
                return Err(railway_cli_status_error(
                    operation,
                    status.code(),
                    &stderr,
                    &environment,
                    timeout,
                ));
            }
            Ok(RailwayCliOutput { stdout, stderr })
        })
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                if let Err(error) = child.kill().await {
                    tracing::debug!(?error, "best-effort Railway CLI termination failed");
                }
                if let Err(error) = child.wait().await {
                    tracing::debug!(?error, "best-effort Railway CLI reap failed");
                }
                Err(RuntimeProcessError::Timeout(timeout))
            }
        };
        cli_home.cleanup().await;
        result
    }
}

fn railway_cli_status_error(
    operation: RailwayCliOperation,
    exit_code: Option<i32>,
    stderr: &str,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
) -> RuntimeProcessError {
    if operation == RailwayCliOperation::ExecuteSandboxCommand && exit_code == Some(124) {
        return RuntimeProcessError::Timeout(timeout);
    }
    RuntimeProcessError::ExecutionFailed(railway_cli_failure_message(
        operation,
        exit_code,
        stderr,
        environment,
    ))
}

fn railway_cli_failure_message(
    operation: RailwayCliOperation,
    exit_code: Option<i32>,
    stderr: &str,
    environment: &BTreeMap<String, String>,
) -> String {
    let mut detail = stderr.to_string();
    for name in ["RAILWAY_TOKEN", "RAILWAY_API_TOKEN"] {
        if let Some(token) = environment.get(name).filter(|token| !token.is_empty()) {
            detail = detail.replace(token, "[REDACTED]");
        }
    }
    let (detail, _) = ironclaw_safety::LeakDetector::new().redact_all_secrets(&detail);
    let exit = exit_code
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| "terminated by signal".to_string());
    let detail = detail.trim();
    if detail.is_empty() {
        format!("Railway preview failed to {} ({exit})", operation.label())
    } else {
        format!(
            "Railway preview failed to {} ({exit}): {detail}",
            operation.label()
        )
    }
}

fn required_config_value(label: &str, value: String) -> Result<String, RuntimeProcessError> {
    if value.trim().is_empty() || value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "Railway preview configuration requires a valid {label}"
        )));
    }
    Ok(value)
}

fn checkpoint_list_argv(config: &RailwayPreviewSandboxConfig) -> Vec<String> {
    vec![
        "sandbox".into(),
        "checkpoint".into(),
        "list".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_list_argv(config: &RailwayPreviewSandboxConfig) -> Vec<String> {
    vec![
        "sandbox".into(),
        "list".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_create_argv(
    config: &RailwayPreviewSandboxConfig,
    checkpoint: Option<&str>,
) -> Vec<String> {
    let mut argv = vec![
        "sandbox".into(),
        "create".into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
        "--idle-timeout-minutes".into(),
        config.idle_timeout_minutes.to_string(),
        "--private-network".into(),
    ];
    if let Some(checkpoint) = checkpoint {
        argv.push("--checkpoint".into());
        argv.push(checkpoint.into());
    }
    argv
}

fn checkpoint_create_argv(
    config: &RailwayPreviewSandboxConfig,
    sandbox_id: &str,
    checkpoint_name: &str,
) -> Vec<String> {
    vec![
        "sandbox".into(),
        "checkpoint".into(),
        "create".into(),
        checkpoint_name.into(),
        "--id".into(),
        sandbox_id.into(),
        "--json".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_destroy_argv(config: &RailwayPreviewSandboxConfig, sandbox_id: &str) -> Vec<String> {
    vec![
        "sandbox".into(),
        "destroy".into(),
        "--id".into(),
        sandbox_id.into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
    ]
}

fn sandbox_exec_argv(
    config: &RailwayPreviewSandboxConfig,
    sandbox_id: &str,
    timeout: Duration,
    command: Vec<String>,
) -> Vec<String> {
    let mut args = vec![
        "sandbox".into(),
        "exec".into(),
        "--project".into(),
        config.project_id.clone(),
        "--environment".into(),
        config.environment_id.clone(),
        "--id".into(),
        sandbox_id.into(),
        "--timeout".into(),
        timeout.as_secs().max(1).to_string(),
        "--".into(),
    ];
    args.extend(command);
    args
}

fn workspace_bootstrap_argv(workspace: &str) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!("mkdir -p \"$1\" && chown {WORKER_USER} \"$1\""),
        WORKSPACE_BOOTSTRAP_MARKER.into(),
        workspace.into(),
    ]
}

fn ephemeral_worker_argv(
    config: &RailwayPreviewSandboxConfig,
    railway_workspace: &str,
    workdir: &str,
    model_command: &str,
    invocation_id: &ironclaw_host_api::ids::InvocationId,
) -> Vec<String> {
    let mut worker = vec!["--rm".into()];
    worker.extend(super::worker_spec::DockerWorkerSecuritySpec::new(None).docker_run_args());
    worker.extend([
        "--mount".into(),
        format!("type=bind,src={railway_workspace},dst={WORKSPACE_ROOT}"),
        "--workdir".into(),
        workdir.into(),
        "--memory".into(),
        "512m".into(),
        config.worker_image.clone(),
        "sh".into(),
        "-c".into(),
        model_command.into(),
    ]);
    let mut args = vec![
        "sh".into(),
        "-c".into(),
        OUTER_EXEC_WRAPPER.into(),
        "ironclaw-railway-wrapper".into(),
        "sh".into(),
        "-c".into(),
        RAILWAY_MANAGED_EGRESS_WRAPPER.into(),
        "ironclaw-railway-egress".into(),
        config.proxy_image.clone(),
        config.proxy_posture.clone(),
        config.proxy_config_template.clone(),
        RAILWAY_PROXY_BOOTSTRAP_CONFIG.into(),
        invocation_id.to_string(),
        config.worker_image.clone(),
    ];
    args.extend(worker);
    args
}

fn railway_workspace_path(key: &RebornSandboxUserKey) -> String {
    format!("{RAILWAY_WORKSPACES_ROOT}/{}", key.container_name())
}

fn checkpoint_name(key: &RebornSandboxUserKey) -> String {
    format!("{}-checkpoint", key.container_name())
}

fn request_timeout(requested: Option<u64>) -> Duration {
    requested
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT)
        .min(MAX_COMMAND_TIMEOUT)
}

fn deadline_remaining(deadline: Instant) -> Result<Duration, RuntimeProcessError> {
    deadline
        .checked_duration_since(Instant::now())
        .ok_or(RuntimeProcessError::Timeout(Duration::ZERO))
}

fn reject_request_environment(
    request: &CommandExecutionRequest,
) -> Result<(), RuntimeProcessError> {
    // Mount views are host-side authorization metadata. This remote-only
    // preview deliberately materializes none of them; ignoring the metadata
    // grants the worker less access, while accepting the `Some(view)` shape
    // used by the production shell capability.
    if !request.extra_env.is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox does not support request environment variables".into(),
        ));
    }
    Ok(())
}

fn validated_workdir(workdir: Option<&str>) -> Result<String, RuntimeProcessError> {
    let Some(workdir) = workdir else {
        return Ok(WORKSPACE_ROOT.into());
    };
    reject_nul("working directory", workdir)?;
    if workdir == WORKSPACE_ROOT {
        return Ok(WORKSPACE_ROOT.into());
    }
    let Some(relative) = workdir.strip_prefix("/workspace/") else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox working directory must be /workspace or a descendant".into(),
        ));
    };
    for component in Path::new(relative).components() {
        if !matches!(component, Component::Normal(_) | Component::CurDir) {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox working directory must stay under /workspace".into(),
            ));
        }
    }
    Ok(workdir.into())
}

fn reject_nul(label: &str, value: &str) -> Result<(), RuntimeProcessError> {
    if value.as_bytes().contains(&0) {
        return Err(RuntimeProcessError::ExecutionFailed(format!(
            "Railway preview sandbox {label} contains null bytes"
        )));
    }
    Ok(())
}

fn parse_sandbox_id(stdout: &str) -> Result<String, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview sandbox creation returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox creation returned an invalid response".into(),
        )
    })?;
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .get("sandbox")
                .and_then(|sandbox| sandbox.get("id"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|id| !id.trim().is_empty() && !id.as_bytes().contains(&0))
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox creation returned no sandbox identifier".into(),
            )
        })?;
    Ok(id.into())
}

fn checkpoint_exists(stdout: &str, checkpoint_name: &str) -> Result<bool, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview checkpoint listing returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview checkpoint listing returned an invalid response".into(),
        )
    })?;
    let checkpoints = value
        .as_array()
        .or_else(|| {
            value
                .get("checkpoints")
                .and_then(serde_json::Value::as_array)
        })
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview checkpoint listing returned an invalid response".into(),
            )
        })?;
    Ok(checkpoints.iter().any(|checkpoint| {
        checkpoint
            .get("key")
            .or_else(|| checkpoint.get("name"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name == checkpoint_name)
    }))
}

fn sandbox_exists(stdout: &str, sandbox_id: &str) -> Result<bool, RuntimeProcessError> {
    let value: serde_json::Value = serde_json::from_str(stdout).map_err(|error| {
        tracing::error!(
            ?error,
            "Railway preview sandbox listing returned invalid JSON"
        );
        RuntimeProcessError::ExecutionFailed(
            "Railway preview sandbox listing returned an invalid response".into(),
        )
    })?;
    let sandboxes = value
        .as_array()
        .or_else(|| value.get("sandboxes").and_then(serde_json::Value::as_array))
        .ok_or_else(|| {
            RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox listing returned an invalid response".into(),
            )
        })?;
    Ok(sandboxes.iter().any(|sandbox| {
        sandbox
            .get("id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                sandbox
                    .get("sandbox")
                    .and_then(|sandbox| sandbox.get("id"))
                    .and_then(serde_json::Value::as_str)
            })
            .is_some_and(|id| id == sandbox_id)
    }))
}

fn parse_exit_sentinel(stdout: String) -> Result<(String, i32), RuntimeProcessError> {
    let Some(marker_start) = stdout.rfind(EXIT_SENTINEL) else {
        return Err(RuntimeProcessError::ExecutionFailed(
            "Railway preview command completed without a verified exit status".into(),
        ));
    };
    let status_start = marker_start + EXIT_SENTINEL.len();
    let status_end = stdout[status_start..]
        .find('\n')
        .map(|offset| status_start + offset)
        .unwrap_or(stdout.len());
    let code = stdout[status_start..status_end]
        .parse::<i32>()
        .map_err(|error| {
            tracing::error!(
                ?error,
                "Railway preview command returned invalid exit status"
            );
            RuntimeProcessError::ExecutionFailed(
                "Railway preview command returned an invalid exit status".into(),
            )
        })?;
    let line_start = stdout[..marker_start] // safety: `str::rfind` returns a UTF-8 boundary.
        .rfind('\n')
        .map(|offset| offset + 1)
        .unwrap_or(marker_start);
    let line_end = if status_end < stdout.len() {
        status_end + 1
    } else {
        status_end
    };
    let mut output = stdout;
    output.replace_range(line_start..line_end, "");
    Ok((output, code))
}

fn combine_output(stdout: String, stderr: String) -> String {
    if stdout.is_empty() || stderr.is_empty() {
        format!("{stdout}{stderr}")
    } else {
        format!("{stdout}\n\n--- stderr ---\n{stderr}")
    }
}

fn redact_command_output(output: String) -> String {
    match ironclaw_safety::LeakDetector::new().scan_and_clean(&output) {
        Ok(redacted) => redacted,
        Err(error) => {
            tracing::error!(
                ?error,
                "Railway preview command output failed secret scanning"
            );
            "Railway preview command output was blocked because it may contain secret material."
                .to_string()
        }
    }
}

/// This cannot use `process_output::read_stream_capped`: sentinel parsing
/// needs raw stdout before model-output sanitization. It remains bounded and
/// drains beyond the cap so verbose CLI output cannot block the child.
async fn read_stream_bounded<R>(mut stream: R, limit: usize) -> Result<String, RuntimeProcessError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut captured = Vec::with_capacity(limit.min(16 * 1024));
    let head_limit = limit / 2;
    let tail_limit = limit.saturating_sub(head_limit);
    let mut tail = Vec::with_capacity(tail_limit.min(16 * 1024));
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = stream.read(&mut buffer).await.map_err(|error| {
            tracing::error!(?error, "trusted Railway CLI output read failed");
            RuntimeProcessError::ExecutionFailed(
                "Railway preview could not read trusted CLI output".into(),
            )
        })?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read]; // safety: `read` is at most the byte-buffer length.
        if !truncated && captured.len().saturating_add(read) <= limit {
            captured.extend_from_slice(chunk);
            continue;
        }
        if !truncated {
            truncated = true;
            if captured.len() > head_limit {
                tail.extend_from_slice(&captured[head_limit..]);
                captured.truncate(head_limit);
            }
        }
        tail.extend_from_slice(chunk);
        if tail.len() > tail_limit {
            tail.drain(..tail.len() - tail_limit);
        }
    }
    if !truncated {
        return Ok(String::from_utf8_lossy(&captured).into_owned());
    }
    Ok(format!(
        "{}\n\n... [trusted CLI output truncated] ...\n\n{}",
        String::from_utf8_lossy(&captured),
        String::from_utf8_lossy(&tail)
    ))
}

fn railway_cli_environment() -> Result<BTreeMap<String, String>, RuntimeProcessError> {
    railway_cli_environment_from(std::env::vars())
}

fn railway_cli_environment_from<I>(
    variables: I,
) -> Result<BTreeMap<String, String>, RuntimeProcessError>
where
    I: IntoIterator<Item = (String, String)>,
{
    let variables: BTreeMap<_, _> = variables.into_iter().collect();
    let mut allowed = BTreeMap::new();
    for name in [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "CURL_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "NODE_EXTRA_CA_CERTS",
    ] {
        if let Some(value) = variables.get(name).filter(|value| !value.is_empty()) {
            allowed.insert(name.into(), value.clone());
        }
    }
    let project_token = variables
        .get("RAILWAY_TOKEN")
        .filter(|value| !value.is_empty());
    let api_token = variables
        .get("RAILWAY_API_TOKEN")
        .filter(|value| !value.is_empty());
    let auth = match (project_token, api_token) {
        (Some(value), None) => ("RAILWAY_TOKEN", value),
        (None, Some(value)) => ("RAILWAY_API_TOKEN", value),
        (None, None) => {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox requires Railway CLI authentication".into(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(RuntimeProcessError::ExecutionFailed(
                "Railway preview sandbox requires exactly one Railway token variable".into(),
            ));
        }
    };
    allowed.insert(auth.0.into(), auth.1.clone());
    Ok(allowed)
}

#[cfg(test)]
mod tests;
