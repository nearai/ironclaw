use chrono::Utc;
use ironclaw_filesystem::{
    CasApply, CasExpectation, CasUpdateError, ContentType, Entry, FilesystemError, cas_update,
};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;

use super::FilesystemTelegramHostState;
use super::records::{
    StoredPairingCompletion, StoredPairingUserPointer, pairing_code_path, pairing_completion_path,
    pairing_user_path,
};
use crate::pairing::{PairingCode, TelegramPairingError, TelegramPairingRecord};

impl FilesystemTelegramHostState {
    pub async fn persist_pairing_completion(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
        chat_id: i64,
    ) -> Result<(), TelegramPairingError> {
        let path = pairing_completion_path(installation_id, user_id).map_err(map_fs_pairing)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_pairing_completion,
            encode_pairing_completion,
            move |_current: Option<StoredPairingCompletion>| {
                let completion = StoredPairingCompletion {
                    installation_id: installation_id.clone(),
                    user_id: user_id.clone(),
                    chat_id,
                    completed: false,
                };
                async move { Ok(CasApply::new(completion, ())) }
            },
        )
        .await
        .map_err(map_cas_pairing)
    }

    pub async fn pending_pairing_completion_chat(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<Option<i64>, TelegramPairingError> {
        let path = pairing_completion_path(installation_id, user_id).map_err(map_fs_pairing)?;
        Ok(self
            .read_record::<StoredPairingCompletion>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(record, _)| record)
            .filter(|record| {
                !record.completed
                    && &record.installation_id == installation_id
                    && &record.user_id == user_id
            })
            .map(|record| record.chat_id))
    }

    pub async fn finish_pairing_completion(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
        chat_id: i64,
    ) -> Result<(), TelegramPairingError> {
        let path = pairing_completion_path(installation_id, user_id).map_err(map_fs_pairing)?;
        let installation_id = installation_id.clone();
        let user_id = user_id.clone();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &path,
            decode_pairing_completion,
            encode_pairing_completion,
            move |current: Option<StoredPairingCompletion>| {
                let installation_id = installation_id.clone();
                let user_id = user_id.clone();
                async move {
                    let Some(mut completion) = current else {
                        let missing = StoredPairingCompletion {
                            installation_id,
                            user_id,
                            chat_id,
                            completed: true,
                        };
                        return Ok(CasApply::no_op(missing, ()));
                    };
                    if completion.installation_id != installation_id
                        || completion.user_id != user_id
                        || completion.chat_id != chat_id
                    {
                        return Err(TelegramPairingError::StoreUnavailable {
                            reason: "pairing completion identity changed concurrently".to_string(),
                        });
                    }
                    completion.completed = true;
                    Ok(CasApply::new(completion, ()))
                }
            },
        )
        .await
        .map_err(map_cas_pairing)
    }

    pub async fn upsert_pending_pairing(
        &self,
        record: TelegramPairingRecord,
    ) -> Result<(), TelegramPairingError> {
        let user_path = pairing_user_path(&record.user_id).map_err(map_fs_pairing)?;
        let previous_pointer = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(pointer, _)| pointer);
        let code_path = pairing_code_path(&record.code).map_err(map_fs_pairing)?;
        let record_for_write = record.clone();
        cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &code_path,
            decode_pairing_record,
            encode_pairing_record,
            move |current: Option<TelegramPairingRecord>| {
                let record = record_for_write.clone();
                async move {
                    if current.is_some() {
                        return Err(TelegramPairingError::ConcurrentUpdate);
                    }
                    Ok(CasApply::new(record, ()))
                }
            },
        )
        .await
        .map_err(map_cas_pairing)?;

        let expected_pointer = previous_pointer.clone();
        let next_pointer = StoredPairingUserPointer {
            code: record.code.clone(),
            active: true,
        };
        let next_pointer_for_apply = next_pointer.clone();
        let published = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &user_path,
            decode_pairing_pointer,
            encode_pairing_pointer,
            move |current: Option<StoredPairingUserPointer>| {
                let expected = expected_pointer.clone();
                let next = next_pointer_for_apply.clone();
                async move {
                    if current != expected {
                        return Ok(CasApply::no_op(current.unwrap_or(next), false));
                    }
                    Ok(CasApply::new(next, true))
                }
            },
        )
        .await
        .map_err(map_cas_pairing)?;
        if !published {
            self.best_effort_delete_pairing_code(&record.code).await;
            return Err(TelegramPairingError::ConcurrentUpdate);
        }
        if let Some(previous) = previous_pointer
            && previous.code != record.code
        {
            self.best_effort_delete_pairing_code(&previous.code).await;
        }
        Ok(())
    }

    pub async fn pairing_for_code(
        &self,
        code: &PairingCode,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let path = pairing_code_path(code).map_err(map_fs_pairing)?;
        let Some(record) = self
            .read_record::<TelegramPairingRecord>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(record, _)| record)
        else {
            return Ok(None);
        };
        let user_path = pairing_user_path(&record.user_id).map_err(map_fs_pairing)?;
        let authoritative = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
            .is_some_and(|(pointer, _)| pointer.active && pointer.code == *code);
        Ok(authoritative.then_some(record))
    }

    pub async fn live_pairing_for_user(
        &self,
        user_id: &UserId,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let user_path = pairing_user_path(user_id).map_err(map_fs_pairing)?;
        let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(None);
        };
        if !pointer.active {
            return Ok(None);
        }
        Ok(self
            .pairing_for_code(&pointer.code)
            .await?
            .filter(|record| record.is_live(Utc::now())))
    }

    pub async fn claim_pairing(
        &self,
        code: &PairingCode,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let path = pairing_code_path(code).map_err(map_fs_pairing)?;
        let Some((mut record, version)) = self
            .read_record::<TelegramPairingRecord>(&path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(None);
        };
        if !record.is_live(Utc::now()) {
            return Ok(None);
        }
        record.consumed_at = Some(Utc::now());
        let claimed = match self
            .write_record(&path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) => Some(record),
            Err(FilesystemError::VersionMismatch { .. }) => None,
            Err(error) => return Err(map_fs_pairing(error)),
        };
        let Some(claimed) = claimed else {
            return Ok(None);
        };
        let user_path = pairing_user_path(&claimed.user_id).map_err(map_fs_pairing)?;
        let authoritative = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
            .is_some_and(|(pointer, _)| pointer.active && pointer.code == *code);
        Ok(authoritative.then_some(claimed))
    }

    pub async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError> {
        let user_path = pairing_user_path(user_id).map_err(map_fs_pairing)?;
        let Some((fallback, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(());
        };
        let invalidated = cas_update(
            self.filesystem.as_ref(),
            &self.scope,
            &user_path,
            decode_pairing_pointer,
            encode_pairing_pointer,
            move |current: Option<StoredPairingUserPointer>| {
                let fallback = fallback.clone();
                async move {
                    let Some(mut pointer) = current else {
                        return Ok(CasApply::no_op(fallback, None));
                    };
                    let code = pointer.active.then(|| pointer.code.clone());
                    pointer.active = false;
                    Ok(CasApply::new(pointer, code))
                }
            },
        )
        .await
        .map_err(map_cas_pairing)?;
        if let Some(code) = invalidated {
            self.best_effort_delete_pairing_code(&code).await;
        }
        Ok(())
    }

    async fn best_effort_delete_pairing_code(&self, code: &PairingCode) {
        let Ok(path) = pairing_code_path(code) else {
            return;
        };
        let version = match self.read_record::<TelegramPairingRecord>(&path).await {
            Ok(Some((_record, version))) => version,
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(%error, %code, "stale telegram pairing code cleanup read failed");
                return;
            }
        };
        if let Err(error) = self
            .filesystem
            .delete_if_version(&self.scope, &path, version)
            .await
            && !matches!(error, FilesystemError::NotFound { .. })
        {
            tracing::debug!(%error, %code, "stale telegram pairing code cleanup failed");
        }
    }
}

