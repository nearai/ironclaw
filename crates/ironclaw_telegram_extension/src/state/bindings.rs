use async_trait::async_trait;
use ironclaw_filesystem::{CasExpectation, FilesystemError};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;

use super::FilesystemTelegramHostState;
use super::records::{
    StoredTelegramBinding, StoredTelegramBindingUserIndex, binding_path, binding_user_index_path,
};
use crate::telegram_pairing::{RemovedTelegramBinding, TelegramBindingError};
use ironclaw_channel_host::identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};

impl FilesystemTelegramHostState {
    pub async fn bind_telegram_user(
        &self,
        provider_user_id: &str,
        user_id: &UserId,
        epoch: &str,
    ) -> Result<(), TelegramBindingError> {
        let lock = self.lock_for(format!("telegram-binding:{provider_user_id}"));
        let _held = lock.lock().await;
        let path = binding_path(provider_user_id).map_err(map_fs_binding)?;
        if let Some((existing, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(map_fs_binding)?
            && existing.user_id != user_id.as_str()
        {
            return Err(TelegramBindingError::AlreadyBoundToOtherUser);
        }
        self.write_record(
            &path,
            &StoredTelegramBinding {
                provider_user_id: provider_user_id.to_string(),
                user_id: user_id.as_str().to_string(),
                epoch: epoch.to_string(),
            },
            CasExpectation::Any,
        )
        .await
        .map_err(map_fs_binding)?;

        let index_path = binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let mut index = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(map_fs_binding)?
            .map(|(index, _)| index)
            .unwrap_or_default();
        if !index
            .provider_user_ids
            .iter()
            .any(|existing| existing == provider_user_id)
        {
            index.provider_user_ids.push(provider_user_id.to_string());
        }
        self.write_record(&index_path, &index, CasExpectation::Any)
            .await
            .map_err(map_fs_binding)?;
        Ok(())
    }

    pub async fn unbind_telegram_users_for_user(
        &self,
        user_id: &UserId,
        installation: Option<&AdapterInstallationId>,
    ) -> Result<Vec<RemovedTelegramBinding>, TelegramBindingError> {
        let index_path = binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let Some((index, _)) = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(map_fs_binding)?
        else {
            return Ok(Vec::new());
        };
        let mut removed = Vec::new();
        let mut retained = Vec::new();
        for provider_user_id in index.provider_user_ids {
            let in_scope = installation.is_none_or(|installation| {
                crate::telegram_actor_identity::provider_user_id_in_installation(
                    &provider_user_id,
                    installation,
                )
            });
            if !in_scope {
                retained.push(provider_user_id);
                continue;
            }
            let path = binding_path(&provider_user_id).map_err(map_fs_binding)?;
            let epoch = self
                .read_record::<StoredTelegramBinding>(&path)
                .await
                .map_err(map_fs_binding)?
                .map(|(record, _)| record.epoch);
            match self.delete_record(&path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {
                    removed.push(RemovedTelegramBinding {
                        provider_user_id,
                        epoch,
                    });
                }
                Err(error) => return Err(map_fs_binding(error)),
            }
        }
        self.write_record(
            &index_path,
            &StoredTelegramBindingUserIndex {
                provider_user_ids: retained,
            },
            CasExpectation::Any,
        )
        .await
        .map_err(map_fs_binding)?;
        Ok(removed)
    }

    pub async fn bound_user_for(
        &self,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, TelegramBindingError> {
        let path = binding_path(provider_user_id).map_err(map_fs_binding)?;
        let Some((record, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(map_fs_binding)?
        else {
            return Ok(None);
        };
        UserId::new(record.user_id).map(Some).map_err(|error| {
            TelegramBindingError::StoreUnavailable {
                reason: format!("stored telegram binding user id invalid: {error}"),
            }
        })
    }
}

#[async_trait]
impl RebornUserIdentityLookup for FilesystemTelegramHostState {
    async fn resolve_user_identity(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<UserId>, RebornUserIdentityLookupError> {
        Ok(self
            .resolve_user_identity_with_binding_epoch(provider, provider_user_id)
            .await?
            .map(|(user_id, _)| user_id))
    }

    async fn resolve_user_identity_with_binding_epoch(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<
        Option<(
            UserId,
            Option<ironclaw_conversations::ExternalActorBindingEpoch>,
        )>,
        RebornUserIdentityLookupError,
    > {
        if provider != crate::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER {
            return Ok(None);
        }
        let path = binding_path(provider_user_id)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        let Some((record, _)) = self
            .read_record::<StoredTelegramBinding>(&path)
            .await
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
        else {
            return Ok(None);
        };
        let user_id = UserId::new(record.user_id)
            .map_err(|error| RebornUserIdentityLookupError::InvalidUserId(error.to_string()))?;
        let epoch = ironclaw_conversations::ExternalActorBindingEpoch::new(record.epoch)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        Ok(Some((user_id, Some(epoch))))
    }

    async fn user_has_provider_binding(
        &self,
        provider: &str,
        user_id: &UserId,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        self.user_has_provider_binding_with_provider_user_id_prefix(provider, user_id, None)
            .await
    }

    async fn user_has_provider_binding_with_provider_user_id_prefix(
        &self,
        provider: &str,
        user_id: &UserId,
        provider_user_id_prefix: Option<&str>,
    ) -> Result<bool, RebornUserIdentityLookupError> {
        if provider != crate::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER {
            return Ok(false);
        }
        let index_path = binding_user_index_path(user_id)
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
        let Some((index, _)) = self
            .read_record::<StoredTelegramBindingUserIndex>(&index_path)
            .await
            .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
        else {
            return Ok(false);
        };
        Ok(index.provider_user_ids.iter().any(|candidate| {
            provider_user_id_prefix
                .map(|prefix| provider_user_id_matches_installation_prefix(candidate, prefix))
                .unwrap_or(true)
        }))
    }
}

fn provider_user_id_matches_installation_prefix(candidate: &str, prefix: &str) -> bool {
    let installation = prefix.strip_suffix(':').unwrap_or(prefix);
    if installation.is_empty() {
        return true;
    }
    crate::telegram_actor_identity::installation_segment_matches(candidate, installation)
}

fn map_fs_binding(error: FilesystemError) -> TelegramBindingError {
    TelegramBindingError::StoreUnavailable {
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_channel_host::identity::RebornUserIdentityLookup;
    use ironclaw_host_api::UserId;
    use ironclaw_product_adapters::AdapterInstallationId;

    use crate::test_support::telegram_state;

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("user")
    }

    fn installation(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("installation")
    }

    #[tokio::test]
    async fn state_binding_scope_is_exact_never_prefix_overlap() {
        let state = telegram_state();
        let ben = user("ben");
        state
            .bind_telegram_user("tg-bot-10:555", &ben, "EPOCH111")
            .await
            .expect("bind");

        for overlapping_prefix in ["tg-bot-1:", "tg-bot-1"] {
            assert!(
                !state
                    .user_has_provider_binding_with_provider_user_id_prefix(
                        crate::telegram_actor_identity::TELEGRAM_IDENTITY_PROVIDER,
                        &ben,
                        Some(overlapping_prefix),
                    )
                    .await
                    .expect("lookup")
            );
        }
        let removed = state
            .unbind_telegram_users_for_user(&ben, Some(&installation("tg-bot-1")))
            .await
            .expect("scoped unbind");
        assert!(removed.is_empty());
        let removed = state
            .unbind_telegram_users_for_user(&ben, None)
            .await
            .expect("unscoped unbind");
        assert_eq!(removed[0].provider_user_id, "tg-bot-10:555");
        assert_eq!(removed[0].epoch.as_deref(), Some("EPOCH111"));
    }
}
