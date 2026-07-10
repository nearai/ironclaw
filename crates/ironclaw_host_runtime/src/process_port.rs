//! Runtime process effect port for command-style first-party capabilities.
//!
//! The port keeps process placement outside individual tools. A capability such
//! as `builtin.shell` describes the command to run; host-runtime composition
//! decides which port implementation receives it. This first slice wires the
//! existing local-host behavior behind an explicit port without changing
//! placement semantics.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_host_api::{MountGrant, MountView, ResourceScope, VirtualPath};
#[cfg(unix)]
use libc::{SIGKILL, kill};
use thiserror::Error;
use tokio::process::Command;

use crate::process_aliases::{
    LocalHostWorkdirAlias, resolve_local_host_workdir, rewrite_local_host_command_aliases,
    rewrite_local_host_output_aliases,
};
use crate::process_output::{
    CapturedCommandOutput, SavedCommandOutput, StreamCapture, capture_command_output,
    read_stream_capped, truncate_output,
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

/// Placement-neutral command request handed to the selected process port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionRequest {
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
}

/// Process-port command result normalized for capability handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionOutput {
    pub output: String,
    pub saved_output: Option<SavedCommandOutput>,
    pub exit_code: i64,
    pub sandboxed: bool,
    pub duration: Duration,
}

/// Stable redacted process-port failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeProcessError {
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("process execution failed: {0}")]
    ExecutionFailed(String),
}

/// Abstract process effect used by process-backed capabilities.
#[async_trait]
pub trait RuntimeProcessPort: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;
}

/// Transport for tenant-sandbox command execution.
///
/// This trait intentionally hides Docker/daemon details from host-runtime tool
/// code. Product adapters can implement it with the V1 sandbox daemon JSON-RPC
/// transport or another tenant-isolated runner.
///
/// Implementations must enforce `CommandExecutionRequest::timeout_secs` and
/// clean up any remote process/container before returning
/// `RuntimeProcessError::Timeout`.
#[async_trait]
pub trait SandboxCommandTransport: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;
}

/// Tenant-isolated process port backed by a sandbox command transport.
#[derive(Clone)]
pub struct TenantSandboxProcessPort {
    transport: std::sync::Arc<dyn SandboxCommandTransport>,
}

impl std::fmt::Debug for TenantSandboxProcessPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantSandboxProcessPort")
            .field("transport", &"<sandbox command transport>")
            .finish()
    }
}

impl TenantSandboxProcessPort {
    pub fn new(transport: std::sync::Arc<dyn SandboxCommandTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl RuntimeProcessPort for TenantSandboxProcessPort {
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
}

/// Local provider-host command environment handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LocalHostProcessEnvMode {
    /// Clear the child environment, forward only `SAFE_ENV_VARS`, and rewrite
    /// `HOME` to the command workdir.
    #[default]
    Scrubbed,
    /// Inherit the host process environment and real `HOME`.
    Inherited,
}

/// Local provider-host command implementation matching the existing shell path.
#[derive(Debug, Clone, Default)]
pub struct LocalHostProcessPort {
    env_mode: LocalHostProcessEnvMode,
    workdir_aliases: Vec<LocalHostWorkdirAlias>,
    mount_sources: Vec<LocalHostMountSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalHostMountSource {
    virtual_root: VirtualPath,
    host_root: PathBuf,
    host_root_spellings: Vec<PathBuf>,
}

impl LocalHostProcessPort {
    pub fn new() -> Self {
        Self {
            env_mode: LocalHostProcessEnvMode::Scrubbed,
            workdir_aliases: Vec::new(),
            mount_sources: Vec::new(),
        }
    }

    pub fn new_inherited_env() -> Self {
        Self {
            env_mode: LocalHostProcessEnvMode::Inherited,
            workdir_aliases: Vec::new(),
            mount_sources: Vec::new(),
        }
    }

