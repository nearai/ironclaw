use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_filesystem::FilesystemError;
use std::time::Duration;

use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode, RuntimeCredentialRequirement},
    dispatch::RuntimeDispatchErrorKind,
    http::RuntimeCredentialTarget,
    path::{ScopedPath, VirtualPath},
    process::{
        CommandExecutionRequest, CredentialedSandboxCommandRequest, RuntimeProcessError,
        SandboxCommandCredentialBinding, SavedCommandOutput, SavedCommandOutputSanitization,
    },
    resource::{ResourceCeiling, ResourceEstimate, ResourceProfile, SandboxQuota},
};
use serde_json::{Value, json};

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityRequest, process_output::saved_output_filename,
};

use super::{FIRST_PARTY_MAX_OUTPUT_BYTES, first_party_capability_manifest};

#[path = "shell_core.rs"]
mod shell_core;

pub const SHELL_CAPABILITY_ID: &str = "builtin.shell";

const DEFAULT_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const MAX_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const MAX_SHELL_TIMEOUT_SECS: u64 = MAX_SHELL_WALL_CLOCK_MS / 1000;
const DEFAULT_SHELL_OUTPUT_BYTES: u64 = crate::process_output::COMMAND_MAX_OUTPUT_SIZE as u64;
const SAVED_OUTPUT_SCOPED_DIR: &str = "command-outputs";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        SHELL_CAPABILITY_ID,
        "Execute shell commands with copied v1 validation and saved-file references for large local output. Prefer computing the answer in the pipeline (`grep | sort | uniq -c`, `awk`) over reading raw data into the conversation — return conclusions, not content.",
        vec![
            EffectKind::DispatchCapability,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default()
                .set_wall_clock_ms(DEFAULT_SHELL_WALL_CLOCK_MS)
                .set_output_bytes(DEFAULT_SHELL_OUTPUT_BYTES)
                .set_process_count(1),
            hard_ceiling: Some(ResourceCeiling {
                max_usd: None,
                max_input_tokens: None,
                max_output_tokens: None,
                max_wall_clock_ms: Some(MAX_SHELL_WALL_CLOCK_MS),
                max_output_bytes: Some(FIRST_PARTY_MAX_OUTPUT_BYTES),
                sandbox: Some(SandboxQuota {
                    process_count: Some(1),
                    ..SandboxQuota::default()
                }),
            }),
        }),
    )
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<(Value, Duration), FirstPartyCapabilityError> {
    let parsed = shell_core::parse_shell_request(&request.input).map_err(shell_error)?;
    reject_unbacked_scoped_workdir(request, parsed.workdir.as_deref())?;
    if parsed
        .timeout_secs
        .is_some_and(|timeout_secs| timeout_secs > MAX_SHELL_TIMEOUT_SECS)
    {
        // Name the ceiling. The model can only lower its `timeout` when the
        // failure says what the maximum is; a bare `resource` category makes
        // it retry the same over-limit value.
        return Err(FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            format!(
                "requested timeout exceeds the maximum of {MAX_SHELL_TIMEOUT_SECS}s for this capability"
            ),
        ));
    }
    shell_core::validate_command(&parsed.command, false).map_err(shell_error)?;
    let output = if request.runtime_credentials.is_empty() {
        request
            .services
            .process
            .run_command(CommandExecutionRequest {
                scope: request.scope.clone(),
                mounts: request.mounts.clone(),
                command: parsed.command,
                workdir: parsed.workdir,
                timeout_secs: parsed.timeout_secs,
                extra_env: parsed.extra_env,
            })
            .await
    } else {
        if !request.services.process.supports_credentialed_command() {
            return Err(process_error(RuntimeProcessError::ExecutionFailed(
                "authorized shell credentials require managed sandbox egress".to_string(),
            )));
        }
        let bindings = credential_bindings(&request.runtime_credentials).map_err(process_error)?;
        request
            .services
            .process
            .run_credentialed_command(
                CredentialedSandboxCommandRequest {
                    capability_id: request.capability_id.clone(),
                    scope: request.scope.clone(),
                    mounts: request.mounts.clone(),
                    command: parsed.command,
                    workdir: parsed.workdir,
                    timeout_secs: parsed.timeout_secs,
                    extra_env: parsed.extra_env,
                    credential_bindings: bindings,
                },
                Vec::new(),
            )
            .await
    }
    .map_err(process_error)?;

    let saved_output_path =
        publish_saved_output_for_file_read(request, output.saved_output.as_ref()).await?;
    let rendered_output = render_shell_output(
        &output.output,
        output.saved_output.as_ref(),
        saved_output_path.as_deref(),
    );
    let output_value = json!({
        "output": rendered_output,
        "exit_code": output.exit_code,
        "success": output.exit_code == 0,
        "sandboxed": output.sandboxed,
    });
    Ok((output_value, output.duration))
}

