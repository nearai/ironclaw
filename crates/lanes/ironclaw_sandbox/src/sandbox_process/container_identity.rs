use std::path::Path;

use ironclaw_host_api::process::RuntimeProcessError;

use crate::sandbox_process::reject_nul;

#[derive(Debug, Clone)]
pub struct RebornSandboxContainerIdentity {
    user: RebornSandboxContainerUser,
    workspace_mode: RebornSandboxWorkspaceMode,
}

impl RebornSandboxContainerIdentity {
    pub fn workspace_owner() -> Self {
        Self {
            user: RebornSandboxContainerUser::WorkspaceOwner,
            workspace_mode: RebornSandboxWorkspaceMode::Private,
        }
    }

    pub fn configured_user(
        user: impl Into<String>,
        workspace_mode: RebornSandboxWorkspaceMode,
    ) -> Self {
        Self {
            user: RebornSandboxContainerUser::Configured(user.into()),
            workspace_mode,
        }
    }

    pub async fn container_user(&self, workspace: &Path) -> Result<String, RuntimeProcessError> {
        match &self.user {
            RebornSandboxContainerUser::WorkspaceOwner => workspace_owner_user(workspace).await,
            RebornSandboxContainerUser::Configured(user) => validate_container_user(user),
        }
    }

    pub fn workspace_mode(&self) -> u32 {
        self.workspace_mode.as_unix_mode()
    }
}

#[derive(Debug, Clone)]
enum RebornSandboxContainerUser {
    /// Match the numeric owner of the private host workspace. This keeps a
    /// native non-root IronClaw deployment writable without granting broader
    /// host permissions, while still overriding a root-default worker image.
    WorkspaceOwner,
    Configured(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebornSandboxWorkspaceMode {
    Private,
    GroupShared,
}

impl RebornSandboxWorkspaceMode {
    pub fn as_unix_mode(self) -> u32 {
        match self {
            Self::Private => 0o700,
            Self::GroupShared => 0o770,
        }
    }
}

fn validate_container_user(user: &str) -> Result<String, RuntimeProcessError> {
    reject_nul("sandbox container user", user)?;
    if user.trim().is_empty() {
        return Err(RuntimeProcessError::ExecutionFailed(
            "sandbox container user must not be empty".to_string(),
        ));
    }
    Ok(user.to_string())
}

#[cfg(unix)]
async fn workspace_owner_user(workspace: &Path) -> Result<String, RuntimeProcessError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = tokio::fs::metadata(workspace).await.map_err(|error| {
        RuntimeProcessError::ExecutionFailed(format!(
            "sandbox workspace identity could not be resolved: {error}"
        ))
    })?;
    if metadata.uid() == 0 {
        return Err(RuntimeProcessError::ExecutionFailed(
            "local sandbox workspace must be owned by a non-root IronClaw user".to_string(),
        ));
    }
    Ok(format!("{}:{}", metadata.uid(), metadata.gid()))
}

#[cfg(not(unix))]
async fn workspace_owner_user(_workspace: &Path) -> Result<String, RuntimeProcessError> {
    // Docker Desktop's Linux VM cannot consume native Windows ownership IDs.
    // The worker image and documented local setup use this explicit non-root
    // identity instead of inheriting an operator-overridable image default.
    Ok("1000:1000".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn container_user_rejects_empty_whitespace_and_nul_values() {
        for user in ["", " \t ", "1000\0:1000"] {
            let identity = RebornSandboxContainerIdentity::configured_user(
                user,
                RebornSandboxWorkspaceMode::Private,
            );

            assert!(identity.container_user(Path::new(".")).await.is_err());
        }
    }

    #[tokio::test]
    async fn container_user_accepts_configured_user() {
        let identity = RebornSandboxContainerIdentity::configured_user(
            "1000:1000",
            RebornSandboxWorkspaceMode::Private,
        );

        assert_eq!(
            identity.container_user(Path::new(".")).await.unwrap(),
            "1000:1000".to_string()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_owner_is_an_explicit_non_root_container_user() {
        let workspace = tempfile::tempdir().unwrap();
        let metadata = std::fs::metadata(workspace.path()).unwrap();
        use std::os::unix::fs::MetadataExt as _;

        if metadata.uid() == 0 {
            assert!(
                RebornSandboxContainerIdentity::workspace_owner()
                    .container_user(workspace.path())
                    .await
                    .is_err()
            );
        } else {
            assert_eq!(
                RebornSandboxContainerIdentity::workspace_owner()
                    .container_user(workspace.path())
                    .await
                    .unwrap(),
                format!("{}:{}", metadata.uid(), metadata.gid())
            );
        }
    }
}
