use async_trait::async_trait;
use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, FilesystemError, cas_update,
};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;

use super::FilesystemTelegramHostState;
use super::records::{
    StoredTelegramBinding, StoredTelegramBindingUserIndex, binding_path, binding_user_index_path,
};
use crate::pairing::{RemovedTelegramBinding, TelegramBindingError};
use ironclaw_channel_host::identity::{RebornUserIdentityLookup, RebornUserIdentityLookupError};

impl FilesystemTelegramHostState {
    pub async fn bind_telegram_user(
        &self,
        provider_user_id: &str,
        user_id: &UserId,
        epoch: &str,
    ) -> Result<(), TelegramBindingError> {
        let path = binding_path(provider_user_id).map_err(map_fs_binding)?;
        let next = StoredTelegramBinding {
            provider_user_id: provider_user_id.to_string(),
            user_id: user_id.as_str().to_string(),
            epoch: epoch.to_string(),
            active: true,
        };
        let next_for_apply = next.clone();
        let previous = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_binding,
            encode_binding,
            move |current: Option<StoredTelegramBinding>| {
                let next = next_for_apply.clone();
                async move {
                    if current
                        .as_ref()
                        .is_some_and(|existing| existing.active && existing.user_id != next.user_id)
                    {
                        return Err(TelegramBindingError::AlreadyBoundToOtherUser);
                    }
                    Ok(CasApply::new(next, current))
                }
            },
        )
        .await
        .map_err(map_cas_binding)?;

        let index_path = binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let provider_user_id_for_index = provider_user_id.to_string();
        let index_result = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &index_path,
            decode_binding_index,
            encode_binding_index,
            move |current: Option<StoredTelegramBindingUserIndex>| {
                let provider_user_id = provider_user_id_for_index.clone();
                async move {
                    let mut index = current.unwrap_or_default();
                    if !index
                        .provider_user_ids
                        .iter()
                        .any(|existing| existing == &provider_user_id)
                    {
                        index.provider_user_ids.push(provider_user_id);
                        index.provider_user_ids.sort();
                    }
                    Ok(CasApply::new(index, ()))
                }
            },
        )
        .await
        .map_err(map_cas_binding);
        if let Err(index_error) = index_result {
            // Compensate only while the binding record is still exactly the
            // value this operation published. A concurrent re-pair wins.
            let prior = previous.clone();
            let next_for_compensation = next.clone();
            let compensation = cas_update(
                self.filesystem.as_ref(),
                &self.scope,
                &path,
                decode_binding,
                encode_binding,
                move |current: Option<StoredTelegramBinding>| {
                    let prior = prior.clone();
                    let next = next_for_compensation.clone();
                    async move {
                        let Some(current) = current else {
                            return Ok(CasApply::no_op(next, ()));
                        };
                        if current != next {
                            return Ok(CasApply::no_op(current, ()));
                        }
                        let restored = prior.unwrap_or(StoredTelegramBinding {
                            active: false,
                            ..next
                        });
                        Ok(CasApply::new(restored, ()))
                    }
                },
            )
            .await;
            if let Err(compensation_error) = compensation {
                tracing::debug!(
                    error = %compensation_error,
                    "telegram binding compensation failed after index update failure"
                );
            }
            return Err(index_error);
        }
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
        for provider_user_id in index.provider_user_ids {
            let in_scope = installation.is_none_or(|installation| {
                crate::telegram_actor_identity::provider_user_id_in_installation(
                    &provider_user_id,
                    installation,
                )
            });
            if !in_scope {
                continue;
            }
            let path = binding_path(&provider_user_id).map_err(map_fs_binding)?;
            let provider_for_apply = provider_user_id.clone();
            let epoch = cas_update(
                self.filesystem.as_ref(),
                &self.scope,
                &path,
                decode_binding,
                encode_binding,
                move |current: Option<StoredTelegramBinding>| {
                    let provider_user_id = provider_for_apply.clone();
                    async move {
                        let Some(mut binding) = current else {
                            let absent = StoredTelegramBinding {
                                provider_user_id,
                                user_id: String::new(),
                                epoch: String::new(),
                                active: false,
                            };
                            return Ok(CasApply::no_op(absent, None));
                        };
                        let epoch = (!binding.user_id.is_empty()).then(|| binding.epoch.clone());
                        binding.active = false;
                        Ok(CasApply::new(binding, epoch))
                    }
                },
            )
            .await
            .map_err(map_cas_binding)?;
            if let Some(epoch) = epoch {
                removed.push(RemovedTelegramBinding {
                    provider_user_id: provider_user_id.clone(),
                    epoch: Some(epoch),
                });
            }
        }
        Ok(removed)
    }

    /// Remove user-index cleanup metadata only after every external actor and
    /// DM-target cleanup step has succeeded. Until then a retry (including
    /// after restart) can reconstruct the work from the inactive binding.
    pub async fn finalize_unbound_telegram_users_for_user(
        &self,
        user_id: &UserId,
        provider_user_ids: &[String],
    ) -> Result<(), TelegramBindingError> {
        let index_path = binding_user_index_path(user_id).map_err(map_fs_binding)?;
        let removal_ids_for_apply = provider_user_ids.to_vec();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &index_path,
            decode_binding_index,
            encode_binding_index,
            move |current: Option<StoredTelegramBindingUserIndex>| {
                let removal_ids = removal_ids_for_apply.clone();
                async move {
                    let mut index = current.unwrap_or_default();
                    index
                        .provider_user_ids
                        .retain(|candidate| !removal_ids.contains(candidate));
                    Ok(CasApply::new(index, ()))
                }
            },
        )
        .await
        .map_err(map_cas_binding)?;
        Ok(())
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
        if !record.active {
            return Ok(None);
        }
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
        if !record.active {
            return Ok(None);
        }
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
        for candidate in index.provider_user_ids.iter().filter(|candidate| {
            provider_user_id_prefix
                .map(|prefix| provider_user_id_matches_installation_prefix(candidate, prefix))
                .unwrap_or(true)
        }) {
            let path = binding_path(candidate)
                .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?;
            let binding = self
                .read_record::<StoredTelegramBinding>(&path)
                .await
                .map_err(|error| RebornUserIdentityLookupError::Backend(error.to_string()))?
                .map(|(record, _)| record);
            if binding
                .as_ref()
                .is_some_and(|record| record.active && record.user_id == user_id.as_str())
            {
                return Ok(true);
            }
        }
        Ok(false)
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