    pub fn with_workdir_alias(
        mut self,
        alias: impl Into<String>,
        host_path: impl Into<PathBuf>,
    ) -> Self {
        match LocalHostWorkdirAlias::try_new(alias, host_path) {
            Ok(alias) => self.workdir_aliases.push(alias),
            Err(reason) => tracing::debug!(
                reason = %reason,
                "ignoring invalid local host process workdir alias"
            ),
        }
        self
    }

    pub fn with_mount_source(
        mut self,
        virtual_root: impl Into<String>,
        host_root: impl Into<PathBuf>,
    ) -> Self {
        let virtual_root = match VirtualPath::new(virtual_root.into()) {
            Ok(virtual_root) => virtual_root,
            Err(reason) => {
                tracing::debug!(
                    reason = %reason,
                    "ignoring invalid local host process mount source"
                );
                return self;
            }
        };
        let configured_host_root = host_root.into();
        let host_root = match std::fs::canonicalize(&configured_host_root) {
            Ok(host_root) if host_root.is_dir() => host_root,
            Ok(host_root) => {
                tracing::debug!(
                    host_root = ?host_root,
                    "ignoring local host process mount source because it is not a directory"
                );
                return self;
            }
            Err(reason) => {
                tracing::debug!(
                    reason = %reason,
                    "ignoring unresolved local host process mount source"
                );
                return self;
            }
        };
        let mut host_root_spellings = vec![host_root.clone()];
        if configured_host_root.is_absolute() && configured_host_root != host_root {
            host_root_spellings.push(configured_host_root);
        }
        if self
            .mount_sources
            .iter()
            .any(|source| source.virtual_root == virtual_root)
        {
            tracing::debug!(
                virtual_root = %virtual_root,
                "ignoring duplicate local host process mount source"
            );
            return self;
        }
        self.mount_sources.push(LocalHostMountSource {
            virtual_root,
            host_root,
            host_root_spellings,
        });
        self
    }

    fn effective_workdir_aliases(
        &self,
        mounts: Option<&MountView>,
    ) -> Result<Vec<LocalHostWorkdirAlias>, RuntimeProcessError> {
        let mut aliases = self.workdir_aliases.clone();
        let Some(mounts) = mounts else {
            return Ok(aliases);
        };

        for grant in &mounts.mounts {
            let source = self.resolve_mount_source(grant)?;
            let Some(host_path) = source else {
                continue;
            };
            let alias = LocalHostWorkdirAlias::try_new(grant.alias.as_str(), host_path)
                .map_err(RuntimeProcessError::ExecutionFailed)?;
            if let Some(existing) = aliases
                .iter_mut()
                .find(|existing| existing.alias() == alias.alias())
            {
                *existing = alias;
            } else {
                aliases.push(alias);
            }
        }

        Ok(aliases)
    }

