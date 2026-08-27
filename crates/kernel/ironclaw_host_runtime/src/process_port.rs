//! Runtime process effect port for command-style first-party capabilities.
//!
//! The port keeps process placement outside individual tools. A capability such
//! as `builtin.shell` describes the command to run; host-runtime composition
//! decides which port implementation receives it. This first slice wires the
//! existing local-host behavior behind an explicit port without changing
//! placement semantics.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{
    action::NetworkScheme,
    http::RuntimeCredentialTarget,
    process::{
        CommandExecutionOutput, CommandExecutionRequest, CredentialedSandboxCommandRequest,
        RuntimeProcessError, SandboxCommandCredential, SandboxCommandCredentialBinding,
        SandboxCommandTransport,
    },
    resource::ResourceScope,
};
#[cfg(unix)]
use libc::{SIGKILL, kill};
use secrecy::ExposeSecret;
use tokio::process::Command;

use crate::process_aliases::{
    HostWorkdirAlias, resolve_local_host_workdir, rewrite_local_host_command_aliases,
    rewrite_local_host_output_aliases,
};
use crate::process_output::{
    CapturedCommandOutput, StreamCapture, capture_command_output, read_stream_capped,
    truncate_output,
};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Environment variables safe to forward to local child processes.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "PWD",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "EDITOR",
    "VISUAL",
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

/// Abstract process effect used by process-backed capabilities.
#[async_trait]
pub trait RuntimeProcessPort: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;

    async fn run_credentialed_command(
        &self,
        _request: CredentialedSandboxCommandRequest,
        _credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        Err(RuntimeProcessError::ExecutionFailed(
            "process port does not support credentialed shell execution".to_string(),
        ))
    }

    fn supports_credentialed_command(&self) -> bool {
        false
    }
}

/// Tenant-isolated process port backed by a sandbox command transport.
#[derive(Clone)]
pub struct UserSandboxProcessPort {
    transport: std::sync::Arc<dyn SandboxCommandTransport>,
}

impl std::fmt::Debug for UserSandboxProcessPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserSandboxProcessPort")
            .field("transport", &"<sandbox command transport>")
            .finish()
    }
}

impl UserSandboxProcessPort {
    pub fn new(transport: std::sync::Arc<dyn SandboxCommandTransport>) -> Self {
        Self { transport }
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeProcessError> {
        self.transport.shutdown().await
    }
}

#[async_trait]
impl RuntimeProcessPort for UserSandboxProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        let mut request = request;
        request.timeout_secs = Some(timeout.as_secs());
        let mut output = self.transport.run_command(request).await?;
        output.output = truncate_output(&output.output);
        output.sandboxed = true;
        Ok(output)
    }

    async fn run_credentialed_command(
        &self,
        mut request: CredentialedSandboxCommandRequest,
        credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        request.timeout_secs = Some(timeout.as_secs());
        let mut output = self
            .transport
            .run_credentialed_command(request, credentials)
            .await?;
        output.output = truncate_output(&output.output);
        output.sandboxed = true;
        Ok(output)
    }

    fn supports_credentialed_command(&self) -> bool {
        self.transport.supports_credentialed_command()
    }
}

/// Consumes the one-shot credentials staged by the normal capability
/// obligation path and converts authorized bindings to sandbox placeholders.
///
/// Credential selection remains with the capability adapter. This port only
/// materializes the exact descriptor requirements carried by the authorized
/// shell request; it never discovers credentials from the secret store.
#[derive(Clone)]
pub(crate) struct StagedCredentialProcessPort {
    inner: Arc<dyn RuntimeProcessPort>,
    secret_injection_store: Arc<crate::obligations::RuntimeSecretInjectionStore>,
}

impl StagedCredentialProcessPort {
    pub(crate) fn new(
        inner: Arc<dyn RuntimeProcessPort>,
        secret_injection_store: Arc<crate::obligations::RuntimeSecretInjectionStore>,
    ) -> Self {
        Self {
            inner,
            secret_injection_store,
        }
    }
}