async fn publish_saved_output_for_file_read(
    request: &FirstPartyCapabilityRequest,
    saved_output: Option<&SavedCommandOutput>,
) -> Result<Option<String>, FirstPartyCapabilityError> {
    let Some(saved_output) = saved_output else {
        return Ok(None);
    };
    let cleanup_saved_output = |context: &'static str| async move {
        if let Err(error) = tokio::fs::remove_file(&saved_output.path).await {
            tracing::debug!(
                ?error,
                context,
                "best-effort cleanup of saved output failed"
            );
        }
    };
    let Some((scoped_path, virtual_path)) = saved_output_publish_path(request, saved_output) else {
        cleanup_saved_output("unpublished").await;
        return Ok(None);
    };
    // Every leg below names what failed. A bare `operation_failed` told the
    // model only that "something" broke while publishing the saved output, so
    // it re-ran the identical command and hit the identical fault.
    let content = tokio::fs::read(&saved_output.path)
        .await
        .map_err(|error| saved_output_error("read saved command output", &error))?;
    if let Some(parent) = virtual_parent(&virtual_path) {
        match request.services.filesystem.create_dir_all(&parent).await {
            Ok(()) | Err(FilesystemError::Unsupported { .. }) => {}
            Err(error) => {
                return Err(saved_output_error(
                    "create saved command output directory",
                    &error,
                ));
            }
        }
    }
    request
        .services
        .filesystem
        .write_file(&virtual_path, &content)
        .await
        .map_err(|error| saved_output_error("write saved command output", &error))?;
    cleanup_saved_output("published").await;
    Ok(Some(scoped_path))
}

fn saved_output_publish_path(
    request: &FirstPartyCapabilityRequest,
    saved_output: &SavedCommandOutput,
) -> Option<(String, VirtualPath)> {
    let mounts = request.mounts.as_ref()?;
    let grant = mounts
        .mounts
        .iter()
        .find(|grant| grant.permissions.read && grant.permissions.write)?;
    let filename = saved_output_filename(saved_output);
    let relative_path = format!("{SAVED_OUTPUT_SCOPED_DIR}/{filename}");
    let scoped_path = format!(
        "{}/{}",
        grant.alias.as_str().trim_end_matches('/'),
        relative_path
    );
    let virtual_path = mounts
        .scoped_path(scoped_path.clone())
        .and_then(|path| mounts.resolve(&path))
        .ok()?;
    Some((scoped_path, virtual_path))
}

fn virtual_parent(path: &VirtualPath) -> Option<VirtualPath> {
    let (parent, _) = path.as_str().rsplit_once('/')?;
    if parent.is_empty() {
        None
    } else {
        VirtualPath::new(parent.to_string()).ok()
    }
}