fn decode_binding(bytes: &[u8]) -> Result<StoredTelegramBinding, TelegramBindingError> {
    serde_json::from_slice(bytes).map_err(|error| TelegramBindingError::StoreUnavailable {
        reason: format!("stored telegram binding is invalid JSON: {error}"),
    })
}

fn encode_binding(value: &StoredTelegramBinding) -> Result<Entry, TelegramBindingError> {
    encode_json(value, "telegram binding")
}

fn decode_binding_index(
    bytes: &[u8],
) -> Result<StoredTelegramBindingUserIndex, TelegramBindingError> {
    serde_json::from_slice(bytes).map_err(|error| TelegramBindingError::StoreUnavailable {
        reason: format!("stored telegram binding index is invalid JSON: {error}"),
    })
}

fn encode_binding_index(
    value: &StoredTelegramBindingUserIndex,
) -> Result<Entry, TelegramBindingError> {
    encode_json(value, "telegram binding index")
}

fn encode_json<T: serde::Serialize>(value: &T, label: &str) -> Result<Entry, TelegramBindingError> {
    let body =
        serde_json::to_vec(value).map_err(|error| TelegramBindingError::StoreUnavailable {
            reason: format!("{label} could not be serialized: {error}"),
        })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn map_cas_binding(error: CasUpdateError<TelegramBindingError>) -> TelegramBindingError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => TelegramBindingError::StoreUnavailable {
            reason: error.to_string(),
        },
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

    #[tokio::test]
    async fn concurrent_provider_bindings_preserve_both_user_index_entries() {
        let state = telegram_state();
        let ben = user("ben");
        let (first, second) = tokio::join!(
            state.bind_telegram_user("tg-bot-1:100", &ben, "EPOCH100"),
            state.bind_telegram_user("tg-bot-2:200", &ben, "EPOCH200"),
        );
        first.expect("first concurrent binding");
        second.expect("second concurrent binding");

        let mut removed = state
            .unbind_telegram_users_for_user(&ben, None)
            .await
            .expect("both indexed bindings remain discoverable");
        removed.sort_by(|left, right| left.provider_user_id.cmp(&right.provider_user_id));
        assert_eq!(
            removed
                .iter()
                .map(|binding| binding.provider_user_id.as_str())
                .collect::<Vec<_>>(),
            vec!["tg-bot-1:100", "tg-bot-2:200"]
        );
    }
}
