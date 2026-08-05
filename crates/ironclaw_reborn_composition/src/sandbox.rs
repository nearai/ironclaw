//! User-sandbox runtime construction.
//!
//! Provider-specific Docker and Railway setup stays behind this factory so the
//! rest of composition receives the same opaque runtime process binding.

use std::{path::PathBuf, sync::Arc};

use ironclaw_host_runtime::UserSandboxProcessPort;
use ironclaw_sandbox::{
    RailwayPreviewSandboxConfig, RailwayPreviewSandboxTransport, RebornSandboxConfig,
    RebornScopedSandboxCommandTransport,
};

use crate::{RebornBuildError, RebornRuntimeProcessBinding};

/// Factory for the concrete transport selected by an explicit sandbox profile.
pub struct UserSandboxFactory;

impl UserSandboxFactory {
    /// Connect the existing local Docker transport and fail profile boot if the
    /// daemon is unavailable. No caller receives an unsandboxed fallback.
    pub async fn local_docker(
        workspace_root: PathBuf,
    ) -> Result<RebornRuntimeProcessBinding, RebornBuildError> {
        let transport =
            RebornScopedSandboxCommandTransport::connect(RebornSandboxConfig::new(workspace_root))
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "user-sandbox process backend requires a reachable Docker daemon: {error}"
                    ),
                })?;
        Ok(binding(Arc::new(transport)))
    }

    /// Build the lazy Railway preview transport without contacting Railway.
    /// The first shell invocation provisions the caller's remote sandbox.
    pub fn railway_preview(config: RailwayPreviewSandboxConfig) -> RebornRuntimeProcessBinding {
        binding(Arc::new(RailwayPreviewSandboxTransport::new(config)))
    }
}

fn binding(
    transport: Arc<dyn ironclaw_host_api::process::SandboxCommandTransport>,
) -> RebornRuntimeProcessBinding {
    RebornRuntimeProcessBinding::user_sandbox(Arc::new(UserSandboxProcessPort::new(transport)))
}
