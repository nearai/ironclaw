use crate::RuntimeProcessError;

use super::reject_nul;

#[derive(Debug, Clone)]
pub struct RebornSandboxContainerIdentity {
    user: Option<String>,
    workspace_mode: RebornSandboxWorkspaceMode,
}

impl RebornSandboxContainerIdentity {
    pub fn image_default() -> Self {
        Self {
            user: None,
            workspace_mode: RebornSandboxWorkspaceMode::Private,
        }
    }

    pub fn configured_user(
        user: impl Into<String>,
        workspace_mode: RebornSandboxWorkspaceMode,
    ) -> Self {
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
        self.workspace_mode.as_unix_mode()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_user_rejects_empty_whitespace_and_nul_values() {
        for user in ["", " \t ", "1000\0:1000"] {
            let identity = RebornSandboxContainerIdentity::configured_user(
                user,
                RebornSandboxWorkspaceMode::Private,
            );

            assert!(identity.container_user().is_err());
        }
    }

    #[test]
    fn container_user_accepts_configured_user() {
        let identity = RebornSandboxContainerIdentity::configured_user(
            "1000:1000",
            RebornSandboxWorkspaceMode::Private,
        );

        assert_eq!(
            identity.container_user().unwrap(),
            Some("1000:1000".to_string())
        );
    }
}
