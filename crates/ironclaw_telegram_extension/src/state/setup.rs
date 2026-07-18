use ironclaw_filesystem::{CasApply, CasUpdateError, ContentType, Entry, cas_update};
use serde::{Deserialize, Serialize};

use super::FilesystemTelegramHostState;
use super::records::setup_path;
use crate::setup::{TelegramInstallationSetup, TelegramSetupError};

/// The active record remains wire-compatible with the original plain setup
/// JSON. Cleanup states are tagged alternatives so an interrupted clear keeps
/// the handles needed to retry provider/secret cleanup after restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredTelegramSetup {
    Active(TelegramInstallationSetup),
    Lifecycle(StoredTelegramSetupLifecycle),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "lifecycle", rename_all = "snake_case")]
enum StoredTelegramSetupLifecycle {
    Clearing {
        setup: TelegramInstallationSetup,
    },
    RollingBack {
        saved: TelegramInstallationSetup,
        previous: Option<TelegramInstallationSetup>,
        #[serde(default)]
        provider_compensated: bool,
    },
    Cleared {
        cleared_revision: u64,
    },
}

impl FilesystemTelegramHostState {
    pub async fn get_telegram_installation_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        Ok(match self.read_setup_record().await? {
            Some(StoredTelegramSetup::Active(setup)) => Some(setup),
            Some(StoredTelegramSetup::Lifecycle(_)) | None => None,
        })
    }

    /// Return the setup whose clear saga is active. `Clearing` remains
    /// recoverable but is intentionally absent from normal ingress/egress
    /// reads so a half-cleaned installation fails closed.
    pub async fn telegram_installation_setup_for_cleanup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        Ok(match self.read_setup_record().await? {
            Some(StoredTelegramSetup::Active(setup))
            | Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Clearing {
                setup,
            })) => Some(setup),
            Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Cleared {
                ..
            }))
            | Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::RollingBack {
                ..
            }))
            | None => None,
        })
    }

    pub async fn next_telegram_installation_setup_revision(
        &self,
    ) -> Result<u64, TelegramSetupError> {
        Ok(
            setup_record_revision(self.read_setup_record().await?.as_ref())
                .unwrap_or(0)
                .saturating_add(1),
        )
    }

    /// Seed/replace helper retained for fixtures and narrow host setup. A stale
    /// revision never overwrites a newer active or tombstoned revision.
    pub async fn put_telegram_installation_setup(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<(), TelegramSetupError> {
        let next = setup.clone();
        let wrote = self
            .cas_setup(move |current| {
                let next = next.clone();
                async move {
                    let current_revision = setup_record_revision(current.as_ref());
                    let replacement = StoredTelegramSetup::Active(next.clone());
                    if current_revision.is_some_and(|revision| {
                        revision > next.revision
                            || (revision == next.revision && current.as_ref() != Some(&replacement))
                    }) {
                        return Ok(noop_setup(current, false));
                    }
                    if current.as_ref() == Some(&replacement) {
                        return Ok(noop_setup(current, true));
                    }
                    Ok(CasApply::new(replacement, true))
                }
            })
            .await?;
        if wrote {
            Ok(())
        } else {
            Err(TelegramSetupError::ConcurrentUpdate)
        }
    }

    /// Publish `next` only while the exact setup snapshot captured by the
    /// caller is still authoritative. This is the cross-process commit point
    /// for saves and activation rollback.
    pub async fn replace_telegram_installation_setup_if_current(
        &self,
        expected: Option<&TelegramInstallationSetup>,
        next: Option<&TelegramInstallationSetup>,
    ) -> Result<bool, TelegramSetupError> {
        let expected = expected.cloned();
        let next = next.cloned();
        self.cas_setup(move |current| {
            let expected = expected.clone();
            let next = next.clone();
            async move {
                let matches = match (&current, &expected) {
                    (None, None) => true,
                    (
                        Some(StoredTelegramSetup::Lifecycle(
                            StoredTelegramSetupLifecycle::Cleared {
                                cleared_revision, ..
                            },
                        )),
                        None,
                    ) => next
                        .as_ref()
                        .is_some_and(|setup| setup.revision > *cleared_revision),
                    (Some(StoredTelegramSetup::Active(current)), Some(expected)) => {
                        current == expected
                    }
                    _ => false,
                };
                if !matches {
                    return Ok(noop_setup(current, false));
                }
                let replacement = match next {
                    Some(setup) => StoredTelegramSetup::Active(setup),
                    None => cleared_setup(expected.as_ref().map_or(0, |setup| setup.revision)),
                };
                Ok(CasApply::new(replacement, true))
            }
        })
        .await
    }

    /// Persist the cleanup intent before any provider or secret side effect.
    pub async fn begin_telegram_installation_setup_cleanup(
        &self,
        expected: &TelegramInstallationSetup,
    ) -> Result<bool, TelegramSetupError> {
        let expected = expected.clone();
        self.cas_setup(move |current| {
            let expected = expected.clone();
            async move {
                match current {
                    Some(StoredTelegramSetup::Active(current)) if current == expected => {
                        Ok(CasApply::new(
                            StoredTelegramSetup::Lifecycle(
                                StoredTelegramSetupLifecycle::Clearing { setup: current },
                            ),
                            true,
                        ))
                    }
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::Clearing { setup },
                    )) if setup == expected => Ok(CasApply::no_op(
                        StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Clearing {
                            setup,
                        }),
                        true,
                    )),
                    other => Ok(noop_setup(other, false)),
                }
            }
        })
        .await
    }

    pub async fn finish_telegram_installation_setup_cleanup(
        &self,
        expected: &TelegramInstallationSetup,
    ) -> Result<bool, TelegramSetupError> {
        let expected = expected.clone();
        self.cas_setup(move |current| {
            let expected = expected.clone();
            async move {
                match current {
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::Clearing { setup },
                    )) if setup == expected => {
                        Ok(CasApply::new(cleared_setup(expected.revision), true))
                    }
                    other => Ok(noop_setup(other, false)),
                }
            }
        })
        .await
    }

    pub async fn begin_telegram_installation_setup_rollback(
        &self,
        saved: &TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<bool, TelegramSetupError> {
        let saved = saved.clone();
        let previous = previous.cloned();
        self.cas_setup(move |current| {
            let saved = saved.clone();
            let previous = previous.clone();
            async move {
                match current {
                    Some(StoredTelegramSetup::Active(current)) if current == saved => {
                        Ok(CasApply::new(
                            StoredTelegramSetup::Lifecycle(
                                StoredTelegramSetupLifecycle::RollingBack {
                                    saved,
                                    previous,
                                    provider_compensated: false,
                                },
                            ),
                            true,
                        ))
                    }
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::RollingBack {
                            saved: current_saved,
                            previous: current_previous,
                            provider_compensated,
                        },
                    )) if current_saved == saved && current_previous == previous => {
                        Ok(CasApply::no_op(
                            StoredTelegramSetup::Lifecycle(
                                StoredTelegramSetupLifecycle::RollingBack {
                                    saved: current_saved,
                                    previous: current_previous,
                                    provider_compensated,
                                },
                            ),
                            true,
                        ))
                    }
                    other => Ok(noop_setup(other, false)),
                }
            }
        })
        .await
    }

    pub async fn telegram_installation_setup_rollback_intent(
        &self,
    ) -> Result<
        Option<(
            TelegramInstallationSetup,
            Option<TelegramInstallationSetup>,
            bool,
        )>,
        TelegramSetupError,
    > {
        Ok(match self.read_setup_record().await? {
            Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::RollingBack {
                saved,
                previous,
                provider_compensated,
            })) => Some((saved, previous, provider_compensated)),
            _ => None,
        })
    }

    pub async fn mark_telegram_installation_setup_rollback_provider_compensated(
        &self,
        saved: &TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<bool, TelegramSetupError> {
        let saved = saved.clone();
        let previous = previous.cloned();
        self.cas_setup(move |current| {
            let saved = saved.clone();
            let previous = previous.clone();
            async move {
                match current {
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::RollingBack {
                            saved: current_saved,
                            previous: current_previous,
                            provider_compensated: false,
                        },
                    )) if current_saved == saved && current_previous == previous => {
                        Ok(CasApply::new(
                            StoredTelegramSetup::Lifecycle(
                                StoredTelegramSetupLifecycle::RollingBack {
                                    saved: current_saved,
                                    previous: current_previous,
                                    provider_compensated: true,
                                },
                            ),
                            true,
                        ))
                    }
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::RollingBack {
                            saved: current_saved,
                            previous: current_previous,
                            provider_compensated: true,
                        },
                    )) if current_saved == saved && current_previous == previous => {
                        Ok(CasApply::no_op(
                            StoredTelegramSetup::Lifecycle(
                                StoredTelegramSetupLifecycle::RollingBack {
                                    saved: current_saved,
                                    previous: current_previous,
                                    provider_compensated: true,
                                },
                            ),
                            true,
                        ))
                    }
                    other => Ok(noop_setup(other, false)),
                }
            }
        })
        .await
    }

    pub async fn finish_telegram_installation_setup_rollback(
        &self,
        saved: &TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<bool, TelegramSetupError> {
        let saved = saved.clone();
        let previous = previous.cloned();
        self.cas_setup(move |current| {
            let saved = saved.clone();
            let previous = previous.clone();
            async move {
                match current {
                    Some(StoredTelegramSetup::Lifecycle(
                        StoredTelegramSetupLifecycle::RollingBack {
                            saved: current_saved,
                            previous: current_previous,
                            provider_compensated: true,
                        },
                    )) if current_saved == saved && current_previous == previous => {
                        let replacement = previous
                            .map(StoredTelegramSetup::Active)
                            .unwrap_or_else(|| cleared_setup(saved.revision));
                        Ok(CasApply::new(replacement, true))
                    }
                    other => Ok(noop_setup(other, false)),
                }
            }
        })
        .await
    }

    pub async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError> {
        let Some(setup) = self.telegram_installation_setup_for_cleanup().await? else {
            return Ok(());
        };
        if !self
            .begin_telegram_installation_setup_cleanup(&setup)
            .await?
            || !self
                .finish_telegram_installation_setup_cleanup(&setup)
                .await?
        {
            return Err(TelegramSetupError::ConcurrentUpdate);
        }
        Ok(())
    }

    async fn read_setup_record(&self) -> Result<Option<StoredTelegramSetup>, TelegramSetupError> {
        let path = setup_path().map_err(map_fs_setup)?;
        Ok(self
            .read_record::<StoredTelegramSetup>(&path)
            .await
            .map_err(map_fs_setup)?
            .map(|(record, _)| record))
    }

    async fn cas_setup<A, Fut>(&self, apply: A) -> Result<bool, TelegramSetupError>
    where
        A: FnMut(Option<StoredTelegramSetup>) -> Fut,
        Fut: std::future::Future<
                Output = Result<CasApply<StoredTelegramSetup, bool>, TelegramSetupError>,
            >,
    {
        let path = setup_path().map_err(map_fs_setup)?;
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_setup,
            encode_setup,
            apply,
        )
        .await
        .map_err(map_cas_setup)
    }
}

