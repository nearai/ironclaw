use std::path::Path;

use ironclaw_host_runtime::{
    RebornSandboxConfig, RebornScopedSandboxCommandTransport, RuntimeProcessError,
    TenantSandboxProcessPort,
};

/// Connects a real, no-broker sandbox transport for Docker-real tests, with
/// `workspace_root` bound to the per-user directory the abstract-FS
/// `/workspace` mount also points at (parity is proven by both sides
/// resolving the same host directory, not by any code sharing).
pub(crate) async fn connect_for_test(
    workspace_dir: &Path,
    image: &str,
) -> Result<TenantSandboxProcessPort, RuntimeProcessError> {
    let config =
        RebornSandboxConfig::new(workspace_dir.to_path_buf()).with_image(image.to_string());
    let transport = RebornScopedSandboxCommandTransport::connect(config).await?;
    Ok(transport.into_process_port())
}
