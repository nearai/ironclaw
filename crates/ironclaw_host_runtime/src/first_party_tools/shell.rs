use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    EffectKind, PermissionMode, ResourceCeiling, ResourceEstimate, ResourceProfile, ResourceUsage,
    RuntimeDispatchErrorKind, SandboxQuota,
};
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{FIRST_PARTY_MAX_OUTPUT_BYTES, first_party_capability_manifest};

#[path = "shell_core.rs"]
mod shell_core;

pub const SHELL_CAPABILITY_ID: &str = "builtin.shell";

const DEFAULT_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const MAX_SHELL_WALL_CLOCK_MS: u64 = 120_000;
const DEFAULT_SHELL_OUTPUT_BYTES: u64 = shell_core::MAX_OUTPUT_SIZE as u64;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        SHELL_CAPABILITY_ID,
        "Execute shell commands with the copied v1 shell validation and output shape",
        vec![
            EffectKind::DispatchCapability,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ],
        PermissionMode::Ask,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate {
                wall_clock_ms: Some(DEFAULT_SHELL_WALL_CLOCK_MS),
                output_bytes: Some(DEFAULT_SHELL_OUTPUT_BYTES),
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
) -> Result<(Value, ResourceUsage), FirstPartyCapabilityError> {
    let parsed = shell_core::parse_shell_request(&request.input).map_err(shell_error)?;
    let output = shell_core::ShellExecutor::new()
        .execute_direct(shell_core::ShellExecutionRequest {
            extra_env: Default::default(),
            ..parsed
        })
        .await
        .map_err(shell_error)?;

    let output_value = json!({
        "output": output.output,
        "exit_code": output.exit_code,
        "success": output.success,
        "sandboxed": output.sandboxed,
    });
    let output_bytes = serde_json::to_vec(&output_value)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OutputDecode))?;
    Ok((
        output_value,
        ResourceUsage {
            wall_clock_ms: output.duration.as_millis().try_into().unwrap_or(u64::MAX),
            output_bytes,
            ..ResourceUsage::default()
        },
    ))
}

fn shell_error(error: shell_core::ShellExecutionError) -> FirstPartyCapabilityError {
    let kind = match error {
        shell_core::ShellExecutionError::InvalidParameters(_) => {
            RuntimeDispatchErrorKind::InputEncode
        }
        shell_core::ShellExecutionError::NotAuthorized(_) => RuntimeDispatchErrorKind::Client,
        shell_core::ShellExecutionError::Timeout(_) => RuntimeDispatchErrorKind::Resource,
        shell_core::ShellExecutionError::ExecutionFailed(_) => RuntimeDispatchErrorKind::Executor,
    };
    FirstPartyCapabilityError::new(kind)
}