#[async_trait]
impl RuntimeProcessPort for StagedCredentialProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        self.inner.run_command(request).await
    }

    fn supports_credentialed_command(&self) -> bool {
        self.inner.supports_credentialed_command()
    }

    async fn run_credentialed_command(
        &self,
        mut request: CredentialedSandboxCommandRequest,
        credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        if !credentials.is_empty() {
            return Err(RuntimeProcessError::ExecutionFailed(
                "caller-supplied sandbox credentials are forbidden".to_string(),
            ));
        }
        let bindings = std::mem::take(&mut request.credential_bindings);
        validate_credential_bindings(&request, &bindings)?;
        let handles = bindings
            .iter()
            .map(|binding| binding.requirement.handle.clone())
            .collect::<Vec<_>>();
        let materials = self
            .secret_injection_store
            .take_many(&request.scope, &request.capability_id, &handles)
            .map_err(|error| {
                tracing::debug!(?error, "sandbox credential staging lookup failed");
                RuntimeProcessError::ExecutionFailed(
                    "sandbox credential staging is unavailable".to_string(),
                )
            })?
            .ok_or_else(|| {
                RuntimeProcessError::ExecutionFailed(
                    "an authorized sandbox credential was not staged for this invocation"
                        .to_string(),
                )
            })?;
        let mut credentials = Vec::with_capacity(bindings.len());
        for (binding, material) in bindings.into_iter().zip(materials) {
            let RuntimeCredentialTarget::Header { name, prefix } = binding.requirement.target
            else {
                return Err(RuntimeProcessError::ExecutionFailed(
                    "sandbox process credentials require a header injection target".to_string(),
                ));
            };
            let placeholder = format!(
                "{}{}",
                ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX,
                uuid::Uuid::new_v4().simple()
            );
            request
                .extra_env
                .insert(binding.placeholder_env.clone(), placeholder.clone());
            credentials.push(SandboxCommandCredential::new(
                binding.requirement.handle,
                binding.placeholder_env,
                placeholder,
                binding.requirement.audience.host_pattern,
                name,
                prefix,
                material.expose_secret().to_string(),
            ));
        }
        self.inner
            .run_credentialed_command(request, credentials)
            .await
    }
}

fn validate_credential_bindings(
    request: &CredentialedSandboxCommandRequest,
    bindings: &[SandboxCommandCredentialBinding],
) -> Result<(), RuntimeProcessError> {
    let mut handles = HashSet::with_capacity(bindings.len());
    let mut env_names = HashSet::with_capacity(bindings.len());
    for binding in bindings {
        let requirement = &binding.requirement;
        let RuntimeCredentialTarget::Header { name, prefix } = &requirement.target else {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox process credentials require a header injection target".to_string(),
            ));
        };
        if requirement.audience.scheme != Some(NetworkScheme::Https)
            || requirement.audience.port.is_some()
            || requirement.audience.host_pattern.contains('*')
            || requirement.audience.host_pattern.is_empty()
            || requirement
                .audience
                .host_pattern
                .chars()
                .any(char::is_control)
            || name.is_empty()
            || name.chars().any(char::is_control)
            || prefix
                .as_ref()
                .is_some_and(|value| value.chars().any(char::is_control))
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox process credential binding is not an exact HTTPS header target"
                    .to_string(),
            ));
        }
        if requirement.placeholder_env.as_deref() != Some(binding.placeholder_env.as_str()) {
            return Err(RuntimeProcessError::ExecutionFailed(
                "authorized shell credential does not match its placeholder environment"
                    .to_string(),
            ));
        }
        if !ironclaw_host_api::process::is_valid_sandbox_credential_env_name(
            &binding.placeholder_env,
        ) || request.extra_env.contains_key(&binding.placeholder_env)
            || !handles.insert(&requirement.handle)
            || !env_names.insert(&binding.placeholder_env)
        {
            return Err(RuntimeProcessError::ExecutionFailed(
                "sandbox process credential binding is ambiguous".to_string(),
            ));
        }
    }
    Ok(())
}

