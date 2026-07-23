use ironclaw_config::IronClawBootConfig;

/// Per-invocation context shared by IronClaw CLI commands.
#[derive(Debug, Clone)]
pub(crate) struct IronClawCliContext {
    boot_config: IronClawBootConfig,
}

impl IronClawCliContext {
    pub(crate) fn resolve_from_env() -> anyhow::Result<Self> {
        Ok(Self {
            boot_config: IronClawBootConfig::resolve_from_env()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_boot_config(boot_config: IronClawBootConfig) -> Self {
        Self { boot_config }
    }

    #[cfg(test)]
    pub(crate) fn test_context() -> (tempfile::TempDir, Self) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = IronClawBootConfig::resolve_from_env_parts(
            None,
            Some(tmp.path().as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("config must resolve with HOME set");
        (tmp, Self::from_boot_config(config))
    }

    pub(crate) fn boot_config(&self) -> &IronClawBootConfig {
        &self.boot_config
    }
}
