//! Telegram rendering-adapter assembly for one setup revision.
//!
//! Owns every concrete `ironclaw_telegram_v2_adapter` construction detail
//! (adapter id, group-trigger policy, auth requirement, egress declaration)
//! so composition's wiring layer only decides WHEN a revision's adapter is
//! rebuilt, never WHAT Telegram's adapter looks like.

use std::sync::Arc;

use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressTarget, EgressCredentialHandle,
    ProductAdapter, ProductAdapterId,
};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
};
use thiserror::Error;

use crate::egress::TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE;
use crate::ingress::TELEGRAM_SECRET_TOKEN_HEADER;
use crate::setup::TelegramInstallationSetup;
use crate::telegram_actor_identity::TELEGRAM_V2_ADAPTER_ID;

#[derive(Debug, Error)]
#[error("invalid Telegram adapter config field {field}: {reason}")]
pub struct TelegramAdapterConfigError {
    pub field: &'static str,
    pub reason: String,
}

fn invalid(field: &'static str, reason: impl std::fmt::Display) -> TelegramAdapterConfigError {
    TelegramAdapterConfigError {
        field,
        reason: reason.to_string(),
    }
}

/// The `telegram_bot_token` egress credential handle.
pub fn telegram_bot_token_handle() -> Result<EgressCredentialHandle, TelegramAdapterConfigError> {
    EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE)
        .map_err(|reason| invalid("bot_token_handle", reason))
}

/// The declared egress targets Telegram's policy-scoped egress allows, each
/// keyed by the bot-token credential handle.
pub fn telegram_declared_egress_targets(
    token_handle: EgressCredentialHandle,
) -> Vec<DeclaredEgressTarget> {
    telegram_declared_egress_hosts()
        .into_iter()
        .map(|host| DeclaredEgressTarget::new(host, Some(token_handle.clone())))
        .collect()
}

/// The Telegram rendering adapter for one setup revision (installation id +
/// group trigger policy come from the setup record; DM-only hosts wire
/// `recognized_commands` empty and progress push off).
pub fn telegram_adapter_for_setup(
    setup: &TelegramInstallationSetup,
    installation_id: AdapterInstallationId,
    token_handle: EgressCredentialHandle,
) -> Result<Arc<dyn ProductAdapter>, TelegramAdapterConfigError> {
    let adapter_id = ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID)
        .map_err(|reason| invalid("adapter_id", reason))?;
    Ok(Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
        adapter_id,
        installation_id,
        group_trigger_policy: GroupTriggerPolicy {
            bot_username: setup.bot_username.clone(),
            bot_user_id: setup.bot_id,
            recognized_commands: vec![],
        },
        egress_credential_handle: token_handle,
        auth_requirement: AuthRequirement::SharedSecretHeader {
            header_name: TELEGRAM_SECRET_TOKEN_HEADER.into(),
        },
        progress_push_enabled: false,
    })))
}