    fn resolve_mount_source(
        &self,
        grant: &MountGrant,
    ) -> Result<Option<PathBuf>, RuntimeProcessError> {
        let source = self
            .mount_sources
            .iter()
            .filter(|source| {
                virtual_path_prefix_matches(source.virtual_root.as_str(), grant.target.as_str())
            })
            .max_by_key(|source| source.virtual_root.as_str().len());

        let Some(source) = source else {
            if self
                .workdir_aliases
                .iter()
                .any(|alias| alias.alias() == grant.alias.as_str())
            {
                return Err(RuntimeProcessError::ExecutionFailed(format!(
                    "no trusted local host mount source is configured for virtual path {}",
                    grant.target
                )));
            }
            return Ok(None);
        };

        let mut joined = source.host_root.clone();
        let tail = grant
            .target
            .as_str()
            .strip_prefix(source.virtual_root.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');
        if !tail.is_empty() {
            for segment in tail.split('/') {
                joined.push(segment);
            }
        }

        if grant.permissions.write {
            std::fs::create_dir_all(&joined).map_err(|error| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "local host mount target {} could not be initialized: {error}",
                    grant.target
                ))
            })?;
        }
        let canonical = std::fs::canonicalize(&joined).map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!(
                "local host mount target {} could not be resolved: {error}",
                grant.target
            ))
        })?;
        if !canonical.starts_with(&source.host_root) {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "local host mount target {} escapes its trusted source",
                grant.target
            )));
        }
        if !canonical.is_dir() {
            return Err(RuntimeProcessError::ExecutionFailed(format!(
                "local host mount target {} is not a directory",
                grant.target
            )));
        }

        Ok(Some(canonical))
    }

    fn validate_mount_source_access(
        &self,
        command: &str,
        cwd: &Path,
        workdir_aliases: &[LocalHostWorkdirAlias],
    ) -> Result<(), RuntimeProcessError> {
        let guards = mount_source_guards(&self.mount_sources, workdir_aliases);
        if guards.is_empty() {
            return Ok(());
        }
        validate_mount_source_path(cwd, &guards, "working directory")?;
        validate_raw_mount_source_paths(command, &guards)?;
        validate_relative_mount_source_paths(command, cwd, &guards)?;
        validate_mount_sensitive_shell_expansions(command, &guards)?;
        Ok(())
    }

    fn default_scoped_workdir_alias(
        &self,
        workdir_aliases: &[LocalHostWorkdirAlias],
    ) -> Option<String> {
        workdir_aliases
            .iter()
            .filter(|alias| {
                let host_path = canonicalize_existing_path_prefix(alias.host_path());
                self.mount_sources.iter().any(|source| {
                    host_path.starts_with(&source.host_root) && host_path != source.host_root
                })
            })
            .max_by_key(|alias| alias.alias().len())
            .map(|alias| alias.alias().to_string())
    }
}

#[async_trait]
impl RuntimeProcessPort for LocalHostProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let workdir_aliases = self.effective_workdir_aliases(request.mounts.as_ref())?;
        let default_workdir = if request.workdir.is_none() {
            self.default_scoped_workdir_alias(&workdir_aliases)
        } else {
            None
        };
        let cwd = resolve_local_host_workdir(
            request.workdir.as_deref().or(default_workdir.as_deref()),
            &workdir_aliases,
        )
        .map_err(|e| {
            RuntimeProcessError::ExecutionFailed(format!("cannot determine working directory: {e}"))
        })?;
        let canonical_cwd = std::fs::canonicalize(&cwd).unwrap_or_else(|_| cwd.clone());
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        if self.env_mode == LocalHostProcessEnvMode::Inherited {
            tracing::warn!(
                host_access = "full-local",
                "running local host command with inherited environment"
            );
        }
        let command = rewrite_local_host_command_aliases(&request.command, &workdir_aliases);
        self.validate_mount_source_access(&command, &canonical_cwd, &workdir_aliases)?;
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
        let preview = rewrite_local_host_output_aliases(&output.preview, &workdir_aliases);
        Ok(CommandExecutionOutput {
            output: preview,
            saved_output: output.saved_output,
            exit_code: i64::from(exit_code),
            sandboxed: false,
            duration: start.elapsed(),
        })
    }
}

struct LocalHostMountSourceGuard {
    source_root: PathBuf,
    source_spellings: Vec<PathBuf>,
    allowed_roots: Vec<PathBuf>,
}

impl LocalHostMountSourceGuard {
    fn is_scoped(&self) -> bool {
        self.allowed_roots
            .iter()
            .all(|allowed_root| allowed_root != &self.source_root)
    }
}

fn mount_source_guards(
    mount_sources: &[LocalHostMountSource],
    workdir_aliases: &[LocalHostWorkdirAlias],
) -> Vec<LocalHostMountSourceGuard> {
    mount_sources
        .iter()
        .map(|source| LocalHostMountSourceGuard {
            source_root: source.host_root.clone(),
            source_spellings: source.host_root_spellings.to_vec(),
            allowed_roots: workdir_aliases
                .iter()
                .map(LocalHostWorkdirAlias::host_path)
                .map(canonicalize_existing_path_prefix)
                .filter(|host_path| host_path.starts_with(&source.host_root))
                .collect(),
        })
        .collect()
}

