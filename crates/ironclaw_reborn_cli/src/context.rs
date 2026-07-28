use ironclaw_reborn_config::RebornBootConfig;

/// Per-invocation context shared by Reborn CLI commands.
#[derive(Debug, Clone)]
pub(crate) struct RebornCliContext {
    boot_config: RebornBootConfig,
}

impl RebornCliContext {
    pub(crate) fn resolve_from_env() -> anyhow::Result<Self> {
        Ok(Self {
            boot_config: RebornBootConfig::resolve_from_env()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_boot_config(boot_config: RebornBootConfig) -> Self {
        Self { boot_config }
    }

    #[cfg(test)]
    pub(crate) fn test_context() -> (tempfile::TempDir, Self) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = RebornBootConfig::resolve_from_env_parts(
            None,
            Some(tmp.path().as_os_str().to_os_string()),
            None,
            None,
        )
        .expect("config must resolve with HOME set");
        (tmp, Self::from_boot_config(config))
    }

    pub(crate) fn boot_config(&self) -> &RebornBootConfig {
        &self.boot_config
    }
}
