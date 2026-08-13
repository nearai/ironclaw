//! Host-runtime adapter for the sandbox workspace-copy executor.

use std::{sync::Arc, time::Instant};

use async_trait::async_trait;
use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_extension_support::sandbox_workspace_copy::{
    SandboxWorkspaceCopyError, SandboxWorkspaceCopyRequest, execute,
};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    process::SandboxWorkspaceFileTransport,
    resource::{ResourceEstimate, ResourceProfile, ResourceUsage},
};
use serde_json::json;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRequest,
    FirstPartyCapabilityResult,
};

use super::first_party_capability_manifest;

pub const SANDBOX_WORKSPACE_COPY_CAPABILITY_ID: &str = "builtin.sandbox_workspace_copy";
pub const MAX_SANDBOX_WORKSPACE_COPY_BYTES: usize =
    ironclaw_extension_support::sandbox_workspace_copy::MAX_SANDBOX_WORKSPACE_COPY_BYTES;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        SANDBOX_WORKSPACE_COPY_CAPABILITY_ID,
        "Copy one bounded regular file between the IronClaw workspace and the separate sandbox /workspace. This copies only; it does not move, delete, recursively copy, or continuously sync files.",
        vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::SpawnProcess,
        ],
        PermissionMode::Allow,
        Some(ResourceProfile {
            default_estimate: ResourceEstimate::default().set_process_count(1),
            hard_ceiling: None,
        }),
    )
}

pub(super) struct SandboxWorkspaceCopyHandler {
    transport: Arc<dyn SandboxWorkspaceFileTransport>,
}

impl SandboxWorkspaceCopyHandler {
    pub(super) fn new(transport: Arc<dyn SandboxWorkspaceFileTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl FirstPartyCapabilityHandler for SandboxWorkspaceCopyHandler {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
        let started = Instant::now();
        let executor_request = SandboxWorkspaceCopyRequest {
            scope: &request.scope,
            mounts: request.mounts.as_ref(),
            filesystem: Arc::clone(&request.services.filesystem),
            transport: Arc::clone(&self.transport),
            input: &request.input,
        };
        let output = execute(&executor_request)
            .await
            .map_err(map_workspace_copy_error)?;
        let wall_clock_ms = started.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

        Ok(FirstPartyCapabilityResult::new(
            json!({
                "direction": output.direction,
                "source_path": output.source_path,
                "destination_path": output.destination_path,
                "bytes_copied": output.bytes_copied,
                "sha256": output.sha256,
                "already_present": output.already_present,
                "overwrite_enabled": output.overwrite_enabled,
            }),
            ResourceUsage::default()
                .set_wall_clock_ms(wall_clock_ms)
                .set_process_count(1),
        ))
    }
}

fn map_workspace_copy_error(error: SandboxWorkspaceCopyError) -> FirstPartyCapabilityError {
    tracing::debug!(?error, "sandbox workspace copy executor failed");
    match error.safe_summary() {
        Some(summary) => FirstPartyCapabilityError::with_safe_summary(error.kind(), summary),
        None => FirstPartyCapabilityError::new(error.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::{
        process::{
            SandboxWorkspaceFileError, SandboxWorkspaceFileReadOutput,
            SandboxWorkspaceFileReadRequest, SandboxWorkspaceFileWriteOutput,
            SandboxWorkspaceFileWriteRequest,
        },
        resource::ResourceScope,
    };

    struct FailingTransport;

    #[async_trait]
    impl SandboxWorkspaceFileTransport for FailingTransport {
        async fn read_file(
            &self,
            _request: SandboxWorkspaceFileReadRequest,
        ) -> Result<SandboxWorkspaceFileReadOutput, SandboxWorkspaceFileError> {
            Err(SandboxWorkspaceFileError::TransportFailed)
        }

        async fn write_file(
            &self,
            _request: SandboxWorkspaceFileWriteRequest,
        ) -> Result<SandboxWorkspaceFileWriteOutput, SandboxWorkspaceFileError> {
            Err(SandboxWorkspaceFileError::TransportFailed)
        }
    }

    #[tokio::test]
    async fn adapter_preserves_executor_error_kind_and_safe_summary() {
        let handler = SandboxWorkspaceCopyHandler::new(Arc::new(FailingTransport));
        let request = FirstPartyCapabilityRequest::request_for_test(
            ironclaw_host_api::ids::CapabilityId::new(SANDBOX_WORKSPACE_COPY_CAPABILITY_ID)
                .expect("capability id"),
            ResourceScope::system(),
            json!({"direction": "invalid"}),
            None,
        );

        let error = handler
            .dispatch(request)
            .await
            .expect_err("invalid executor input must fail");

        assert_eq!(
            error.kind(),
            Some(ironclaw_host_api::dispatch::RuntimeDispatchErrorKind::InputEncode)
        );
        assert_eq!(
            error.safe_summary(),
            Some("workspace copy input is invalid")
        );
    }
}