fn validate_mount_source_path(
    path: &Path,
    guards: &[LocalHostMountSourceGuard],
    label: &str,
) -> Result<(), RuntimeProcessError> {
    let resolved_path = canonicalize_existing_path_prefix(path);
    let Some(guard) = guards
        .iter()
        .find(|guard| resolved_path.starts_with(&guard.source_root))
    else {
        return Ok(());
    };
    if path_has_parent_dir(path)
        || !guard
            .allowed_roots
            .iter()
            .any(|allowed_root| resolved_path.starts_with(allowed_root))
    {
        return Err(disallowed_mount_source_path(label));
    }
    Ok(())
}

fn validate_raw_mount_source_paths(
    command: &str,
    guards: &[LocalHostMountSourceGuard],
) -> Result<(), RuntimeProcessError> {
    for guard in guards {
        for spelling in &guard.source_spellings {
            let Some(source_root) = spelling.to_str() else {
                continue;
            };
            if source_root.is_empty() {
                continue;
            }
            let mut search_start = 0;
            while let Some(relative_index) = command[search_start..].find(source_root) {
                let index = search_start + relative_index;
                let prefix_end = index + source_root.len();
                if !command_path_start_boundary(command, index)
                    || !command_path_end_boundary(command, prefix_end)
                {
                    search_start = prefix_end;
                    continue;
                }
                let token_end = command_path_token_end(command, prefix_end);
                validate_mount_source_path(
                    Path::new(&command[index..token_end]),
                    guards,
                    "command path",
                )?;
                search_start = token_end;
            }
        }
    }
    let mut search_start = 0;
    while search_start < command.len() {
        let Some((token_start, token_end)) = next_command_path_token(command, search_start) else {
            break;
        };
        let token = Path::new(&command[token_start..token_end]);
        if token == Path::new("/") && guards.iter().any(LocalHostMountSourceGuard::is_scoped) {
            return Err(disallowed_mount_source_path("command path"));
        }
        if token.is_absolute() {
            validate_mount_source_path(token, guards, "command path")?;
        }
        search_start = token_end;
    }
    Ok(())
}

fn validate_relative_mount_source_paths(
    command: &str,
    cwd: &Path,
    guards: &[LocalHostMountSourceGuard],
) -> Result<(), RuntimeProcessError> {
    let Some(scoped_root) = scoped_allowed_root_for_path(cwd, guards) else {
        return Ok(());
    };
    let mut search_start = 0;
    while search_start < command.len() {
        let Some((token_start, token_end)) = next_command_path_token(command, search_start) else {
            break;
        };
        let token = &command[token_start..token_end];
        let path = Path::new(token);
        if path.is_relative()
            && path_has_parent_dir(path)
            && (token.contains('/') || previous_command_word(command, token_start) == Some("cd"))
        {
            let resolved = normalize_path_lexically(&cwd.join(path));
            if !resolved.starts_with(scoped_root) {
                return Err(disallowed_mount_source_path("relative command path"));
            }
        }
        search_start = token_end;
    }
    Ok(())
}

fn validate_mount_sensitive_shell_expansions(
    command: &str,
    guards: &[LocalHostMountSourceGuard],
) -> Result<(), RuntimeProcessError> {
    if !guards.iter().any(LocalHostMountSourceGuard::is_scoped) {
        return Ok(());
    }
    if command.contains('`') || command.contains("$(") {
        return Err(disallowed_mount_source_path("command expansion"));
    }
    let bytes = command.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let Some(next) = bytes.get(index + 1).copied() else {
            break;
        };
        if next == b'{' {
            let Some(relative_end) = command[index + 2..].find('}') else {
                return Err(disallowed_mount_source_path("command expansion"));
            };
            let name_start = index + 2;
            let name_end = name_start + relative_end;
            let variable = &command[name_start..name_end];
            if variable != "PWD"
                || command[name_end + 1..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch == '/' || ch == '\\')
            {
                return Err(disallowed_mount_source_path("command expansion"));
            }
            index = name_end + 2;
            continue;
        }
        if shell_variable_start(next) {
            let name_start = index + 1;
            let mut name_end = name_start + 1;
            while name_end < bytes.len() && shell_variable_char(bytes[name_end]) {
                name_end += 1;
            }
            let variable = &command[name_start..name_end];
            if variable != "PWD"
                || command[name_end..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch == '/' || ch == '\\')
            {
                return Err(disallowed_mount_source_path("command expansion"));
            }
            index = name_end;
            continue;
        }
        index += 1;
    }
    Ok(())
}