/// Host-process command environment handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum HostProcessEnvMode {
    /// Clear the child environment, forward only `SAFE_ENV_VARS`, and rewrite
    /// `HOME` to the command workdir.
    #[default]
    Scrubbed,
    /// Inherit the host process environment and real `HOME`.
    Inherited,
}

/// The host-process port: runs an OS process **directly on the host**, unsandboxed
/// (no tenant-scoped mount containment). The name states the trust boundary plainly
/// — renamed from `LocalHostProcessPort` because `Host` describes the boundary
/// (host process vs sandboxed process) where `Local` obscured it. Composition
/// must select this only for a genuinely single-user/local deployment; a multi-user
/// served boot must route to the sandboxed process port instead (§6, issue #6170).
#[derive(Debug, Clone, Default)]
pub struct HostProcessPort {
    env_mode: HostProcessEnvMode,
    workdir_aliases: Vec<HostWorkdirAlias>,
    /// Whether `/workspace` is scoped to the caller, as the file tools scope it.
    ///
    /// The alias list is built once at composition and knows nothing about callers, while
    /// `scoped_workspace_mount_view` resolves `/workspace` to `<root>/tenants/<t>/users/<u>`. With this
    /// false under a per-caller policy, one alias means two different directories in one process: an
    /// agent wrote `scripts/egfr.py` through `write_file`, ran `python3 scripts/egfr.py` in the shell,
    /// and landed three directories above its own file with no error from either side.
    workspace_scoped_per_caller: bool,
}

impl HostProcessPort {
    pub fn new() -> Self {
        Self {
            env_mode: HostProcessEnvMode::Scrubbed,
            workdir_aliases: Vec::new(),
            workspace_scoped_per_caller: false,
        }
    }

    pub fn new_inherited_env() -> Self {
        Self {
            env_mode: HostProcessEnvMode::Inherited,
            workdir_aliases: Vec::new(),
            workspace_scoped_per_caller: false,
        }
    }

    /// Apply the same per-caller workspace scoping the file tools use.
    ///
    /// Composition knows the policy (`workspace_scoped_per_caller`); the alias list cannot, because it
    /// is built before any caller exists. Setting this makes the port derive the caller's subtree from
    /// the scope carried on every request, so `/workspace` resolves to one directory for both worlds.
    pub fn with_workspace_scoped_per_caller(mut self, scoped: bool) -> Self {
        self.workspace_scoped_per_caller = scoped;
        self
    }

    /// The alias list as it applies to ONE request, with per-caller scoping resolved.
    fn aliases_for(
        &self,
        scope: &ironclaw_host_api::resource::ResourceScope,
    ) -> Vec<HostWorkdirAlias> {
        if !self.workspace_scoped_per_caller {
            return self.workdir_aliases.clone();
        }
        self.workdir_aliases
            .iter()
            .map(|alias| {
                alias
                    .scoped_to_caller(scope)
                    .unwrap_or_else(|| alias.clone())
            })
            .collect()
    }

    pub fn with_workdir_alias(
        mut self,
        alias: impl Into<String>,
        host_path: impl Into<PathBuf>,
    ) -> Self {
        match HostWorkdirAlias::try_new(alias, host_path) {
            Ok(alias) => self.workdir_aliases.push(alias),
            Err(reason) => tracing::debug!(
                reason = %reason,
                "ignoring invalid local host process workdir alias"
            ),
        }
        self
    }
}

#[async_trait]
impl RuntimeProcessPort for HostProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let aliases = self.aliases_for(&request.scope);
        let cwd =
            resolve_local_host_workdir(request.workdir.as_deref(), &aliases).map_err(|e| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "cannot determine working directory: {e}"
                ))
            })?;
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        if self.env_mode == HostProcessEnvMode::Inherited {
            tracing::warn!(
                host_access = "full-local",
                "running local host command with inherited environment"
            );
        }
        let command = rewrite_local_host_command_aliases(&request.command, &aliases);
        let start = std::time::Instant::now();
        let (output, exit_code) = execute_local_command(
            &request.scope,
            &command,
            &cwd,
            timeout,
            &request.extra_env,
            self.env_mode,
        )
        .await?;
        // The command was rewritten alias->host before execution, so any host
        // path the program echoed back is now in the captured output. Map it
        // back to the virtual alias so the model-facing preview speaks in
        // `/workspace` terms and never leaks the host layout into the reply.
        // (The saved-output full result is a separate, non-model-facing UI
        // surface and is left to the result-fetch path.)
        let preview = rewrite_local_host_output_aliases(&output.preview, &self.workdir_aliases);
        Ok(CommandExecutionOutput {
            output: preview,
            saved_output: output.saved_output,
            exit_code: i64::from(exit_code),
            sandboxed: false,
            duration: start.elapsed(),
        })
    }
}

