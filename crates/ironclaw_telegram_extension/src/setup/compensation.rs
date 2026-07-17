use ironclaw_host_api::SecretHandle;
use secrecy::SecretString;

use super::{TelegramInstallationSetup, TelegramSetupError, TelegramSetupService};

impl TelegramSetupService {
    pub(super) async fn compensate_remote_webhook(
        &self,
        bot_token: &SecretString,
        new_bot_id: i64,
        previous: Option<&TelegramInstallationSetup>,
    ) {
        match previous {
            Some(previous_setup) if previous_setup.bot_id == new_bot_id => {
                let previous_secret = match self
                    .secret_material(&previous_setup.webhook_secret_handle)
                    .await
                {
                    Ok(secret) => secret,
                    Err(error) => {
                        tracing::debug!(
                            reason = %error,
                            "previous telegram webhook secret unavailable; deleting the fresh registration instead"
                        );
                        self.best_effort_delete_webhook(bot_token).await;
                        return;
                    }
                };
                if let Err(error) = self
                    .bot_api
                    .set_webhook(bot_token, &previous_setup.webhook_url, &previous_secret)
                    .await
                {
                    tracing::debug!(
                        reason = %error,
                        "telegram webhook compensation set_webhook failed"
                    );
                }
            }
            _ => self.best_effort_delete_webhook(bot_token).await,
        }
    }

    pub(super) async fn best_effort_delete_webhook(&self, bot_token: &SecretString) {
        if let Err(error) = self.bot_api.delete_webhook(bot_token).await {
            tracing::debug!(
                reason = %error,
                "telegram webhook compensation delete_webhook failed"
            );
        }
    }

    pub(super) async fn best_effort_delete_secret(&self, handle: &SecretHandle) {
        if let Err(error) = self.secret_store.delete(&self.secret_scope(), handle).await {
            tracing::debug!(
                reason = %error,
                "orphaned telegram setup secret cleanup failed"
            );
        }
    }

    pub async fn rollback_failed_activation_save(
        &self,
        saved: &TelegramInstallationSetup,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<(), TelegramSetupError> {
        let _save_guard = self.save_lock.lock().await;
        let current = self.current_setup().await?;
        if current.as_ref() != Some(saved) {
            return Ok(());
        }
        match self.secret_material(&saved.bot_token_handle).await {
            Ok(bot_token) => {
                self.compensate_remote_webhook(&bot_token, saved.bot_id, previous)
                    .await;
            }
            Err(error) => {
                tracing::debug!(
                    reason = %error,
                    "saved telegram bot token unavailable; skipping provider-side rollback"
                );
            }
        }
        match previous {
            Some(previous_setup) => {
                self.state
                    .put_telegram_installation_setup(previous_setup)
                    .await
            }
            None => self.state.delete_telegram_installation_setup().await,
        }
    }
}
