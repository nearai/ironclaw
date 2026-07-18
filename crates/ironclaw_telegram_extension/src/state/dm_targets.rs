use ironclaw_filesystem::{CasExpectation, FilesystemError};
use ironclaw_host_api::UserId;
use ironclaw_product_adapters::AdapterInstallationId;

use super::FilesystemTelegramHostState;
use super::records::dm_target_path;
use crate::pairing::{TelegramDmTarget, TelegramPairingError};

impl FilesystemTelegramHostState {
    pub async fn upsert_dm_target(
        &self,
        installation_id: &AdapterInstallationId,
        target: TelegramDmTarget,
    ) -> Result<(), TelegramPairingError> {
        let path = dm_target_path(installation_id, &target.user_id).map_err(map_fs_pairing)?;
        self.write_record(&path, &target, CasExpectation::Any)
            .await
            .map_err(map_fs_pairing)?;
        Ok(())
    }

    pub async fn dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<Option<TelegramDmTarget>, TelegramPairingError> {
        let path = dm_target_path(installation_id, user_id).map_err(map_fs_pairing)?;
        Ok(self
            .read_record::<TelegramDmTarget>(&path)
            .await
            .map_err(map_fs_pairing)?
            .map(|(target, _)| target))
    }

    pub async fn delete_dm_target_for_user(
        &self,
        installation_id: &AdapterInstallationId,
        user_id: &UserId,
    ) -> Result<(), TelegramPairingError> {
        let path = dm_target_path(installation_id, user_id).map_err(map_fs_pairing)?;
        match self.delete_record(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
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
    use ironclaw_host_api::UserId;
    use ironclaw_product_adapters::AdapterInstallationId;

    use super::*;
    use crate::test_support::telegram_state;

    #[tokio::test]
    async fn state_dm_target_is_scoped_by_installation_and_user() {
        let state = telegram_state();
        let user_id = UserId::new("ben").expect("user");
        let first = AdapterInstallationId::new("tg-bot-1").expect("installation");
        let second = AdapterInstallationId::new("tg-bot-2").expect("installation");
        state
            .upsert_dm_target(
                &first,
                TelegramDmTarget {
                    user_id: user_id.clone(),
                    chat_id: 555,
                },
            )
            .await
            .expect("target persists");

        assert_eq!(
            state
                .dm_target_for_user(&first, &user_id)
                .await
                .expect("target reads")
                .map(|target| target.chat_id),
            Some(555)
        );
        assert!(
            state
                .dm_target_for_user(&second, &user_id)
                .await
                .expect("target reads")
                .is_none()
        );
    }
}
