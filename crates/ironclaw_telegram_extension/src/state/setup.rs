use ironclaw_filesystem::{CasExpectation, FilesystemError};

use super::FilesystemTelegramHostState;
use super::records::setup_path;
use crate::telegram_setup::{TelegramInstallationSetup, TelegramSetupError};

impl FilesystemTelegramHostState {
    pub async fn get_telegram_installation_setup(
        &self,
    ) -> Result<Option<TelegramInstallationSetup>, TelegramSetupError> {
        let path = setup_path().map_err(map_fs_setup)?;
        Ok(self
            .read_record::<TelegramInstallationSetup>(&path)
            .await
            .map_err(map_fs_setup)?
            .map(|(record, _)| record))
    }

    pub async fn put_telegram_installation_setup(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<(), TelegramSetupError> {
        let path = setup_path().map_err(map_fs_setup)?;
        let guard = self.lock_for("telegram-setup".to_string());
        let _held = guard.lock().await;
        self.write_record(&path, setup, CasExpectation::Any)
            .await
            .map_err(map_fs_setup)?;
        Ok(())
    }

    pub async fn delete_telegram_installation_setup(&self) -> Result<(), TelegramSetupError> {
        let path = setup_path().map_err(map_fs_setup)?;
        match self.delete_record(&path).await {
            Ok(()) => Ok(()),
            Err(FilesystemError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_fs_setup(error)),
        }
    }
}

fn map_fs_setup(error: FilesystemError) -> TelegramSetupError {
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
            bot_token_handle: SecretHandle::new("telegram_bot_token_test").expect("token handle"),
            webhook_secret_handle: SecretHandle::new("telegram_webhook_secret_test")
                .expect("secret handle"),
            revision,
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn state_setup_round_trips_and_deletes_the_production_record() {
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
            Some(expected)
        );
        state
            .delete_telegram_installation_setup()
            .await
            .expect("setup deletes");
        assert!(
            state
                .get_telegram_installation_setup()
                .await
                .expect("setup reads")
                .is_none()
        );
    }

    #[tokio::test]
    async fn state_setup_delete_maps_filesystem_failure_to_store_unavailable() {
        let (state, filesystem) = fault_injected_telegram_state();
        state
            .put_telegram_installation_setup(&setup(1))
            .await
            .expect("setup persists before fault");
        filesystem.fail_deletes();

        assert_eq!(
            state
                .delete_telegram_installation_setup()
                .await
                .expect_err("delete fault is reported"),
            TelegramSetupError::StoreUnavailable
        );
    }
}
