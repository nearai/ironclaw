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

    pub(crate) fn boot_config(&self) -> &RebornBootConfig {
        &self.boot_config
    }

    pub(crate) fn seed_config_if_missing(&self) -> anyhow::Result<()> {
        let path = self.boot_config.home().config_file_path();
        ironclaw_reborn_config::seed_default_config_file_if_missing(&path)
            .map_err(anyhow::Error::from)?;
        Ok(())
    }
}
