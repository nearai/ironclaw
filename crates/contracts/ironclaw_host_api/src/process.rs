//! Placement-neutral process-execution vocabulary and the sandbox transport
//! port.
//!
//! The kernel decides *which* process port receives a command; a lane provides
//! the transport that runs it. Declaring both halves here is what lets a
//! `runtimes`-layer lane implement what the kernel consumes without an upward
//! dependency: `ironclaw_sandbox` (runtimes) implements
//! [`SandboxCommandTransport`], `ironclaw_host_runtime` (kernel) wraps it in
//! `UserSandboxProcessPort`. PROPOSAL §6.6.4 records that this home is
//! load-bearing, not cosmetic.
//!
//! `ironclaw_host_runtime` still owns the *behavior* — process spawning, output
//! capture, alias rewriting, and the local-host port. Only the shapes that
//! cross the kernel↔lane seam live here.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};

use async_trait::async_trait;
use thiserror::Error;

use crate::{
    capability::RuntimeCredentialRequirement,
    ids::{CapabilityId, ExtensionId, SecretHandle},
    mount::MountView,
    resource::ResourceScope,
};

/// Metadata for command output persisted behind a saved-output reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedCommandOutput {
    pub path: PathBuf,
    pub sanitization: SavedCommandOutputSanitization,
    pub stream_was_capped: bool,
    pub max_saved_stream_size: usize,
    pub expires_at_unix_secs: u64,
}

/// Whether persisted command output required redaction or blocking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedCommandOutputSanitization {
    Clean,
    Redacted,
    Blocked,
}

/// Placement-neutral shell command request handed to the selected process port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionRequest {
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
}
/// An authorized runtime credential mapped from its manifest-declared
/// placeholder environment variable.
///
/// The requirement is copied from the authorized capability descriptor. The
/// host process adapter consumes its one-shot staged material; callers cannot
/// supply raw credential material through this shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommandCredentialBinding {
    pub placeholder_env: String,
    pub requirement: RuntimeCredentialRequirement,
}

/// A shell command plus the exact manifest-backed credentials authorized for
/// this invocation. The sandbox receives random placeholders; real credential
/// material remains proxy-side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialedSandboxCommandRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
    pub credential_bindings: Vec<SandboxCommandCredentialBinding>,
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
/// One invocation-scoped credential binding handed only to the sandbox
/// transport. `credential_key` addresses this value inside the proxy's JSON
/// bundle. The command receives `placeholder`; only the proxy-side transport
/// may expose `secret`.
#[derive(Clone)]
pub struct SandboxCommandCredential {
    pub credential_key: SecretHandle,
    pub placeholder_env: String,
    pub placeholder: String,
    pub approved_host: String,
    pub header_name: String,
    pub header_prefix: Option<String>,
    secret: zeroize::Zeroizing<String>,
}

impl SandboxCommandCredential {
    pub fn new(
        credential_key: SecretHandle,
        placeholder_env: String,
        placeholder: String,
        approved_host: String,
        header_name: String,
        header_prefix: Option<String>,
        secret: String,
    ) -> Self {
        Self {
            credential_key,
            placeholder_env,
            placeholder,
            approved_host: approved_host.to_ascii_lowercase(),
            header_name: header_name.to_ascii_lowercase(),
            header_prefix,
            secret: zeroize::Zeroizing::new(secret),
        }
    }

    pub fn expose_secret(&self) -> &str {
        self.secret.as_str()
    }
}

impl std::fmt::Debug for SandboxCommandCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxCommandCredential")
            .field("credential_key", &self.credential_key)
            .field("placeholder_env", &self.placeholder_env)
            .field("approved_host", &self.approved_host)
            .field("header_name", &self.header_name)
            .field("header_prefix", &self.header_prefix)
            .finish_non_exhaustive()
    }
}

/// Stable redacted process-port failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeProcessError {
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("process execution failed: {0}")]
    ExecutionFailed(String),
}
pub const MAX_SHELL_CREDENTIAL_CONTEXTS: usize = 8;

/// Parse the optional extension IDs whose manifest credentials a shell
/// invocation requests. The same parser is used before authorization and at
/// runtime so malformed or duplicate selectors fail closed at both stages.
pub fn shell_credential_contexts(
    input: &serde_json::Value,
) -> Result<Vec<ExtensionId>, ShellCredentialContextError> {
    let Some(value) = input.get("credential_contexts") else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or(ShellCredentialContextError::NotArray)?;
    if values.len() > MAX_SHELL_CREDENTIAL_CONTEXTS {
        return Err(ShellCredentialContextError::TooMany {
            maximum: MAX_SHELL_CREDENTIAL_CONTEXTS,
        });
    }

    let mut contexts = Vec::with_capacity(values.len());
    let mut unique = HashSet::with_capacity(values.len());
    for value in values {
        let raw = value
            .as_str()
            .ok_or(ShellCredentialContextError::InvalidExtensionId)?;
        let context = ExtensionId::new(raw)
            .map_err(|source| ShellCredentialContextError::MalformedExtensionId { source })?;
        if !unique.insert(context.clone()) {
            return Err(ShellCredentialContextError::Duplicate {
                context: context.to_string(),
            });
        }
        contexts.push(context);
    }
    Ok(contexts)
}