async fn execute_local_command(
    scope: &ResourceScope,
    cmd: &str,
    workdir: &PathBuf,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
    env_mode: HostProcessEnvMode,
) -> Result<(CapturedCommandOutput, i32), RuntimeProcessError> {
    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };

    #[cfg(unix)]
    command.process_group(0);

    match env_mode {
        HostProcessEnvMode::Scrubbed => {
            command.env_clear();
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    command.env(var, val);
                }
            }
            // Keep shell "~" expansion available without exposing the host user's home.
            command.env("HOME", workdir);
        }
        HostProcessEnvMode::Inherited => {}
    }
    command.envs(extra_env);
    command
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        RuntimeProcessError::ExecutionFailed(format!("Failed to spawn command: {e}"))
    })?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(out) = stdout_handle {
                read_stream_capped(scope, out).await
            } else {
                Ok(StreamCapture::default())
            }
        };

        let stderr_fut = async {
            if let Some(err) = stderr_handle {
                read_stream_capped(scope, err).await
            } else {
                Ok(StreamCapture::default())
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("Command execution failed: {error}"))
        })?;
        Ok::<_, RuntimeProcessError>((stdout?, stderr?, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok((stdout, stderr, code))) => {
            Ok((capture_command_output(scope, stdout, stderr)?, code))
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            terminate_child_tree(&mut child).await;
            Err(RuntimeProcessError::Timeout(timeout))
        }
    }
}