fn setup_record_revision(record: Option<&StoredTelegramSetup>) -> Option<u64> {
    match record {
        Some(StoredTelegramSetup::Active(setup))
        | Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Clearing { setup })) => {
            Some(setup.revision)
        }
        Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::RollingBack {
            saved,
            ..
        })) => Some(saved.revision),
        Some(StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Cleared {
            cleared_revision,
            ..
        })) => Some(*cleared_revision),
        None => None,
    }
}

fn noop_setup(
    current: Option<StoredTelegramSetup>,
    outcome: bool,
) -> CasApply<StoredTelegramSetup, bool> {
    CasApply::no_op(current.unwrap_or_else(|| cleared_setup(0)), outcome)
}

fn cleared_setup(revision: u64) -> StoredTelegramSetup {
    StoredTelegramSetup::Lifecycle(StoredTelegramSetupLifecycle::Cleared {
        cleared_revision: revision,
    })
}

fn decode_setup(bytes: &[u8]) -> Result<StoredTelegramSetup, TelegramSetupError> {
    serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "telegram setup record is invalid JSON");
        TelegramSetupError::StoreUnavailable
    })
}

fn encode_setup(value: &StoredTelegramSetup) -> Result<Entry, TelegramSetupError> {
    let body = serde_json::to_vec(value).map_err(|error| {
        tracing::debug!(%error, "telegram setup record could not be serialized");
        TelegramSetupError::StoreUnavailable
    })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn map_cas_setup(error: CasUpdateError<TelegramSetupError>) -> TelegramSetupError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => {
            tracing::debug!(%error, "telegram setup CAS update failed");
            TelegramSetupError::StoreUnavailable
        }
    }
}

