use ironclaw_reborn_config::RebornBootConfig;
use std::path::PathBuf;

/// Non-secret evidence that a v1 source may be available for migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum V1MigrationSourceCandidate {
    LibSql(PathBuf),
    /// The URL remains in `MIGRATION_SOURCE_POSTGRES`; only its presence is
    /// represented here so it cannot leak into CLI diagnostics or argv.
    PostgresEnvironment,
}

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

    pub(crate) fn v1_migration_source_candidate(&self) -> Option<V1MigrationSourceCandidate> {
        if std::env::var_os("MIGRATION_SOURCE_POSTGRES").is_some_and(|value| !value.is_empty()) {
            return Some(V1MigrationSourceCandidate::PostgresEnvironment);
        }

        let v1_base = self.v1_source_home()?;
        let database = v1_base.join("ironclaw.db");
        database
            .is_file()
            .then_some(V1MigrationSourceCandidate::LibSql(database))
    }

    pub(crate) fn v1_source_home(&self) -> Option<PathBuf> {
        let v1_base = match std::env::var_os("IRONCLAW_BASE_DIR") {
            Some(path) if !path.is_empty() => PathBuf::from(path),
            _ => PathBuf::from(std::env::var_os("HOME")?).join(".ironclaw"),
        };
        Some(v1_base)
    }
}
