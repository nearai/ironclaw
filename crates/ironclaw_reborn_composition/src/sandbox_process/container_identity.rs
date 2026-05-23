use ironclaw_host_runtime::RuntimeProcessError;

use super::reject_nul;

#[derive(Debug, Clone)]
pub struct RebornSandboxContainerIdentity {
    user: Option<String>,
    workspace_mode: u32,
}

impl RebornSandboxContainerIdentity {
    pub fn image_default() -> Self {
        Self {
            user: None,
            workspace_mode: 0o700,
        }
    }

    pub fn configured_user(user: impl Into<String>, workspace_mode: u32) -> Self {
        Self {
            user: Some(user.into()),
            workspace_mode,
        }
    }

    pub fn container_user(&self) -> Result<Option<String>, RuntimeProcessError> {
        self.user
            .as_deref()
            .map(validate_container_user)
            .transpose()
    }

    pub fn workspace_mode(&self) -> u32 {
        self.workspace_mode
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