fn map_fs_pairing(error: FilesystemError) -> TelegramPairingError {
    TelegramPairingError::StoreUnavailable {
        reason: error.to_string(),
    }
}

fn decode_pairing_completion(
    bytes: &[u8],
) -> Result<StoredPairingCompletion, TelegramPairingError> {
    serde_json::from_slice(bytes).map_err(|error| TelegramPairingError::StoreUnavailable {
        reason: format!("stored telegram pairing completion is invalid JSON: {error}"),
    })
}

fn decode_pairing_record(bytes: &[u8]) -> Result<TelegramPairingRecord, TelegramPairingError> {
    serde_json::from_slice(bytes).map_err(|error| TelegramPairingError::StoreUnavailable {
        reason: format!("stored telegram pairing record is invalid JSON: {error}"),
    })
}

fn encode_pairing_record(value: &TelegramPairingRecord) -> Result<Entry, TelegramPairingError> {
    let body =
        serde_json::to_vec(value).map_err(|error| TelegramPairingError::StoreUnavailable {
            reason: format!("telegram pairing record could not be serialized: {error}"),
        })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn decode_pairing_pointer(bytes: &[u8]) -> Result<StoredPairingUserPointer, TelegramPairingError> {
    serde_json::from_slice(bytes).map_err(|error| TelegramPairingError::StoreUnavailable {
        reason: format!("stored telegram pairing pointer is invalid JSON: {error}"),
    })
}

fn encode_pairing_pointer(value: &StoredPairingUserPointer) -> Result<Entry, TelegramPairingError> {
    let body =
        serde_json::to_vec(value).map_err(|error| TelegramPairingError::StoreUnavailable {
            reason: format!("telegram pairing pointer could not be serialized: {error}"),
        })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn encode_pairing_completion(
    value: &StoredPairingCompletion,
) -> Result<Entry, TelegramPairingError> {
    let body =
        serde_json::to_vec(value).map_err(|error| TelegramPairingError::StoreUnavailable {
            reason: format!("telegram pairing completion could not be serialized: {error}"),
        })?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn map_cas_pairing(error: CasUpdateError<TelegramPairingError>) -> TelegramPairingError {
    match error {
        CasUpdateError::Apply(error) => error,
        error => TelegramPairingError::StoreUnavailable {
            reason: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use ironclaw_host_api::{TenantId, UserId};
    use ironclaw_product_adapters::AdapterInstallationId;

    use super::*;
    use crate::test_support::{fault_injected_telegram_state, telegram_state};

    fn user(value: &str) -> UserId {
        UserId::new(value).expect("user")
    }

    fn live_record(code: &str, user_id: &str) -> TelegramPairingRecord {
        let now = Utc::now();
        TelegramPairingRecord {
            code: PairingCode::parse(code).expect("valid pairing code"),
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: user(user_id),
            installation_id: AdapterInstallationId::new("tg-bot-1").expect("installation"),
            created_at: now,
            expires_at: now + Duration::minutes(15),
            consumed_at: None,
        }
    }

    #[tokio::test]
    async fn state_rotation_invalidates_the_previous_code() {
        let state = telegram_state();
        state
            .upsert_pending_pairing(live_record("ABCD2345", "ben"))
            .await
            .expect("first code");
        state
            .upsert_pending_pairing(live_record("EFGH6789", "ben"))
            .await
            .expect("rotated code");

        assert!(
            state
                .pairing_for_code(&PairingCode::parse("ABCD2345").expect("valid pairing code"))
                .await
                .expect("old code read")
                .is_none()
        );
        assert_eq!(
            state
                .live_pairing_for_user(&user("ben"))
                .await
                .expect("live code")
                .map(|record| record.code),
            Some(PairingCode::parse("EFGH6789").expect("valid pairing code"))
        );
    }

    #[tokio::test]
    async fn concurrent_rotations_publish_exactly_one_authoritative_code() {
        let (state, filesystem) = fault_injected_telegram_state();
        state
            .upsert_pending_pairing(live_record("ABCD2345", "ben"))
            .await
            .expect("initial code");
        let read_gate = std::sync::Arc::new(tokio::sync::Barrier::new(3));
        filesystem.hold_next_reads_at(2, std::sync::Arc::clone(&read_gate));

        let first_state = std::sync::Arc::clone(&state);
        let first = tokio::spawn(async move {
            first_state
                .upsert_pending_pairing(live_record("EFGH6789", "ben"))
                .await
        });
        let second_state = std::sync::Arc::clone(&state);
        let second = tokio::spawn(async move {
            second_state
                .upsert_pending_pairing(live_record("JKLM2345", "ben"))
                .await
        });
        read_gate.wait().await;

        let results = [
            first.await.expect("first task joins"),
            second.await.expect("second task joins"),
        ];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(TelegramPairingError::ConcurrentUpdate)))
                .count(),
            1
        );
        let authoritative = state
            .live_pairing_for_user(&user("ben"))
            .await
            .expect("authoritative code reads")
            .expect("one code remains")
            .code;
        assert!(
            authoritative == PairingCode::parse("EFGH6789").expect("first code")
                || authoritative == PairingCode::parse("JKLM2345").expect("second code")
        );
    }

    #[tokio::test]
    async fn state_claim_is_single_consumer_and_keeps_the_receipt() {
        let state = telegram_state();
        state
            .upsert_pending_pairing(live_record("ABCD2345", "ben"))
            .await
            .expect("upsert");

        let claimed = state
            .claim_pairing(&PairingCode::parse("ABCD2345").expect("valid pairing code"))
            .await
            .expect("claim")
            .expect("first claim wins");
        assert!(claimed.consumed_at.is_some());
        assert!(
            state
                .claim_pairing(&PairingCode::parse("ABCD2345").expect("valid pairing code"))
                .await
                .expect("second claim")
                .is_none()
        );
        assert!(
            state
                .pairing_for_code(&PairingCode::parse("ABCD2345").expect("valid pairing code"))
                .await
                .expect("receipt read")
                .is_some_and(|record| record.consumed_at.is_some())
        );
    }

    #[tokio::test]
    async fn state_claim_refuses_expired_codes() {
        let state = telegram_state();
        let mut record = live_record("EFGH6789", "ben");
        record.expires_at = Utc::now() - Duration::seconds(1);
        state.upsert_pending_pairing(record).await.expect("upsert");

        assert!(
            state
                .claim_pairing(&PairingCode::parse("EFGH6789").expect("valid pairing code"))
                .await
                .expect("claim")
                .is_none()
        );
    }

    #[tokio::test]
    async fn state_claim_reports_non_conflict_cas_failure() {
        let (state, filesystem) = fault_injected_telegram_state();
        state
            .upsert_pending_pairing(live_record("JKLM2345", "ben"))
            .await
            .expect("upsert");
        filesystem.fail_versioned_writes();

        let error = state
            .claim_pairing(&PairingCode::parse("JKLM2345").expect("valid pairing code"))
            .await
            .expect_err("backend CAS fault is not a consumed-code conflict");
        assert!(matches!(
            error,
            TelegramPairingError::StoreUnavailable { .. }
        ));
    }
}
