#[derive(Debug, thiserror::Error)]
pub enum RebornExtensionHostBuildError {
    #[error("invalid reborn extension-host configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error("reborn extension-host filesystem build failed")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn extension-host mount view construction failed")]
    Mount(#[from] ironclaw_host_api::error::HostApiError),
}
