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
}
