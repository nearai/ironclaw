use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_filesystem::FilesystemError;
use std::time::Duration;

use ironclaw_host_api::{
    EffectKind, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile,
    RuntimeDispatchErrorKind, SandboxQuota, ScopedPath, VirtualPath,
};
use serde_json::{Value, json};

use crate::{
    CommandExecutionRequest, FirstPartyCapabilityError, FirstPartyCapabilityRequest,
    RuntimeProcessError, SavedCommandOutput, SavedCommandOutputSanitization,
    process_output::saved_output_filename,
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
    let Some((scoped_path, virtual_path)) = saved_output_publish_path(request, saved_output) else {
        let _ = tokio::fs::remove_file(&saved_output.path).await;
        return Ok(None);
    };
    let content = tokio::fs::read(&saved_output.path)
        .await
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    if let Some(parent) = virtual_parent(&virtual_path) {
        match request.services.filesystem.create_dir_all(&parent).await {
            Ok(()) | Err(FilesystemError::Unsupported { .. }) => {}
            Err(_) => {
                return Err(FirstPartyCapabilityError::new(
                    RuntimeDispatchErrorKind::OperationFailed,
                ));
            }
        }
    }
    request
        .services
        .filesystem
        .write_file(&virtual_path, &content)
        .await
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed))?;
    let _ = tokio::fs::remove_file(&saved_output.path).await;
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
        assert!(rendered.contains("use file_read to inspect it"));
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