fn render_shell_output(
    output: &str,
    saved_output: Option<&SavedCommandOutput>,
    saved_output_path: Option<&str>,
) -> String {
    let Some(saved_output) = saved_output else {
        return output.to_string();
    };
    let Some(saved_output_path) = saved_output_path else {
        return format!(
            "{output}\n\nFull output was captured but no file_read-accessible scoped path was available"
        );
    };
    let mut note = match saved_output.sanitization {
        SavedCommandOutputSanitization::Blocked => {
            format!(
                "Full output was not saved because it matched secret-leak blocking rules; marker saved to: {saved_output_path}"
            )
        }
        SavedCommandOutputSanitization::Redacted => {
            format!("Full output saved to: {saved_output_path} (secret-like values redacted)")
        }
        SavedCommandOutputSanitization::Clean => {
            format!("Full output saved to: {saved_output_path}")
        }
    };
    note.push_str("\nUse file_read to inspect it");
    if saved_output.stream_was_capped {
        note.push_str(&format!(
            " (saved output capped at {} bytes per stream)",
            saved_output.max_saved_stream_size
        ));
    }
    format!("{output}\n\n{note}")
}

fn reject_unbacked_scoped_workdir(
    request: &FirstPartyCapabilityRequest,
    workdir: Option<&str>,
) -> Result<(), FirstPartyCapabilityError> {
    let Some(workdir) = workdir else {
        return Ok(());
    };
    let Some(mounts) = request
        .mounts
        .as_ref()
        .filter(|mounts| !mounts.mounts.is_empty())
    else {
        return Ok(());
    };
    // Every rejection below is model-correctable — it can drop `workdir` and
    // `cd` inside the command instead — but only if the failure says which
    // rule it broke. A bare `client`/`input_encode` category left it retrying
    // the same workdir.
    let scoped_path = ScopedPath::new(workdir.to_string()).map_err(|error| {
        FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::InputEncode,
            bounded_failure_reason(format!("workdir is not a valid scoped path: {error}")),
        )
    })?;
    let (_virtual_path, grant) = mounts.resolve_with_grant(&scoped_path).map_err(|error| {
        FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Client,
            bounded_failure_reason(format!(
                "workdir does not resolve to a granted mount: {error}"
            )),
        )
    })?;
    if !grant.permissions.execute {
        return Err(FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Client,
            "workdir resolves to a mount without execute permission",
        ));
    }

    // Shell execution still uses the local process fallback. Until the resolved
    // process backend can receive virtual cwd + scoped mounts, fail closed rather
    // than translating scoped paths to ambient host paths in this handler.
    Err(FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::Client,
        "scoped workdir is not supported by this shell backend; omit workdir and cd inside the command instead",
    ))
}

/// Bound a failure reason before it becomes the dispatch safe summary.
///
/// Truncation counts chars (never bytes), so multibyte input cannot split a
/// code point. No other sanitization happens here on purpose: downstream,
/// `failure_from` (production.rs) validates the reason against the strict
/// loop safe-summary rules — reasons that pass ride the summary, and reasons
/// that fail (paths, newlines) are preserved on the model-visible diagnostic
/// detail, where secret values are scrubbed and control characters
/// normalized at the loop boundary.
fn bounded_failure_reason(reason: String) -> String {
    const MAX_CHARS: usize = 512;
    if reason.chars().count() <= MAX_CHARS {
        return reason;
    }
    let bounded: String = reason.chars().take(MAX_CHARS - 3).collect();
    format!("{bounded}...")
}

fn shell_error(error: shell_core::ShellExecutionError) -> FirstPartyCapabilityError {
    // Carry the reason: the model can only repair its call (fix a parameter,
    // pick another approach) when the failure says what went wrong.
    let (kind, reason) = match error {
        shell_core::ShellExecutionError::InvalidParameters(reason) => {
            (RuntimeDispatchErrorKind::InputEncode, reason)
        }
        shell_core::ShellExecutionError::NotAuthorized(reason) => {
            (RuntimeDispatchErrorKind::PolicyDenied, reason)
        }
    };
    FirstPartyCapabilityError::with_safe_summary(kind, bounded_failure_reason(reason))
}