async fn terminate_child_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: Child was spawned into its own process group with pgid == pid.
        // Negative pid targets only that process group; result is best-effort.
        unsafe {
            let _ = kill(-(pid as i32), SIGKILL);
        }
    }
    if let Err(error) = child.kill().await {
        tracing::debug!(?error, "best-effort child termination failed");
    }
    if let Err(error) = child.wait().await {
        tracing::debug!(?error, "best-effort reap of terminated child failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_output::COMMAND_MAX_OUTPUT_SIZE;
    #[cfg(unix)]
    use ironclaw_host_api::process::SavedCommandOutputSanitization;
    use ironclaw_host_api::{
        action::{NetworkScheme, NetworkTargetPattern},
        capability::{RuntimeCredentialRequirement, RuntimeCredentialRequirementSource},
        http::RuntimeCredentialTarget,
        ids::{CapabilityId, SecretHandle},
    };
    use parking_lot::Mutex;

    #[derive(Debug, PartialEq, Eq)]
    struct RecordedCredential {
        placeholder_env: String,
        placeholder: String,
        approved_host: String,
        header_name: String,
        header_prefix: Option<String>,
        secret: String,
    }

    #[derive(Debug)]
    struct RecordingSandboxTransport {
        requests: Mutex<Vec<CommandExecutionRequest>>,
        credentialed_requests:
            Mutex<Vec<(CredentialedSandboxCommandRequest, Vec<RecordedCredential>)>>,
        output: String,
    }

    impl Default for RecordingSandboxTransport {
        fn default() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                credentialed_requests: Mutex::new(Vec::new()),
                output: "echo sandbox".to_string(),
            }
        }
    }

    #[derive(Debug)]
    struct FailingSandboxTransport;

    #[derive(Debug)]
    struct TimeoutSandboxTransport;

    #[async_trait]
    impl SandboxCommandTransport for RecordingSandboxTransport {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            self.requests.lock().push(request);
            Ok(CommandExecutionOutput {
                output: self.output.clone(),
                saved_output: None,
                exit_code: 0,
                sandboxed: false,
                duration: Duration::from_millis(3),
            })
        }

        async fn run_credentialed_command(
            &self,
            request: CredentialedSandboxCommandRequest,
            credentials: Vec<SandboxCommandCredential>,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            let credentials = credentials
                .iter()
                .map(|credential| RecordedCredential {
                    placeholder_env: credential.placeholder_env.clone(),
                    placeholder: credential.placeholder.clone(),
                    approved_host: credential.approved_host.clone(),
                    header_name: credential.header_name.clone(),
                    header_prefix: credential.header_prefix.clone(),
                    secret: credential.expose_secret().to_string(),
                })
                .collect();
            self.credentialed_requests
                .lock()
                .push((request, credentials));
            Ok(CommandExecutionOutput {
                output: self.output.clone(),
                saved_output: None,
                exit_code: 0,
                sandboxed: false,
                duration: Duration::from_millis(3),
            })
        }

        fn supports_credentialed_command(&self) -> bool {
            true
        }
    }

    #[async_trait]
    impl SandboxCommandTransport for FailingSandboxTransport {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed(
                "sandbox transport failed".to_string(),
            ))
        }
    }

    #[async_trait]
    impl SandboxCommandTransport for TimeoutSandboxTransport {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::Timeout(Duration::from_secs(
                request.timeout_secs.unwrap_or_default(),
            )))
        }
    }

    #[tokio::test]
    async fn staged_credential_process_port_materializes_invocation_placeholders_for_compound_shell()
     {
        use ironclaw_secrets::SecretMaterial;

        let transport = Arc::new(RecordingSandboxTransport::default());
        let sandbox_port: Arc<dyn RuntimeProcessPort> =
            Arc::new(UserSandboxProcessPort::new(transport.clone()));
        let store = Arc::new(crate::obligations::RuntimeSecretInjectionStore::new());
        let scope = ResourceScope::system();
        let capability_id = CapabilityId::new(crate::SHELL_CAPABILITY_ID).unwrap();
        let atlas = credential_binding(
            "atlas_runtime_token",
            "ATLAS_TOKEN",
            "api.atlas.test",
            "authorization",
            Some("Bearer "),
        );
        let acme = credential_binding(
            "acme_runtime_token",
            "ACME_TOKEN",
            "api.acme.test",
            "x-api-key",
            None,
        );
        for (binding, secret) in [(&atlas, "atlas_real_token"), (&acme, "acme_real_token")] {
            store
                .insert(
                    &scope,
                    &capability_id,
                    &binding.requirement.handle,
                    SecretMaterial::from(secret),
                )
                .unwrap();
        }
        let port = StagedCredentialProcessPort::new(sandbox_port, store);

        port.run_credentialed_command(
            CredentialedSandboxCommandRequest {
                capability_id: capability_id.clone(),
                scope,
                mounts: None,
                command: "set -e; api-client resource list | jq '.items'; api-client audit"
                    .to_string(),
                workdir: None,
                timeout_secs: Some(10),
                extra_env: HashMap::new(),
                credential_bindings: vec![atlas, acme],
            },
            Vec::new(),
        )
        .await
        .unwrap();

        let requests = transport.credentialed_requests.lock();
        let (request, credentials) = &requests[0];
        assert_eq!(
            request.command,
            "set -e; api-client resource list | jq '.items'; api-client audit"
        );
        assert!(request.credential_bindings.is_empty());
        assert_eq!(
            credentials,
            &[
                RecordedCredential {
                    placeholder_env: "ATLAS_TOKEN".to_string(),
                    placeholder: request.extra_env["ATLAS_TOKEN"].clone(),
                    approved_host: "api.atlas.test".to_string(),
                    header_name: "authorization".to_string(),
                    header_prefix: Some("Bearer ".to_string()),
                    secret: "atlas_real_token".to_string(),
                },
                RecordedCredential {
                    placeholder_env: "ACME_TOKEN".to_string(),
                    placeholder: request.extra_env["ACME_TOKEN"].clone(),
                    approved_host: "api.acme.test".to_string(),
                    header_name: "x-api-key".to_string(),
                    header_prefix: None,
                    secret: "acme_real_token".to_string(),
                },
            ]
        );
        for (credential, name) in credentials.iter().zip(["ATLAS_TOKEN", "ACME_TOKEN"]) {
            let placeholder = request.extra_env.get(name).unwrap();
            assert_eq!(credential.placeholder, *placeholder);
            assert!(placeholder.starts_with(ironclaw_secrets::CREDENTIAL_PLACEHOLDER_PREFIX));
            assert!(!placeholder.contains("real_token"));
        }
    }

    fn credential_binding(
        handle: &str,
        placeholder_env: &str,
        approved_host: &str,
        header_name: &str,
        header_prefix: Option<&str>,
    ) -> SandboxCommandCredentialBinding {
        SandboxCommandCredentialBinding {
            placeholder_env: placeholder_env.to_string(),
            requirement: RuntimeCredentialRequirement {
                handle: SecretHandle::new(handle).unwrap(),
                source: RuntimeCredentialRequirementSource::SecretHandle,
                provider_scopes: Vec::new(),
                audience: NetworkTargetPattern {
                    scheme: Some(NetworkScheme::Https),
                    host_pattern: approved_host.to_string(),
                    port: None,
                },
                target: RuntimeCredentialTarget::Header {
                    name: header_name.to_string(),
                    prefix: header_prefix.map(str::to_string),
                },
                placeholder_env: Some(placeholder_env.to_string()),
                required: true,
            },
        }
    }

    fn credentialed_request(
        bindings: Vec<SandboxCommandCredentialBinding>,
    ) -> CredentialedSandboxCommandRequest {
        CredentialedSandboxCommandRequest {
            capability_id: CapabilityId::new(crate::SHELL_CAPABILITY_ID).unwrap(),
            scope: ResourceScope::system(),
            mounts: None,
            command: "api-client resource list".to_string(),
            workdir: None,
            timeout_secs: Some(10),
            extra_env: HashMap::new(),
            credential_bindings: bindings,
        }
    }

    #[test]
    fn user_sandbox_process_port_delegates_credential_support_to_transport() {
        let supported = UserSandboxProcessPort::new(Arc::new(RecordingSandboxTransport::default()));
        let unsupported = UserSandboxProcessPort::new(Arc::new(FailingSandboxTransport));

        assert!(supported.supports_credentialed_command());
        assert!(!unsupported.supports_credentialed_command());
    }

    #[tokio::test]
    async fn user_sandbox_process_port_defaults_credentialed_timeout_to_120_seconds() {
        let transport = Arc::new(RecordingSandboxTransport::default());
        let port = UserSandboxProcessPort::new(transport.clone());
        let mut request = credentialed_request(Vec::new());
        request.timeout_secs = None;

        port.run_credentialed_command(request, Vec::new())
            .await
            .unwrap();

        let requests = transport.credentialed_requests.lock();
        assert_eq!(requests[0].0.timeout_secs, Some(120));
    }
    #[tokio::test]
    async fn staged_port_rejects_invalid_bindings_without_consuming_staged_material() {
        use ironclaw_secrets::SecretMaterial;

        let base = credential_binding(
            "atlas_runtime_token",
            "ATLAS_TOKEN",
            "api.atlas.test",
            "authorization",
            Some("Bearer "),
        );
        let mut wildcard = base.clone();
        wildcard.requirement.audience.host_pattern = "*.atlas.test".to_string();
        let mut empty_host = base.clone();
        empty_host.requirement.audience.host_pattern.clear();
        let mut query_target = base.clone();
        query_target.requirement.target = RuntimeCredentialTarget::QueryParam {
            name: "access_token".to_string(),
        };
        let mut missing_placeholder_env = base.clone();
        missing_placeholder_env.requirement.placeholder_env = None;

        for (case, binding) in [
            ("wildcard host", wildcard),
            ("empty host", empty_host),
            ("non-header target", query_target),
            ("missing placeholder env", missing_placeholder_env),
        ] {
            let transport = Arc::new(RecordingSandboxTransport::default());
            let inner: Arc<dyn RuntimeProcessPort> =
                Arc::new(UserSandboxProcessPort::new(transport.clone()));
            let store = Arc::new(crate::obligations::RuntimeSecretInjectionStore::new());
            let request = credentialed_request(vec![binding.clone()]);
            store
                .insert(
                    &request.scope,
                    &request.capability_id,
                    &binding.requirement.handle,
                    SecretMaterial::from("atlas_real_token"),
                )
                .unwrap();
            let port = StagedCredentialProcessPort::new(inner, store.clone());

            let error = port
                .run_credentialed_command(request.clone(), Vec::new())
                .await
                .unwrap_err();

            assert!(
                matches!(error, RuntimeProcessError::ExecutionFailed(_)),
                "expected fail-closed binding rejection for {case}: {error}"
            );
            assert!(
                transport.credentialed_requests.lock().is_empty(),
                "invalid binding reached the sandbox transport: {case}"
            );
            assert!(
                store
                    .take(
                        &request.scope,
                        &request.capability_id,
                        &binding.requirement.handle,
                    )
                    .unwrap()
                    .is_some(),
                "invalid binding consumed staged material: {case}"
            );
        }
    }

    #[tokio::test]
    async fn staged_port_keeps_present_material_when_another_handle_is_unstaged() {
        use ironclaw_secrets::SecretMaterial;

        let staged = credential_binding(
            "atlas_runtime_token",
            "ATLAS_TOKEN",
            "api.atlas.test",
            "authorization",
            Some("Bearer "),
        );
        let unstaged = credential_binding(
            "acme_runtime_token",
            "ACME_TOKEN",
            "api.acme.test",
            "x-api-key",
            None,
        );
        let request = credentialed_request(vec![staged.clone(), unstaged]);
        let transport = Arc::new(RecordingSandboxTransport::default());
        let inner: Arc<dyn RuntimeProcessPort> =
            Arc::new(UserSandboxProcessPort::new(transport.clone()));
        let store = Arc::new(crate::obligations::RuntimeSecretInjectionStore::new());
        store
            .insert(
                &request.scope,
                &request.capability_id,
                &staged.requirement.handle,
                SecretMaterial::from("atlas_real_token"),
            )
            .unwrap();
        let port = StagedCredentialProcessPort::new(inner, store.clone());

        let error = port
            .run_credentialed_command(request.clone(), Vec::new())
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeProcessError::ExecutionFailed(
                "an authorized sandbox credential was not staged for this invocation".to_string()
            )
        );
        assert!(transport.credentialed_requests.lock().is_empty());
        assert!(
            store
                .take(
                    &request.scope,
                    &request.capability_id,
                    &staged.requirement.handle,
                )
                .unwrap()
                .is_some(),
            "atomic take_many must preserve already staged material"
        );
    }

    #[tokio::test]
    async fn user_sandbox_process_port_marks_output_sandboxed() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = UserSandboxProcessPort::new(transport);

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap();

        assert_eq!(output.output, "echo sandbox");
        assert!(output.sandboxed);
    }

    #[tokio::test]
    async fn user_sandbox_process_port_sets_default_timeout_on_transport_request() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = UserSandboxProcessPort::new(transport.clone());

        port.run_command(CommandExecutionRequest {
            scope: ResourceScope::system(),
            mounts: None,
            command: "echo sandbox".to_string(),
            workdir: None,
            timeout_secs: None,
            extra_env: HashMap::new(),
        })
        .await
        .unwrap();

        let requests = transport.requests.lock();
        assert_eq!(
            requests[0].timeout_secs,
            Some(DEFAULT_COMMAND_TIMEOUT.as_secs())
        );
    }

    #[tokio::test]
    async fn user_sandbox_process_port_propagates_transport_error() {
        let port = UserSandboxProcessPort::new(std::sync::Arc::new(FailingSandboxTransport));

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeProcessError::ExecutionFailed("sandbox transport failed".to_string())
        );
    }

    #[tokio::test]
    async fn user_sandbox_process_port_propagates_transport_timeout() {
        let port = UserSandboxProcessPort::new(std::sync::Arc::new(TimeoutSandboxTransport));

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, RuntimeProcessError::Timeout(Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn user_sandbox_process_port_truncates_transport_output() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport {
            requests: Mutex::new(Vec::new()),
            credentialed_requests: Mutex::new(Vec::new()),
            output: "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1),
        });
        let port = UserSandboxProcessPort::new(transport);

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap();

        assert!(output.output.contains("... [truncated 1 bytes] ..."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_saves_large_output_file() {
        let workdir = tempfile::tempdir().expect("tempdir");
        let middle = "MIDDLE-FROM-COMMAND";

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "yes a | head -c 70000; printf 'MIDDLE-FROM-COMMAND'; yes z | head -c 70000",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            HostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = std::fs::read_to_string(&saved_output.path).expect("saved output readable");
        #[allow(clippy::let_underscore_must_use)]
        // best-effort test teardown; cleanup failure is irrelevant
        let _ = std::fs::remove_file(&saved_output.path);

        assert_eq!(exit_code, 0);
        assert!(!output.preview.contains(middle));
        assert_eq!(
            saved_output.sanitization,
            SavedCommandOutputSanitization::Clean
        );
        assert!(saved.contains(middle));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_overrides_home_to_workdir() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "printf '%s' \"$HOME\"",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            HostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview, workdir.path().display().to_string());
        assert_eq!(output.saved_output, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_inherited_env_preserves_home_and_host_env() {
        let workdir = tempfile::tempdir().expect("tempdir");
        let home = std::env::var("HOME").expect("HOME set for inherited env test");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "printf '%s\\n%s' \"$HOME\" \"$IRONCLAW_REBORN_SENTINEL\"",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::from([(
                "IRONCLAW_REBORN_SENTINEL".to_string(),
                "inherited".to_string(),
            )]),
            HostProcessEnvMode::Inherited,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview, format!("{home}\ninherited"));
        assert_eq!(output.saved_output, None);
    }

    #[tokio::test]
    async fn local_host_process_port_translates_workspace_workdir_when_configured() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        // Production canonicalizes the workspace root before wiring the alias
        // (`HostWorkdirAlias::try_new` requires a canonical host_path);
        // honor that here so the alias prefix matches the canonical `$PWD` the
        // OS reports (macOS resolves `/var/...` -> `/private/var/...`).
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        std::fs::create_dir_all(workspace_root.join("qa-coding-smoke"))
            .expect("nested workspace dir");
        let port =
            HostProcessPort::new_inherited_env().with_workdir_alias("/workspace", workspace_root);

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "printf '%s' \"$PWD\"".to_string(),
                workdir: Some("/workspace/qa-coding-smoke".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        // `$PWD` is the real host workspace path at exec time; the reverse output
        // rewrite maps it back to the virtual alias before it reaches the model,
        // so the caller never sees the host layout.
        assert_eq!(output.output, "/workspace/qa-coding-smoke");
    }

    #[tokio::test]
    async fn local_host_process_port_virtualizes_host_paths_in_output() {
        // Regression for the produced-file path leak: the command is rewritten
        // `/workspace` -> host path before exec, so a program that echoes a path
        // it was handed (`printf '... %s' /workspace/out.pdf`) prints the host
        // path. The reverse output rewrite must restore the `/workspace` form so
        // the model reports a downloadable workspace path, not the host layout.
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let port = HostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace.path().to_path_buf());

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "printf 'saved to %s\\n' /workspace/out.pdf".to_string(),
                workdir: None,
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "saved to /workspace/out.pdf\n");
    }

    #[tokio::test]
    async fn local_host_process_port_rewrites_command_path_aliases() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let scratch = workspace.path().join("qa-coding-smoke");
        let port = HostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace.path().to_path_buf());

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "mkdir -p /workspace/qa-coding-smoke && test -d /workspace/qa-coding-smoke && printf ok".to_string(),
                workdir: None,
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "ok");
        assert!(scratch.exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn execute_local_command_runs_through_windows_cmd() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "echo %HOME%",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            HostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview.trim(), workdir.path().display().to_string());
        assert_eq!(output.saved_output, None);
    }
}
