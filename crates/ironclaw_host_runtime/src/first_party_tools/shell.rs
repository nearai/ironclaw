use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use std::time::Duration;

use ironclaw_host_api::{
    EffectKind, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile,
    RuntimeDispatchErrorKind, SandboxQuota, ScopedPath,
};
use serde_json::{Value, json};

use crate::{
    CommandExecutionRequest, FirstPartyCapabilityError, FirstPartyCapabilityRequest,
    RuntimeProcessError, SavedCommandOutput, SavedCommandOutputSanitization,
};

use super::{FIRST_PARTY_MAX_OUTPUT_BYTES, first_party_capability_manifest};

#[path = "shell_core.rs"]
mod shell_core;

pub const SHELL_CAPABILITY_ID: &str = "builtin.shell";

const DEFAULT_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const MAX_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const MAX_SHELL_TIMEOUT_SECS: u64 = MAX_SHELL_WALL_CLOCK_MS / 1000;
const DEFAULT_SHELL_OUTPUT_BYTES: u64 = crate::process_output::COMMAND_MAX_OUTPUT_SIZE as u64;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        SHELL_CAPABILITY_ID,
        "Execute shell commands with copied v1 validation and saved-file references for large local output",
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
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(DEFAULT_SHELL_WALL_CLOCK_MS),
                output_bytes: Some(DEFAULT_SHELL_OUTPUT_BYTES),
                process_count: Some(1),
                ..ResourceEstimate::default()
            },
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
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Resource,
        ));
    }
    shell_core::validate_command(&parsed.command, false).map_err(shell_error)?;
    let output = request
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
        .map_err(process_error)?;

    let rendered_output = render_shell_output(&output.output, output.saved_output.as_ref());
    let output_value = json!({
        "output": rendered_output,
        "exit_code": output.exit_code,
        "success": output.exit_code == 0,
        "sandboxed": output.sandboxed,
    });
    Ok((output_value, output.duration))
}

fn render_shell_output(output: &str, saved_output: Option<&SavedCommandOutput>) -> String {
    let Some(saved_output) = saved_output else {
        return output.to_string();
    };
    let mut note = match saved_output.sanitization {
        SavedCommandOutputSanitization::Blocked => {
            format!(
                "Full output was not saved because it matched secret-leak blocking rules; marker saved to: {}",
                saved_output.path.display()
            )
        }
        SavedCommandOutputSanitization::Redacted => {
            format!(
                "Full output saved to: {} (secret-like values redacted)",
                saved_output.path.display()
            )
        }
        SavedCommandOutputSanitization::Clean => {
            format!("Full output saved to: {}", saved_output.path.display())
        }
    };
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
    let Some(mounts) = request
        .mounts
        .as_ref()
        .filter(|mounts| !mounts.mounts.is_empty())
    else {
        return Ok(());
    };

    let Some(workdir) = workdir else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Client,
        ));
    };
    let scoped_path = ScopedPath::new(workdir.to_string())
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?;
    let (_virtual_path, grant) = mounts
        .resolve_with_grant(&scoped_path)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Client))?;
    if !grant.permissions.execute {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::Client,
        ));
    }

    // Shell execution still uses the local process fallback. Until the resolved
    // process backend can receive virtual cwd + scoped mounts, fail closed rather
    // than translating scoped paths to ambient host paths in this handler.
    Err(FirstPartyCapabilityError::new(
        RuntimeDispatchErrorKind::Client,
    ))
}

fn shell_error(error: shell_core::ShellExecutionError) -> FirstPartyCapabilityError {
    let kind = match error {
        shell_core::ShellExecutionError::InvalidParameters(_) => {
            RuntimeDispatchErrorKind::InputEncode
        }
        shell_core::ShellExecutionError::NotAuthorized(_) => RuntimeDispatchErrorKind::Client,
    };
    FirstPartyCapabilityError::new(kind)
}

fn process_error(error: RuntimeProcessError) -> FirstPartyCapabilityError {
    let kind = match error {
        RuntimeProcessError::Timeout(_) => RuntimeDispatchErrorKind::Resource,
        RuntimeProcessError::ExecutionFailed(_) => RuntimeDispatchErrorKind::Executor,
    };
    FirstPartyCapabilityError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_shell_output_preserves_unsaved_output() {
        assert_eq!(render_shell_output("hello", None), "hello");
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
        );

        assert!(rendered.contains("Full output saved to: /tmp/command.log"));
        assert!(rendered.contains("secret-like values redacted"));
    }

    #[test]
    fn render_shell_output_reports_blocked_saved_output() {
        let rendered = render_shell_output(
            "preview",
            Some(&SavedCommandOutput {
                sanitization: SavedCommandOutputSanitization::Blocked,
                ..saved_output()
            }),
        );

        assert!(rendered.contains("Full output was not saved because"));
        assert!(rendered.contains("marker saved to: /tmp/command.log"));
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
        );

        assert!(rendered.contains("saved output capped at 123 bytes per stream"));
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
