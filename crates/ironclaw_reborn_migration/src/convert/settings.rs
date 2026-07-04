//! Settings converter (v1 `settings` key/value → Reborn config).
//!
//! Reborn configuration is a *typed* `config.toml` schema plus `providers.json`
//! and the `LlmKeyStore` — there is **no generic key/value settings store**.
//! A standalone migration cannot safely fold arbitrary v1 keys into that typed
//! schema (and the config file lives at the Reborn home, not in the state
//! store), so every v1 setting is enumerated and recorded as a loss naming the
//! key, so an operator can re-apply the ones that matter via `ironclaw-reborn`
//! config. Nothing is silently dropped.

use crate::error::MigrationError;
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    _tgt: &mut RebornTarget,
    _options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let users = src.distinct_users().await?;
    for user_id in &users {
        let settings =
            src.db
                .get_all_settings(user_id)
                .await
                .map_err(|e| MigrationError::ReadSource {
                    domain: "settings".into(),
                    reason: e.to_string(),
                })?;
        for key in settings.keys() {
            report.record_loss(
                Domain::Setting,
                format!("{user_id}:{key}"),
                key.clone(),
                LossReason::NoTargetConcept,
                "Reborn config is a typed config.toml / providers.json / LlmKeyStore; \
                 there is no generic key/value settings store to migrate into. \
                 Re-apply via `ironclaw-reborn` config if needed."
                    .to_string(),
            );
        }
    }
    Ok(())
}