pub fn is_valid_sandbox_credential_env_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|ch: char| ch.is_ascii_digit())
        && name
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        && !matches!(
            name,
            "BASH_ENV"
                | "CDPATH"
                | "ENV"
                | "IFS"
                | "LD_AUDIT"
                | "LD_LIBRARY_PATH"
                | "LD_PRELOAD"
                | "PATH"
                | "SHELLOPTS"
        )
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShellCredentialContextError {
    #[error("credential_contexts must be an array of active extension IDs")]
    NotArray,
    #[error("credential_contexts entries must be valid extension IDs")]
    InvalidExtensionId,
    #[error("credential_contexts entries must be valid extension IDs")]
    MalformedExtensionId {
        #[source]
        source: crate::error::HostApiError,
    },
    #[error("credential_contexts may contain at most {maximum} entries")]
    TooMany { maximum: usize },
    #[error("credential context `{context}` is duplicated")]
    Duplicate { context: String },
}

/// Transport for user-sandbox command execution.
///
/// This trait intentionally hides Docker/daemon details from host-runtime tool
/// code. A lane implements it with a container runtime or another runner that
/// isolates each authenticated user within the tenant boundary.
///
/// Implementations must enforce [`CommandExecutionRequest::timeout_secs`] and
/// clean up any remote process/container before returning
/// [`RuntimeProcessError::Timeout`].
#[async_trait]
pub trait SandboxCommandTransport: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;

    async fn run_credentialed_command(
        &self,
        _request: CredentialedSandboxCommandRequest,
        credentials: Vec<SandboxCommandCredential>,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let reason = if credentials.is_empty() {
            "sandbox transport does not support credentialed shell execution"
        } else {
            "sandbox transport does not support credential bindings"
        };
        Err(RuntimeProcessError::ExecutionFailed(reason.to_string()))
    }

    fn supports_credentialed_command(&self) -> bool {
        false
    }

    /// Release remote resources owned by this transport after command
    /// producers have stopped. Local transports may keep the default no-op;
    /// remote transports override this with idempotent provider cleanup.
    async fn shutdown(&self) -> Result<(), RuntimeProcessError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentialed_request_preserves_shell_command_text() {
        let request = CredentialedSandboxCommandRequest {
            capability_id: CapabilityId::new("builtin.shell").unwrap(),
            scope: ResourceScope::system(),
            mounts: None,
            command: "set -e; atlas resources | jq '.[].name'".to_string(),
            workdir: None,
            timeout_secs: None,
            extra_env: HashMap::new(),
            credential_bindings: Vec::new(),
        };

        assert_eq!(
            request.command, "set -e; atlas resources | jq '.[].name'",
            "credential mediation must not narrow a shell invocation to one executable"
        );
    }

    #[test]
    fn shell_credential_context_parser_bounds_and_deduplicates_extension_ids() {
        assert_eq!(
            shell_credential_contexts(&serde_json::json!({
                "credential_contexts": ["atlas", "zephyrite"]
            }))
            .unwrap()
            .iter()
            .map(ExtensionId::as_str)
            .collect::<Vec<_>>(),
            ["atlas", "zephyrite"]
        );
        assert!(matches!(
            shell_credential_contexts(&serde_json::json!({
                "credential_contexts": ["atlas", "atlas"]
            })),
            Err(ShellCredentialContextError::Duplicate { .. })
        ));
        let too_many = vec!["atlas"; MAX_SHELL_CREDENTIAL_CONTEXTS + 1];
        assert!(matches!(
            shell_credential_contexts(&serde_json::json!({
                "credential_contexts": too_many
            })),
            Err(ShellCredentialContextError::TooMany { .. })
        ));
    }

    #[test]
    fn sandbox_credential_environment_names_share_the_fail_closed_contract() {
        for valid in ["GH_TOKEN", "ATLAS_API_KEY", "_PRIVATE"] {
            assert!(is_valid_sandbox_credential_env_name(valid), "{valid}");
        }
        for invalid in ["", "lowercase", "9TOKEN", "BASH_ENV", "LD_PRELOAD", "PATH"] {
            assert!(!is_valid_sandbox_credential_env_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn malformed_shell_context_keeps_its_id_validation_source() {
        let error = shell_credential_contexts(&serde_json::json!({
            "credential_contexts": ["Bad Context"]
        }))
        .expect_err("malformed extension id must fail");

        assert!(matches!(
            error,
            ShellCredentialContextError::MalformedExtensionId { .. }
        ));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn sandbox_credential_normalizes_case_insensitive_comparison_fields() {
        let credential = |approved_host: &str, header_name: &str| {
            SandboxCommandCredential::new(
                SecretHandle::new("atlas_runtime_token").unwrap(),
                "ATLAS_TOKEN".to_string(),
                "icsbx_placeholder".to_string(),
                approved_host.to_string(),
                header_name.to_string(),
                Some("token ".to_string()),
                "secret".to_string(),
            )
        };

        let lowercase = credential("api.atlas.test", "authorization");
        let mixed_case = credential("API.Atlas.TEST", "Authorization");

        assert_eq!(lowercase.approved_host, "api.atlas.test");
        assert_eq!(lowercase.header_name, "authorization");
        assert_eq!(mixed_case.approved_host, lowercase.approved_host);
        assert_eq!(mixed_case.header_name, lowercase.header_name);
    }
}