fn shell_variable_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn shell_variable_char(byte: u8) -> bool {
    shell_variable_start(byte) || byte.is_ascii_digit()
}

fn scoped_allowed_root_for_path<'a>(
    path: &Path,
    guards: &'a [LocalHostMountSourceGuard],
) -> Option<&'a Path> {
    guards
        .iter()
        .filter(|guard| guard.is_scoped())
        .flat_map(|guard| guard.allowed_roots.iter().map(PathBuf::as_path))
        .filter(|allowed_root| path.starts_with(allowed_root))
        .max_by_key(|allowed_root| allowed_root.as_os_str().len())
}

fn next_command_path_token(command: &str, search_start: usize) -> Option<(usize, usize)> {
    let mut token_start = None;
    for (relative_index, ch) in command[search_start..].char_indices() {
        if command_path_char(ch) {
            token_start = Some(search_start + relative_index);
            break;
        }
    }
    let token_start = token_start?;
    Some((token_start, command_path_token_end(command, token_start)))
}

fn command_path_token_end(command: &str, mut index: usize) -> usize {
    while index < command.len() {
        let Some(ch) = command[index..].chars().next() else {
            break;
        };
        if !command_path_char(ch) {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn command_path_start_boundary(command: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    command[..index]
        .chars()
        .next_back()
        .is_none_or(|ch| !command_path_char(ch))
}

fn command_path_end_boundary(command: &str, index: usize) -> bool {
    if index >= command.len() {
        return true;
    }
    command[index..]
        .chars()
        .next()
        .is_some_and(|ch| ch == '/' || !command_path_char(ch))
}

fn command_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.')
}

fn previous_command_word(command: &str, token_start: usize) -> Option<&str> {
    let prefix = &command[..token_start];
    let end = prefix.trim_end().len();
    if end == 0 {
        return None;
    }
    let start = prefix[..end]
        .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
        .map(|index| index + 1)
        .unwrap_or(0);
    Some(&prefix[start..end])
}

fn path_has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }
    normalized
}

fn canonicalize_existing_path_prefix(path: &Path) -> PathBuf {
    let mut current = path;
    let mut missing_tail = Vec::new();
    loop {
        if let Ok(canonical) = std::fs::canonicalize(current) {
            let mut resolved = canonical;
            for segment in missing_tail.iter().rev() {
                resolved.push(segment);
            }
            return resolved;
        }
        let Some(parent) = current.parent() else {
            return path.to_path_buf();
        };
        let Some(file_name) = current.file_name() else {
            return path.to_path_buf();
        };
        missing_tail.push(file_name.to_os_string());
        current = parent;
    }
}

fn disallowed_mount_source_path(label: &str) -> RuntimeProcessError {
    RuntimeProcessError::ExecutionFailed(format!(
        "{label} references a host workspace path outside the mounted workspace"
    ))
}

fn virtual_path_prefix_matches(prefix: &str, path: &str) -> bool {
    Path::new(path).starts_with(Path::new(prefix))
}

