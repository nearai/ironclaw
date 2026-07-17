use chrono::Utc;
use ironclaw_filesystem::{CasExpectation, FilesystemError};
use ironclaw_host_api::UserId;

use super::FilesystemTelegramHostState;
use super::records::{StoredPairingUserPointer, pairing_code_path, pairing_user_path};
use crate::pairing::{TelegramPairingError, TelegramPairingRecord};

impl FilesystemTelegramHostState {
    pub async fn upsert_pending_pairing(
        &self,
        record: TelegramPairingRecord,
    ) -> Result<(), TelegramPairingError> {
        let user_lock = self.lock_for(format!("telegram-pairing:{}", record.user_id.as_str()));
        let _held = user_lock.lock().await;
        let user_path = pairing_user_path(&record.user_id).map_err(map_fs_pairing)?;
        if let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        {
            let previous_path = pairing_code_path(&pointer.code).map_err(map_fs_pairing)?;
            match self.delete_record(&previous_path).await {
                Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
                Err(error) => return Err(map_fs_pairing(error)),
            }
        }
        let code_path = pairing_code_path(&record.code).map_err(map_fs_pairing)?;
        self.write_record(&code_path, &record, CasExpectation::Any)
            .await
            .map_err(map_fs_pairing)?;
        self.write_record(
            &user_path,
            &StoredPairingUserPointer {
                code: record.code.to_ascii_uppercase(),
            },
            CasExpectation::Any,
        )
        .await
        .map_err(map_fs_pairing)?;
        Ok(())
    }

    pub async fn pairing_for_code(
        &self,
        code: &str,
    ) -> Result<Option<TelegramPairingRecord>, TelegramPairingError> {
        let path = pairing_code_path(code).map_err(map_fs_pairing)?;
        Ok(self
            .read_record::<TelegramPairingRecord>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(record, _)| record))
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
        Ok(self
            .pairing_for_code(&pointer.code)
            .await?
            .filter(|record| record.is_live(Utc::now())))
    }

    pub async fn claim_pairing(
        &self,
        code: &str,
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
        match self
            .write_record(&path, &record, CasExpectation::Version(version))
            .await
        {
            Ok(_) => Ok(Some(record)),
            Err(FilesystemError::VersionMismatch { .. }) => Ok(None),
            Err(error) => Err(map_fs_pairing(error)),
        }
    }

    pub async fn invalidate_for_user(&self, user_id: &UserId) -> Result<(), TelegramPairingError> {
        let user_lock = self.lock_for(format!("telegram-pairing:{}", user_id.as_str()));
        let _held = user_lock.lock().await;
        let user_path = pairing_user_path(user_id).map_err(map_fs_pairing)?;
        let Some((pointer, _)) = self
            .read_record::<StoredPairingUserPointer>(&user_path)
            .await
            .map_err(map_fs_pairing)?
        else {
            return Ok(());
        };
        let code_path = pairing_code_path(&pointer.code).map_err(map_fs_pairing)?;
        match self.delete_record(&code_path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => {}
            Err(error) => return Err(map_fs_pairing(error)),
        }
        match self.delete_record(&user_path).await {
            Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_pairing(error)),
        }
    }
}

fn map_fs_pairing(error: FilesystemError) -> TelegramPairingError {
    TelegramPairingError::StoreUnavailable {
        reason: error.to_string(),
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
            code: code.to_string(),
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
                .pairing_for_code("ABCD2345")
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
            Some("EFGH6789".to_string())
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
            .claim_pairing("ABCD2345")
            .await
            .expect("claim")
            .expect("first claim wins");
        assert!(claimed.consumed_at.is_some());
        assert!(
            state
                .claim_pairing("ABCD2345")
                .await
                .expect("second claim")
                .is_none()
        );
        assert!(
            state
                .pairing_for_code("ABCD2345")
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
                .claim_pairing("EFGH6789")
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
            .claim_pairing("JKLM2345")
            .await
            .expect_err("backend CAS fault is not a consumed-code conflict");
        assert!(matches!(
            error,
            TelegramPairingError::StoreUnavailable { .. }
        ));
    }
}