/// Name the failing leg of saved-output publication instead of collapsing every
/// filesystem fault into a bare `operation_failed`.
///
/// `error` is untrusted host text (it carries paths). It rides the same channel
/// as `shell_error`'s reasons: strict-validated as a safe summary downstream,
/// and preserved on the scrubbed model-visible diagnostic detail when it fails
/// that validation.
fn saved_output_error(
    leg: &'static str,
    error: &dyn std::fmt::Display,
) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::OperationFailed,
        bounded_failure_reason(format!("failed to {leg}: {error}")),
    )
}

fn credential_bindings(
    requirements: &[RuntimeCredentialRequirement],
) -> Result<Vec<SandboxCommandCredentialBinding>, RuntimeProcessError> {
    let bindings = requirements
        .iter()
        .filter_map(|requirement| {
            let placeholder_env = requirement.placeholder_env.as_ref()?;
            if !matches!(requirement.target, RuntimeCredentialTarget::Header { .. }) {
                return None;
            }
            Some(SandboxCommandCredentialBinding {
                placeholder_env: placeholder_env.clone(),
                requirement: requirement.clone(),
            })
        })
        .collect::<Vec<_>>();
    if bindings.len() != requirements.len() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "authorized shell credentials require placeholder environments and header targets"
                .to_string(),
        ));
    }
    Ok(bindings)
}

