use ironclaw_host_api::SecretHandle;
use secrecy::SecretString;

use super::{TelegramInstallationSetup, TelegramSetupError, TelegramSetupService};

impl TelegramSetupService {
    /// Restore the provider to whichever durable revision won publication.
    /// This is used after a save loses the setup CAS race: restoring the stale
    /// snapshot captured by that loser would overwrite the winning webhook.
    pub(super) async fn reconcile_remote_webhook_to_current(
        &self,
        attempted_bot_token: &SecretString,
        attempted_bot_id: i64,
    ) {
        let current = match self.current_setup().await {
            Ok(current) => current,
            Err(error) => {
                tracing::debug!(
                    reason = %error,
                    "current telegram setup unavailable during provider reconciliation"
                );
                return;
            }
        };
        let Some(current) = current else {
            self.best_effort_delete_webhook(attempted_bot_token).await;
            return;
        };
        if current.bot_id != attempted_bot_id {
            self.best_effort_delete_webhook(attempted_bot_token).await;
            return;
        }
        let current_token = match self.secret_material(&current.bot_token_handle).await {
            Ok(token) => token,
            Err(error) => {
                tracing::debug!(
                    reason = %error,
                    "winning telegram bot token unavailable during provider reconciliation"
                );
                return;
            }
        };
        let current_secret = match self.secret_material(&current.webhook_secret_handle).await {
            Ok(secret) => secret,
            Err(error) => {
                tracing::debug!(
                    reason = %error,
                    "winning telegram webhook secret unavailable during provider reconciliation"
                );
                return;
            }
        };
        if let Err(error) = self
            .bot_api
            .set_webhook(&current_token, &current.webhook_url, &current_secret)
            .await
        {
            tracing::debug!(
                reason = %error,
                "winning telegram webhook could not be restored after a concurrent save"
            );
        }
    }

    async fn compensate_remote_webhook_confirmed(
        &self,
        bot_token: &SecretString,
        new_bot_id: i64,
        previous: Option<&TelegramInstallationSetup>,
    ) -> Result<(), TelegramSetupError> {
        match previous {
            Some(previous_setup) if previous_setup.bot_id == new_bot_id => {
                let previous_secret = self
                    .secret_material(&previous_setup.webhook_secret_handle)
                    .await?;
                self.bot_api
                    .set_webhook(bot_token, &previous_setup.webhook_url, &previous_secret)
                    .await?;
            }
            _ => self.bot_api.delete_webhook(bot_token).await?,
        }
        Ok(())
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
        if !self
            .state
            .begin_telegram_installation_setup_rollback(saved, previous)
            .await?
        {
            // A newer revision won while activation failed; never overwrite it
            // with this stale rollback.
            return Ok(());
        }
        self.recover_pending_setup_rollback().await
    }

    pub(super) async fn recover_pending_setup_rollback(&self) -> Result<(), TelegramSetupError> {
        let Some((saved, previous, provider_compensated)) = self
            .state
            .telegram_installation_setup_rollback_intent()
            .await?
        else {
            return Ok(());
        };
        if !provider_compensated {
            let bot_token = self.secret_material(&saved.bot_token_handle).await?;
            self.compensate_remote_webhook_confirmed(&bot_token, saved.bot_id, previous.as_ref())
                .await?;
            if !self
                .state
                .mark_telegram_installation_setup_rollback_provider_compensated(
                    &saved,
                    previous.as_ref(),
                )
                .await?
            {
                return Err(TelegramSetupError::ConcurrentUpdate);
            }
        }
        // Once provider confirmation is durable, retries no longer need the
        // bot token. Both secrets can be deleted before the previous setup is
        // restored, even if final CAS publication later fails transiently.
        self.delete_secret(&saved.webhook_secret_handle).await?;
        self.delete_secret(&saved.bot_token_handle).await?;
        if !self
            .state
            .finish_telegram_installation_setup_rollback(&saved, previous.as_ref())
            .await?
        {
            return Err(TelegramSetupError::ConcurrentUpdate);
        }
        Ok(())
    }
}
