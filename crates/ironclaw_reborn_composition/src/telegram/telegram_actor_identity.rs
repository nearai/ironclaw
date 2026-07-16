//! Telegram external-actor → Reborn user resolution.
//!
//! Mirrors the Slack actor resolver: adapter/actor-kind gated, re-read on
//! every inbound update (revocation is observed immediately), with the
//! binding-epoch fast path for `is_current` rechecks. Provider identity is
//! installation-scoped (`{installation}:{telegram_user_id}` under provider
//! `telegram`), and the installation id embeds the bot id — so rotating the
//! same bot's token preserves pairings while a bot swap orphans them.

use std::sync::Arc;

use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_product_workflow::{
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductWorkflowError,
    ResolvedProductActorUser,
};

use crate::channel_identity::RebornUserIdentityLookup;

/// Identity provider key for Telegram bindings. Deliberately the bare vendor
/// name (never `telegram_personal`/`telegram_bot` — retired taxonomy).
pub(crate) const TELEGRAM_IDENTITY_PROVIDER: &str = "telegram";

/// The host-wired adapter id for the Telegram v2 adapter instance.
pub(crate) const TELEGRAM_V2_ADAPTER_ID: &str = "telegram_v2";

#[derive(Clone)]
pub(crate) struct TelegramUserIdentityActorResolver {
    lookup: Arc<dyn RebornUserIdentityLookup>,
}

impl TelegramUserIdentityActorResolver {
    pub(crate) fn new(lookup: Arc<dyn RebornUserIdentityLookup>) -> Self {
        Self { lookup }
    }
}

impl std::fmt::Debug for TelegramUserIdentityActorResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TelegramUserIdentityActorResolver(..)")
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for TelegramUserIdentityActorResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        if request.adapter_id.as_str() != TELEGRAM_V2_ADAPTER_ID {
            return Ok(None);
        }
        if request.external_actor_ref.kind()
            != ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND
        {
            return Ok(None);
        }
        let provider_user_id = telegram_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        self.lookup
            .resolve_user_identity_with_binding_epoch(TELEGRAM_IDENTITY_PROVIDER, &provider_user_id)
            .await
            .map(|resolved| {
                resolved.map(|(user_id, binding_epoch)| match binding_epoch {
                    Some(binding_epoch) => {
                        ResolvedProductActorUser::with_binding_epoch(user_id, binding_epoch)
                    }
                    None => ResolvedProductActorUser::new(user_id),
                })
            })
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })
    }

    async fn resolved_product_actor_user_is_current(
        &self,
        request: &ProductActorUserResolutionRequest,
        expected: &ResolvedProductActorUser,
    ) -> Result<bool, ProductWorkflowError> {
        if request.adapter_id.as_str() != TELEGRAM_V2_ADAPTER_ID
            || request.external_actor_ref.kind()
                != ironclaw_telegram_v2_adapter::TELEGRAM_USER_ACTOR_KIND
        {
            return Ok(false);
        }
        let Some(expected_epoch) = expected.binding_epoch.as_ref() else {
            return Ok(self
                .resolve_product_actor_user(request.clone())
                .await?
                .as_ref()
                == Some(expected));
        };
        let provider_user_id = telegram_user_identity_provider_user_id(
            &request.installation_id,
            request.external_actor_ref.id(),
        );
        self.lookup
            .user_identity_binding_epoch_is_current(
                TELEGRAM_IDENTITY_PROVIDER,
                &provider_user_id,
                &expected.user_id,
                expected_epoch,
            )
            .await
            .map_err(|error| ProductWorkflowError::BindingResolutionFailed {
                reason: error.to_string(),
            })
    }
}

pub(crate) fn telegram_user_identity_provider_user_id(
    installation_id: &AdapterInstallationId,
    telegram_user_id: &str,
) -> String {
    format!("{}:{telegram_user_id}", installation_id.as_str())
}