fn process_error(error: RuntimeProcessError) -> FirstPartyCapabilityError {
    let (kind, reason) = match error {
        RuntimeProcessError::Timeout(duration) => (
            RuntimeDispatchErrorKind::Resource,
            format!("shell command timed out after {}s", duration.as_secs()),
        ),
        RuntimeProcessError::ExecutionFailed(reason) => (
            RuntimeDispatchErrorKind::Executor,
            format!("shell execution failed: {reason}"),
        ),
    };
    FirstPartyCapabilityError::with_safe_summary(kind, bounded_failure_reason(reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use parking_lot::Mutex;

    use ironclaw_host_api::{
        action::{NetworkScheme, NetworkTargetPattern},
        capability::{RuntimeCredentialRequirement, RuntimeCredentialRequirementSource},
        ids::{CapabilityId, SecretHandle},
        process::{CommandExecutionOutput, SandboxCommandCredential},
        resource::ResourceScope,
    };

    #[derive(Debug, Default)]
    struct RecordingProcessPort {
        credentialed_requests: Mutex<Vec<CredentialedSandboxCommandRequest>>,
    }

    #[async_trait::async_trait]
    impl crate::RuntimeProcessPort for RecordingProcessPort {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed(
                "unexpected shell fallback".to_string(),
            ))
        }

        fn supports_credentialed_command(&self) -> bool {
            true
        }

        async fn run_credentialed_command(
            &self,
            request: CredentialedSandboxCommandRequest,
            credentials: Vec<SandboxCommandCredential>,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            assert!(credentials.is_empty());
            self.credentialed_requests.lock().push(request);
            Ok(CommandExecutionOutput {
                output: "[]".to_string(),
                saved_output: None,
                exit_code: 0,
                sandboxed: true,
                duration: Duration::from_millis(1),
            })
        }
    }

    #[derive(Debug, Default)]
    struct OrdinaryOnlyProcessPort {
        requests: Mutex<Vec<CommandExecutionRequest>>,
    }

    #[async_trait::async_trait]
    impl crate::RuntimeProcessPort for OrdinaryOnlyProcessPort {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            self.requests.lock().push(request);
            Err(RuntimeProcessError::ExecutionFailed(
                "ordinary process path reached".to_string(),
            ))
        }
    }

    fn dispatch_safe_summary(error: &FirstPartyCapabilityError) -> Option<&str> {
        match error {
            FirstPartyCapabilityError::Dispatch { safe_summary, .. } => safe_summary.as_deref(),
            FirstPartyCapabilityError::AuthRequired { .. } => None,
        }
    }

    #[test]
    fn shell_error_carries_the_invalid_parameter_reason() {
        // The model must see WHY its shell call was rejected (e.g. which
        // parameter was missing); a bare input-encode category leaves it
        // retrying the identical call blind.
        let error = shell_error(shell_core::ShellExecutionError::InvalidParameters(
            "missing 'command' parameter".to_string(),
        ));

        let summary = dispatch_safe_summary(&error).expect("reason must be carried");
        assert!(
            summary.contains("missing 'command' parameter"),
            "summary should carry the parameter reason, got: {summary}"
        );
    }

    #[test]
    fn process_error_carries_timeout_and_execution_reasons() {
        let timeout = process_error(RuntimeProcessError::Timeout(Duration::from_secs(30)));
        let summary = dispatch_safe_summary(&timeout).expect("timeout reason must be carried");
        assert!(
            summary.contains("timed out"),
            "summary should describe the timeout, got: {summary}"
        );

        let failed = process_error(RuntimeProcessError::ExecutionFailed(
            "spawn failed: no such file".to_string(),
        ));
        let summary = dispatch_safe_summary(&failed).expect("execution reason must be carried");
        assert!(
            summary.contains("spawn failed: no such file"),
            "summary should carry the execution failure reason, got: {summary}"
        );
    }

    #[test]
    fn shell_error_preserves_paths_and_newlines_for_the_diagnostic_channel() {
        // The producer must NOT pre-sanitize: reasons carrying paths or
        // newlines fail the strict loop safe-summary validator downstream and
        // are then preserved verbatim on the model-visible diagnostic detail
        // (`failure_from` in production.rs), where secret values are scrubbed
        // and control characters normalized at the loop boundary. Stripping
        // them here would blind the diagnostic.
        let error = shell_error(shell_core::ShellExecutionError::NotAuthorized(
            "Blocked sensitive file access: cat /etc/passwd\nsecond line".to_string(),
        ));

        let summary = dispatch_safe_summary(&error).expect("reason must be carried");
        assert!(
            summary.contains("/etc/passwd"),
            "the concrete path must be preserved for the diagnostic, got: {summary}"
        );
        assert!(
            summary.contains('\n'),
            "newlines must be preserved for the diagnostic, got: {summary:?}"
        );
    }

    #[test]
    fn bounded_failure_reason_keeps_exactly_512_chars_intact() {
        let input = "x".repeat(512);

        assert_eq!(bounded_failure_reason(input.clone()), input);
    }

    #[test]
    fn bounded_failure_reason_truncates_513_chars_to_512_with_ellipsis() {
        let bounded = bounded_failure_reason("x".repeat(513));

        assert_eq!(bounded.chars().count(), 512);
        assert!(bounded.ends_with("..."));
        assert!(bounded.starts_with(&"x".repeat(509)));
    }

    #[test]
    fn bounded_failure_reason_truncates_multibyte_input_on_char_boundaries() {
        // 513 three-byte chars: byte-index truncation would panic or split a
        // code point; char-based truncation must stay on boundaries.
        let bounded = bounded_failure_reason("界".repeat(513));

        assert_eq!(bounded.chars().count(), 512);
        assert!(bounded.ends_with("..."));
        assert_eq!(
            bounded.chars().take(509).collect::<String>(),
            "界".repeat(509)
        );

        // Exactly at the limit, multibyte input is kept intact.
        let exact = "界".repeat(512);
        assert_eq!(bounded_failure_reason(exact.clone()), exact);
    }

    #[test]
    fn render_shell_output_preserves_unsaved_output() {
        assert_eq!(render_shell_output("hello", None, None), "hello");
    }

    #[test]
    fn render_shell_output_reports_redacted_saved_output() {
        let saved = saved_output();

        let rendered = render_shell_output(
            "preview",
            Some(&SavedCommandOutput {
                sanitization: SavedCommandOutputSanitization::Redacted,
                ..saved
            }),
            Some("/workspace/command-outputs/command.log"),
        );

        assert!(rendered.contains("Full output saved to: /workspace/command-outputs/command.log"));
        assert!(!rendered.contains("/tmp/command.log"));
        assert!(rendered.contains("secret-like values redacted"));
        assert!(rendered.contains("Use file_read to inspect it"));
    }

    #[test]
    fn render_shell_output_reports_blocked_saved_output() {
        let rendered = render_shell_output(
            "preview",
            Some(&SavedCommandOutput {
                sanitization: SavedCommandOutputSanitization::Blocked,
                ..saved_output()
            }),
            Some("/workspace/command-outputs/command.log"),
        );

        assert!(rendered.contains("Full output was not saved because"));
        assert!(rendered.contains("marker saved to: /workspace/command-outputs/command.log"));
        assert!(!rendered.contains("/tmp/command.log"));
    }

    #[test]
    fn render_shell_output_reports_stream_cap() {
        let rendered = render_shell_output(
            "preview",
            Some(&SavedCommandOutput {
                stream_was_capped: true,
                max_saved_stream_size: 123,
                ..saved_output()
            }),
            Some("/workspace/command-outputs/command.log"),
        );

        assert!(rendered.contains("saved output capped at 123 bytes per stream"));
    }

    #[test]
    fn generic_adapter_maps_declared_requirement_to_binding() {
        let requirement = atlas_requirement();
        let bindings = credential_bindings(std::slice::from_ref(&requirement)).unwrap();

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].placeholder_env, "ATLAS_TOKEN");
        assert_eq!(bindings[0].requirement, requirement);
    }

    #[tokio::test]
    async fn dispatch_preserves_compound_command_for_credentialed_shell() {
        let process = Arc::new(RecordingProcessPort::default());
        let command = "set -e; atlas resources list | jq '.items'; atlas audit";
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
            ResourceScope::system(),
            json!({
                "command": command,
                "credential_contexts": ["atlas"]
            }),
            None,
        );
        request.runtime_credentials = vec![atlas_requirement()];
        request.services.process = process.clone();

        dispatch(&request).await.unwrap();

        let requests = process.credentialed_requests.lock();
        let credentialed = &requests[0];
        assert_eq!(credentialed.capability_id, request.capability_id);
        assert_eq!(credentialed.command, command);
        assert!(credentialed.extra_env.is_empty());
        assert_eq!(credentialed.credential_bindings.len(), 1);
        assert_eq!(
            credentialed.credential_bindings[0].placeholder_env,
            "ATLAS_TOKEN"
        );
        assert_eq!(
            credentialed.credential_bindings[0].requirement,
            request.runtime_credentials[0]
        );
    }

    #[tokio::test]
    async fn dispatch_requires_managed_egress_for_credential_contexts() {
        let process = Arc::new(OrdinaryOnlyProcessPort::default());
        let mut request = FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
            ResourceScope::system(),
            json!({
                "command": "atlas resources list",
                "credential_contexts": ["atlas"]
            }),
            None,
        );
        request.runtime_credentials = vec![atlas_requirement()];
        request.services.process = process.clone();

        let error = dispatch(&request).await.unwrap_err();

        assert!(dispatch_safe_summary(&error).is_some_and(|summary| {
            summary.contains("authorized shell credentials require managed sandbox egress")
                && !summary.contains("not staged")
        }));
        assert!(process.requests.lock().is_empty());
    }

    fn atlas_requirement() -> RuntimeCredentialRequirement {
        RuntimeCredentialRequirement {
            handle: SecretHandle::new("atlas_runtime_token").unwrap(),
            source: RuntimeCredentialRequirementSource::SecretHandle,
            provider_scopes: Vec::new(),
            audience: NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "api.atlas.test".to_string(),
                port: None,
            },
            target: RuntimeCredentialTarget::Header {
                name: "authorization".to_string(),
                prefix: Some("Bearer ".to_string()),
            },
            placeholder_env: Some("ATLAS_TOKEN".to_string()),
            required: true,
        }
    }

    fn saved_output() -> SavedCommandOutput {
        SavedCommandOutput {
            path: std::path::PathBuf::from("/tmp/command.log"),
            sanitization: SavedCommandOutputSanitization::Clean,
            stream_was_capped: false,
            max_saved_stream_size: 16,
            expires_at_unix_secs: 1,
        }
    }
}
