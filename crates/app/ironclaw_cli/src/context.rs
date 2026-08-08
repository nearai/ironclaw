use ironclaw_config::RebornBootConfig;

/// Per-invocation context shared by Reborn CLI commands.
#[derive(Debug, Clone)]
pub(crate) struct RebornCliContext {
    boot_config: RebornBootConfig,
    ambient_proxy_present: bool,
}

impl RebornCliContext {
    pub(crate) fn resolve_from_env() -> anyhow::Result<Self> {
        let ambient_proxy_present = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
        .into_iter()
        .any(|name| {
            std::env::var_os(name)
                .map(|value| !value.as_os_str().is_empty())
                .unwrap_or(false)
        });
        Ok(Self {
            boot_config: RebornBootConfig::resolve_from_env()?,
            ambient_proxy_present,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_boot_config(boot_config: RebornBootConfig) -> Self {
        Self {
            boot_config,
            ambient_proxy_present: false,
        }
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

    pub(crate) fn ambient_proxy_present(&self) -> bool {
        self.ambient_proxy_present
    }
}