async fn execute_local_command(
    scope: &ResourceScope,
    cmd: &str,
    workdir: &PathBuf,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
    env_mode: LocalHostProcessEnvMode,
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
        LocalHostProcessEnvMode::Scrubbed => {
            command.env_clear();
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    command.env(var, val);
                }
            }
            // Keep shell "~" expansion available without exposing the host user's home.
            command.env("HOME", workdir);
        }
        LocalHostProcessEnvMode::Inherited => {}
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
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_output::COMMAND_MAX_OUTPUT_SIZE;
    #[cfg(unix)]
    use crate::process_output::SavedCommandOutputSanitization;
    use ironclaw_host_api::{MountAlias, MountPermissions};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct RecordingSandboxTransport {
        requests: Mutex<Vec<CommandExecutionRequest>>,
        output: String,
    }

    impl Default for RecordingSandboxTransport {
        fn default() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
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
            self.requests.lock().unwrap().push(request);
            Ok(CommandExecutionOutput {
                output: self.output.clone(),
                saved_output: None,
                exit_code: 0,
                sandboxed: false,
                duration: Duration::from_millis(3),
            })
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
    async fn tenant_sandbox_process_port_marks_output_sandboxed() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = TenantSandboxProcessPort::new(transport);

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
    async fn tenant_sandbox_process_port_sets_default_timeout_on_transport_request() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = TenantSandboxProcessPort::new(transport.clone());

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

        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].timeout_secs,
            Some(DEFAULT_COMMAND_TIMEOUT.as_secs())
        );
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_propagates_transport_error() {
        let port = TenantSandboxProcessPort::new(std::sync::Arc::new(FailingSandboxTransport));

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
    async fn tenant_sandbox_process_port_propagates_transport_timeout() {
        let port = TenantSandboxProcessPort::new(std::sync::Arc::new(TimeoutSandboxTransport));

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
    async fn tenant_sandbox_process_port_truncates_transport_output() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport {
            requests: Mutex::new(Vec::new()),
            output: "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1),
        });
        let port = TenantSandboxProcessPort::new(transport);

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
            LocalHostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = std::fs::read_to_string(&saved_output.path).expect("saved output readable");
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
            LocalHostProcessEnvMode::Scrubbed,
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
            LocalHostProcessEnvMode::Inherited,
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
        // (`LocalHostWorkdirAlias::try_new` requires a canonical host_path);
        // honor that here so the alias prefix matches the canonical `$PWD` the
        // OS reports (macOS resolves `/var/...` -> `/private/var/...`).
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        std::fs::create_dir_all(workspace_root.join("qa-coding-smoke"))
            .expect("nested workspace dir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root);

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
        let port = LocalHostProcessPort::new_inherited_env()
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
        let port = LocalHostProcessPort::new_inherited_env()
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

    #[tokio::test]
    async fn local_host_process_port_scopes_workspace_alias_from_request_mounts() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root.clone());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "printf '# Hello\\n' > /workspace/hello.md && printf '%s' \"$PWD\""
                    .to_string(),
                workdir: Some("/workspace".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "/workspace");
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("tenants/tenant-a/users/user-a/hello.md"))
                .expect("scoped shell artifact"),
            "# Hello\n"
        );
        assert!(
            !workspace_root.join("hello.md").exists(),
            "scoped shell writes must not land in the raw workspace root"
        );
    }

    #[tokio::test]
    async fn local_host_process_port_defaults_to_scoped_workspace_workdir() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root.clone());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "printf '# Hello\\n' > /workspace/hello.md && printf '%s' \"$PWD\""
                    .to_string(),
                workdir: None,
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "/workspace");
        assert_eq!(
            std::fs::read_to_string(workspace_root.join("tenants/tenant-a/users/user-a/hello.md"))
                .expect("scoped shell artifact"),
            "# Hello\n"
        );
        assert!(
            !workspace_root.join("hello.md").exists(),
            "default shell workdir must not write to the raw workspace root"
        );
    }

    #[tokio::test]
    async fn local_host_process_port_allows_noncanonical_owner_workspace_alias() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace.path().to_path_buf();
        let shell_workdir = workspace_root.join("qa-coding-smoke");
        std::fs::create_dir_all(&shell_workdir).expect("nested workspace dir");
        let host_home = tempfile::tempdir().expect("host home tempdir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root)
            .with_workdir_alias("/host", host_home.path().to_path_buf());

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "mkdir -p /workspace/qa-coding-smoke && test -d /host && printf ok"
                    .to_string(),
                workdir: Some("/workspace/qa-coding-smoke".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "ok");
    }

    #[tokio::test]
    async fn local_host_process_port_rejects_raw_workspace_source_escape() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let scoped_user_root = workspace_root.join("tenants/tenant-a/users/user-a");
        let other_user_root = workspace_root.join("tenants/tenant-a/users/user-b");
        std::fs::create_dir_all(&scoped_user_root).expect("scoped user dir");
        std::fs::create_dir_all(&other_user_root).expect("other user dir");
        let other_user_file = other_user_root.join("pwned.md");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root.clone());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: format!("printf hacked > {}", other_user_file.display()),
                workdir: Some("/workspace".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("raw host workspace escape should be rejected");

        assert!(
            matches!(error, RuntimeProcessError::ExecutionFailed(message) if message.contains("outside the mounted workspace"))
        );
        assert!(
            !other_user_file.exists(),
            "raw host workspace path must not write another user's artifact"
        );
    }

    #[tokio::test]
    async fn local_host_process_port_rejects_raw_root_for_scoped_workspace() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let scoped_user_root = workspace_root.join("tenants/tenant-a/users/user-a");
        std::fs::create_dir_all(&scoped_user_root).expect("scoped user dir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "ls /".to_string(),
                workdir: Some("/workspace".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("scoped shell must not expose the host root");

        assert!(
            matches!(error, RuntimeProcessError::ExecutionFailed(message) if message.contains("outside the mounted workspace"))
        );
    }

    #[tokio::test]
    async fn local_host_process_port_rejects_relative_workspace_escape() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let scoped_user_root = workspace_root.join("tenants/tenant-a/users/user-a");
        let other_user_root = workspace_root.join("tenants/tenant-a/users/user-b");
        std::fs::create_dir_all(&scoped_user_root).expect("scoped user dir");
        std::fs::create_dir_all(&other_user_root).expect("other user dir");
        let other_user_file = other_user_root.join("pwned.md");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root.clone());
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "printf hacked > ../user-b/pwned.md".to_string(),
                workdir: Some("/workspace".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect_err("relative workspace escape should be rejected");

        assert!(
            matches!(error, RuntimeProcessError::ExecutionFailed(message) if message.contains("outside the mounted workspace"))
        );
        assert!(
            !other_user_file.exists(),
            "relative parent traversal must not write another user's artifact"
        );
    }

    #[tokio::test]
    async fn local_host_process_port_allows_literal_parent_text_in_scoped_command() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let scoped_user_root = workspace_root.join("tenants/tenant-a/users/user-a");
        std::fs::create_dir_all(&scoped_user_root).expect("scoped user dir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: Some(mounts),
                command: "printf '..'".to_string(),
                workdir: Some("/workspace".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("literal parent text should not be treated as traversal");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "..");
    }

    #[tokio::test]
    async fn local_host_process_port_rejects_mount_sensitive_shell_expansion() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let workspace_root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let scoped_user_root = workspace_root.join("tenants/tenant-a/users/user-a");
        std::fs::create_dir_all(&scoped_user_root).expect("scoped user dir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace_root.clone())
            .with_mount_source("/projects/workspace", workspace_root);
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new("/projects/workspace/tenants/tenant-a/users/user-a")
                .expect("workspace target"),
            MountPermissions::read_write(),
        )])
        .expect("mount view");

        for command in [
            "printf '%s' \"$(pwd)\"",
            "printf '%s' `pwd`",
            "printf '%s' \"$HOME\"",
            "printf hacked > \"$PWD/../user-b/pwned.md\"",
        ] {
            let error = port
                .run_command(CommandExecutionRequest {
                    scope: ResourceScope::system(),
                    mounts: Some(mounts.clone()),
                    command: command.to_string(),
                    workdir: Some("/workspace".to_string()),
                    timeout_secs: Some(5),
                    extra_env: HashMap::new(),
                })
                .await
                .expect_err("mount-sensitive shell expansion should be rejected");

            assert!(
                matches!(&error, RuntimeProcessError::ExecutionFailed(message) if message.contains("outside the mounted workspace")),
                "unexpected error for {command:?}: {error:?}"
            );
        }
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
            LocalHostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview.trim(), workdir.path().display().to_string());
        assert_eq!(output.saved_output, None);
    }
}
