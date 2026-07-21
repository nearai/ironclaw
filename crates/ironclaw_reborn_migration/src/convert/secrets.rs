//! Secrets converter (v1 `secrets` → Reborn `FilesystemSecretStore`).
//!
//! v1 and Reborn both use AES-256-GCM but bind ciphertext to *different* schemes,
//! so migration must **decrypt** each v1 secret and **re-encrypt** through
//! Reborn's `SecretStore::put`. Decryption uses the v1 secrets store constructed
//! with the supplied master key (`--secret-master-key`); the same key builds the
//! Reborn crypto in `RebornTarget::secret_store`. Without a master key, secrets
//! are skipped with a recorded loss. A secret whose decrypt fails (e.g. expired,
//! or wrong key) is recorded per-secret and skipped rather than aborting the run.

use std::sync::Arc;

use ironclaw_host_api::{InvocationId, ResourceScope, SecretHandle};
use ironclaw_secrets::SecretMaterial;

use crate::error::MigrationError;
use crate::legacy_snapshot::secrets::{LegacySecretsSource, SecretsCrypto, create_secrets_store};
use crate::options::MigrationOptions;
use crate::report::{Domain, LossReason, MigrationReport};
use crate::source::V1Source;
use crate::target::RebornTarget;

pub(crate) async fn run(
    src: &V1Source,
    tgt: &mut RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
) -> Result<(), MigrationError> {
    let Some(master_key) = options.secret_master_key.as_ref() else {
        report.record_loss(
            Domain::Secret,
            "secrets",
            "*",
            LossReason::NoTargetField,
            "no --secret-master-key supplied; v1 secrets cannot be decrypted and were skipped"
                .to_string(),
        );
        return Ok(());
    };
    let Some(secret_store) = tgt.secret_store.clone() else {
        report.record_loss(
            Domain::Secret,
            "secrets",
            "*",
            LossReason::NoTargetField,
            "target secret store unavailable".to_string(),
        );
        return Ok(());
    };

    // v1 store, built from the same master key, used only to list + decrypt.
    let crypto = Arc::new(
        SecretsCrypto::new(master_key.clone())
            .map_err(|e| MigrationError::OpenSource(format!("v1 secrets master key: {e}")))?,
    );
    let Some(v1_store) = create_secrets_store(crypto, &src.handles) else {
        report.record_loss(
            Domain::Secret,
            "secrets",
            "*",
            LossReason::Unparseable,
            "could not construct a v1 secrets store for the source backend".to_string(),
        );
        return Ok(());
    };

    // v1 `list`/`get_decrypted` are per-user; enumerate users from the raw table.
    let users = src.distinct_user_ids_in("secrets", "user_id").await?;
    for user_id in users {
        let refs = v1_store
            .list(&user_id)
            .await
            .map_err(|e| MigrationError::ReadSource {
                domain: "secrets".into(),
                reason: e.to_string(),
            })?;
        for secret_ref in refs {
            migrate_one(
                &v1_store,
                secret_store.as_ref(),
                tgt,
                options,
                report,
                &user_id,
                &secret_ref.name,
            )
            .await?;
        }
    }
    Ok(())
}

// arch-exempt: too_many_args, secret converter threads both v1 + Reborn store
// handles plus scope/options and the user/name key; the aggregation would be a
// per-secret context struct, plan v1-migration
#[allow(clippy::too_many_arguments)]
async fn migrate_one(
    v1_store: &LegacySecretsSource,
    secret_store: &dyn ironclaw_secrets::SecretStore,
    tgt: &RebornTarget,
    options: &MigrationOptions,
    report: &mut MigrationReport,
    user_id: &str,
    name: &str,
) -> Result<(), MigrationError> {
    // Decrypt the plaintext. A failure here (expired row, key mismatch) is a
    // per-secret loss, not a run-abort.
    let decrypted = match v1_store.get_decrypted(user_id, name).await {
        Ok(value) => value,
        Err(e) => {
            report.record_loss(
                Domain::Secret,
                format!("{user_id}:{name}"),
                "decrypt",
                LossReason::Unparseable,
                format!("could not decrypt v1 secret (skipped): {e}"),
            );
            return Ok(());
        }
    };
    // Preserve expiry when the record carries one. `get_decrypted` above does
    // not surface the record metadata, so this second read fetches `expires_at`;
    // a read failure here is not silently dropped — the secret still migrates,
    // but the lost expiry is recorded.
    let expires_at = match v1_store.get(user_id, name).await {
        Ok(secret) => secret.expires_at,
        Err(e) => {
            report.record_loss(
                Domain::Secret,
                format!("{user_id}:{name}"),
                "expires_at",
                LossReason::Degraded,
                format!(
                    "could not re-read v1 secret metadata for expiry (migrated without it): {e}"
                ),
            );
            None
        }
    };

    let handle = match SecretHandle::new(name) {
        Ok(handle) => handle,
        Err(e) => {
            report.record_loss(
                Domain::Secret,
                format!("{user_id}:{name}"),
                "handle",
                LossReason::Unparseable,
                format!("v1 secret name is not a valid Reborn secret handle: {e}"),
            );
            return Ok(());
        }
    };
    // A malformed source user id is a per-item loss, not a run abort: skip this
    // secret and keep migrating the rest.
    let Some(user) = report.valid_user_id(
        Domain::Secret,
        format!("{user_id}:{name}"),
        "user_id",
        user_id,
    ) else {
        return Ok(());
    };

    if options.dry_run {
        report.stats.secrets += 1;
        return Ok(());
    }

    let scope = ResourceScope {
        tenant_id: tgt.tenant_id.clone(),
        user_id: user,
        agent_id: Some(tgt.agent_id.clone()),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let material = SecretMaterial::from(decrypted.expose().to_string());
    secret_store
        .put(scope, handle, material, expires_at)
        .await
        .map_err(|e| MigrationError::WriteTarget {
            domain: format!("secret {user_id}:{name}"),
            reason: e.to_string(),
        })?;
    report.stats.secrets += 1;
    Ok(())
}