fn map_fs_setup(error: ironclaw_filesystem::FilesystemError) -> TelegramSetupError {
    tracing::debug!(%error, "telegram setup store filesystem error");
    TelegramSetupError::StoreUnavailable
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_host_api::SecretHandle;

    use super::*;
    use crate::test_support::{fault_injected_telegram_state, telegram_state};

    fn setup(revision: u64) -> TelegramInstallationSetup {
        TelegramInstallationSetup {
            bot_id: 42,
            bot_username: "ironclaw_test_bot".to_string(),
            webhook_url: "https://example.test/webhook".to_string(),
            bot_token_handle: SecretHandle::new(format!("telegram_bot_token_test_{revision}"))
                .expect("token handle"),
            webhook_secret_handle: SecretHandle::new(format!(
                "telegram_webhook_secret_test_{revision}"
            ))
            .expect("secret handle"),
            revision,
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn state_setup_round_trips_and_tombstones_the_production_record() {
        let state = telegram_state();
        let expected = setup(3);

        state
            .put_telegram_installation_setup(&expected)
            .await
            .expect("setup persists");
        assert_eq!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads"),
            Some(expected.clone())
        );
        state
            .delete_telegram_installation_setup()
            .await
            .expect("setup tombstones");
        assert!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads")
                .is_none()
        );
    }

    #[tokio::test]
    async fn stale_setup_revision_cannot_overwrite_a_newer_revision() {
        let state = telegram_state();
        state
            .put_telegram_installation_setup(&setup(2))
            .await
            .expect("newer setup persists");

        assert_eq!(
            state
                .put_telegram_installation_setup(&setup(1))
                .await
                .expect_err("stale setup is rejected"),
            TelegramSetupError::ConcurrentUpdate
        );
        assert_eq!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads")
                .map(|setup| setup.revision),
            Some(2)
        );
    }

    #[tokio::test]
    async fn distinct_setup_with_same_revision_cannot_overwrite_winner() {
        let state = telegram_state();
        let winner = setup(2);
        let mut conflicting = setup(2);
        conflicting.bot_username = "different-bot".to_string();
        state
            .put_telegram_installation_setup(&winner)
            .await
            .expect("winner persists");

        assert_eq!(
            state
                .put_telegram_installation_setup(&conflicting)
                .await
                .expect_err("same revision with different content conflicts"),
            TelegramSetupError::ConcurrentUpdate
        );
        assert_eq!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads"),
            Some(winner)
        );
    }

    #[tokio::test]
    async fn rollback_compare_and_swap_cannot_replace_a_newer_save() {
        let state = telegram_state();
        let revision_one = setup(1);
        let revision_two = setup(2);
        state
            .put_telegram_installation_setup(&revision_two)
            .await
            .expect("newer setup persists");

        assert!(
            !state
                .replace_telegram_installation_setup_if_current(
                    Some(&revision_one),
                    Some(&setup(0)),
                )
                .await
                .expect("conditional rollback checks current revision")
        );
        assert_eq!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads"),
            Some(revision_two)
        );
    }

    #[tokio::test]
    async fn state_setup_cleanup_intent_survives_a_followup_read() {
        let state = telegram_state();
        let expected = setup(1);
        state
            .put_telegram_installation_setup(&expected)
            .await
            .expect("setup persists before cleanup");
        assert!(
            state
                .begin_telegram_installation_setup_cleanup(&expected)
                .await
                .expect("cleanup intent persists")
        );

        assert!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("normal lookup")
                .is_none(),
            "ingress fails closed while cleanup is pending"
        );
        assert_eq!(
            state
                .telegram_installation_setup_for_cleanup()
                .await
                .expect("cleanup lookup"),
            Some(expected)
        );
    }

    #[tokio::test]
    async fn state_setup_cleanup_maps_filesystem_failure_to_store_unavailable() {
        let (state, filesystem) = fault_injected_telegram_state();
        let expected = setup(1);
        state
            .put_telegram_installation_setup(&expected)
            .await
            .expect("setup persists before fault");
        filesystem.fail_versioned_writes();

        assert_eq!(
            state
                .begin_telegram_installation_setup_cleanup(&expected)
                .await
                .expect_err("cleanup fault is reported"),
            TelegramSetupError::StoreUnavailable
        );
    }
}
