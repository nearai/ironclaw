use std::path::{Path, PathBuf};

use ironclaw_host_api::UserId;

use crate::RebornBuildError;
use crate::root::default_system_prompt::seed_default_system_prompt;

const DEFAULT_SYSTEM_PROMPT_PATH: &str = "system/prompts/default-system.md";
#[cfg(test)]
pub(crate) use ironclaw_extension_host::bundled_skills::{
    LEGACY_SKILLS_BACKFILL_MARKER, backfill_legacy_user_skills,
};

/// Initializes standalone host content after storage roots are prepared.
pub(crate) async fn bootstrap_standalone_host(
    storage_root: &Path,
    owner_user_id: &UserId,
) -> Result<PathBuf, RebornBuildError> {
    let backfill_root = storage_root.to_path_buf();
    let backfill_owner_user_id = owner_user_id.clone();
    tokio::task::spawn_blocking(move || {
        ironclaw_extension_host::bundled_skills::backfill_legacy_user_skills(
            &backfill_root,
            &backfill_owner_user_id,
        )
    })
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("legacy skill backfill task failed: {error}"),
    })??;

    let default_system_prompt_path = storage_root.join(DEFAULT_SYSTEM_PROMPT_PATH);
    seed_default_system_prompt(storage_root, &default_system_prompt_path).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    ironclaw_extension_host::bundled_skills::ensure_bundled_reborn_skills_installed(storage_root)
        .await?;

    Ok(default_system_prompt_path)
}
