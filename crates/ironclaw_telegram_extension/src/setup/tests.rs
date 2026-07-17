use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ResourceScope, SecretHandle, TenantId, UserId};
use ironclaw_secrets::{InMemorySecretStore, SecretMaterial, SecretStore, SecretStoreError};
use secrecy::{ExposeSecret, SecretString};

use super::*;
use crate::state::FilesystemTelegramHostState;
use crate::test_support::{
    RecordedBotApiCall as BotApiCall, RecordingBotApi, fault_injected_telegram_state,
    telegram_state,
};

fn service_with(
    state: Arc<FilesystemTelegramHostState>,
    bot_api: Arc<RecordingBotApi>,
    public_base_url: Option<&str>,
) -> TelegramSetupService {
    service_with_secret_store(
        state,
        Arc::new(InMemorySecretStore::new()),
        bot_api,
        public_base_url,
    )
}

fn service_with_secret_store(
    state: Arc<FilesystemTelegramHostState>,
    secret_store: Arc<dyn SecretStore>,
    bot_api: Arc<RecordingBotApi>,
    public_base_url: Option<&str>,
) -> TelegramSetupService {
    TelegramSetupService::new(
        TenantId::new("tenant-a").expect("tenant"),
        AgentId::new("agent-a").expect("agent"),
        None,
        UserId::new("operator").expect("user"),
        state,
        secret_store,
        bot_api.client(),
        public_base_url.map(str::to_string),
    )
}

/// Delegating secret store whose `put` can be switched to fail —
/// everything else forwards to a real in-memory store.
#[derive(Debug)]
struct FailingPutSecretStore {
    inner: InMemorySecretStore,
    fail_puts: std::sync::atomic::AtomicBool,
}

impl FailingPutSecretStore {
    fn new() -> Self {
        Self {
            inner: InMemorySecretStore::new(),
            fail_puts: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn fail_puts(&self) {
        self.fail_puts
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl ironclaw_secrets::SecretStore for FailingPutSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
        expires_at: Option<ironclaw_host_api::Timestamp>,
    ) -> Result<ironclaw_secrets::SecretMetadata, SecretStoreError> {
        if self.fail_puts.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(SecretStoreError::StoreUnavailable {
                reason: "test secret outage".to_string(),
            });
        }
        self.inner.put(scope, handle, material, expires_at).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<ironclaw_secrets::SecretMetadata>, SecretStoreError> {
        self.inner.metadata(scope, handle).await
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_secrets::SecretMetadata>, SecretStoreError> {
        self.inner.metadata_for_scope(scope).await
    }

    async fn delete(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        self.inner.delete(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<ironclaw_secrets::SecretLease, SecretStoreError> {
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: ironclaw_secrets::SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: ironclaw_secrets::SecretLeaseId,
    ) -> Result<ironclaw_secrets::SecretLease, SecretStoreError> {
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_secrets::SecretLease>, SecretStoreError> {
        self.inner.leases_for_scope(scope).await
    }
}

fn update_with_token(token: &str) -> TelegramInstallationSetupUpdate {
    TelegramInstallationSetupUpdate {
        bot_token: Some(SecretString::from(token.to_string())),
        webhook_url_override: None,
    }
}

#[tokio::test]
async fn save_happy_path_validates_registers_and_persists() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );

    let (previous, saved) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("save succeeds");
    assert!(previous.is_none());
    assert_eq!(saved.bot_id, 4242);
    assert_eq!(saved.bot_username, "ironclaw_qa_bot");
    assert_eq!(
        saved.webhook_url,
        "https://ironclaw.example/webhooks/extensions/telegram/updates"
    );
    assert_eq!(saved.revision, 1);
    let calls = bot_api.calls();
    assert!(matches!(calls[0], BotApiCall::GetMe));
    match &calls[1] {
        BotApiCall::SetWebhook { url, .. } => assert_eq!(
            url, "https://ironclaw.example/webhooks/extensions/telegram/updates",
            "setWebhook must register the derived public updates URL"
        ),
        other => panic!("expected SetWebhook as the second bot api call, got {other:?}"),
    }
    let token = service.bot_token().await.expect("token resolves");
    assert_eq!(
        token.expect("token present").expose_secret(),
        "123:abc",
        "bot token round-trips through the secret store"
    );
    assert!(
        service
            .webhook_secret()
            .await
            .expect("secret resolves")
            .is_some()
    );
    let status = service.status().await.expect("status");
    assert!(status.configured && status.bot_token_configured);
    assert_eq!(status.bot_username.as_deref(), Some("ironclaw_qa_bot"));
}

#[tokio::test]
async fn invalid_token_persists_nothing() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    bot_api.reject_get_me(401);
    let service = service_with(
        Arc::clone(&store),
        bot_api,
        Some("https://ironclaw.example"),
    );
    let error = service
        .save_with_previous(update_with_token("bad"))
        .await
        .expect_err("save fails closed");
    assert!(matches!(error, TelegramSetupError::BotApi { .. }));
    assert!(service.current_setup().await.expect("read").is_none());
}

#[tokio::test]
async fn malformed_get_me_response_persists_nothing() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    bot_api.malformed_get_me_response();
    let service = service_with(
        Arc::clone(&store),
        bot_api,
        Some("https://ironclaw.example"),
    );

    let error = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect_err("malformed provider response fails closed");
    assert!(matches!(error, TelegramSetupError::BotApi { .. }));
    assert!(service.current_setup().await.expect("read").is_none());
}

#[tokio::test]
async fn set_webhook_failure_persists_nothing() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    bot_api.set_bot_identity(1, "b");
    bot_api.reject_set_webhook(400);
    let service = service_with(
        Arc::clone(&store),
        bot_api,
        Some("https://ironclaw.example"),
    );
    let error = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect_err("save fails closed");
    assert!(matches!(error, TelegramSetupError::BotApi { .. }));
    assert!(service.current_setup().await.expect("read").is_none());
}

#[tokio::test]
async fn missing_public_base_url_fails_before_any_bot_api_call_after_validation() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(Arc::clone(&store), Arc::clone(&bot_api), None);
    let error = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect_err("save fails closed");
    assert!(matches!(error, TelegramSetupError::PublicUrlMissing));
    assert!(
        !bot_api
            .calls()
            .iter()
            .any(|call| matches!(call, BotApiCall::SetWebhook { .. })),
        "webhook must not be registered without a public URL"
    );
    assert!(service.current_setup().await.expect("read").is_none());
}

#[tokio::test]
async fn rotation_bumps_revision_and_keeps_installation_identity() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        bot_api,
        Some("https://ironclaw.example"),
    );
    let (_, first) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("first save");
    let (previous, second) = service
        .save_with_previous(update_with_token("123:rotated"))
        .await
        .expect("second save");
    assert_eq!(previous.as_ref(), Some(&first));
    assert_eq!(second.revision, 2);
    assert_ne!(second.webhook_secret_handle, first.webhook_secret_handle);
    assert_eq!(
        second.installation_id().expect("id"),
        first.installation_id().expect("id"),
        "same bot keeps the installation identity"
    );
}

#[tokio::test]
async fn blank_token_keeps_existing_material() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        bot_api,
        Some("https://ironclaw.example"),
    );
    service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("first save");
    service
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("   ".to_string())),
            webhook_url_override: None,
        })
        .await
        .expect("blank token save reuses existing");
    let token = service.bot_token().await.expect("token").expect("present");
    assert_eq!(token.expose_secret(), "123:abc");
}

#[tokio::test]
async fn clear_deletes_webhook_and_record() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );
    service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("save");
    service.clear().await.expect("clear succeeds");
    assert!(service.current_setup().await.expect("read").is_none());
    assert!(
        bot_api
            .calls()
            .iter()
            .any(|call| matches!(call, BotApiCall::DeleteWebhook)),
        "clear must attempt deleteWebhook"
    );
}

#[tokio::test]
async fn rollback_restores_previous_record_and_previous_webhook_registration() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );
    let (_, first) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("first save");
    let first_secret = current_webhook_secret(&service).await;
    let (previous, second) = service
        .save_with_previous(update_with_token("123:rotated"))
        .await
        .expect("second save");
    service
        .rollback_failed_activation_save(&second, previous.as_ref())
        .await
        .expect("rollback");
    assert_eq!(service.current_setup().await.expect("read"), Some(first));
    // Telegram was registered with the SAVED secret; the rollback must
    // re-register the PREVIOUS one or the restored record rejects every
    // webhook until the admin re-saves.
    match bot_api.calls().last().expect("calls recorded") {
        BotApiCall::SetWebhook { secret, .. } => assert_eq!(
            secret, &first_secret,
            "provider rollback must restore the previous webhook secret"
        ),
        other => panic!("expected a compensating SetWebhook, got {other:?}"),
    }
}

async fn current_webhook_secret(service: &TelegramSetupService) -> String {
    service
        .webhook_secret()
        .await
        .expect("secret read")
        .expect("secret present")
        .expose_secret()
        .to_string()
}

/// Persistence fails after `setWebhook` on a first-time configure: the
/// fresh provider registration must be deleted (there is no previous one
/// to restore) so Telegram is not left delivering to a deployment that
/// never persisted the setup.
#[tokio::test]
async fn failed_secret_persist_deletes_fresh_webhook_when_no_previous() {
    let store = telegram_state();
    let secret_store = Arc::new(FailingPutSecretStore::new());
    secret_store.fail_puts();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with_secret_store(
        Arc::clone(&store),
        Arc::clone(&secret_store) as Arc<dyn SecretStore>,
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );

    let error = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect_err("save fails");
    assert!(matches!(
        error,
        TelegramSetupError::SecretStoreUnavailable { .. }
    ));
    assert!(service.current_setup().await.expect("read").is_none());
    assert!(
        matches!(bot_api.calls().last(), Some(BotApiCall::DeleteWebhook)),
        "the fresh webhook registration must be compensated away, got {:?}",
        bot_api.calls()
    );
}

/// A same-bot update whose record persist fails must restore the
/// PREVIOUS webhook registration at Telegram — otherwise Telegram keeps
/// signing with the new secret while the durable record still holds the
/// old one, and ingress rejects every webhook.
#[tokio::test]
async fn failed_record_persist_restores_previous_webhook_for_same_bot() {
    let (store, filesystem) = fault_injected_telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );
    let (_, first) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("first save");
    let first_secret = current_webhook_secret(&service).await;

    filesystem.fail_writes();
    let error = service
        .save_with_previous(update_with_token("123:rotated"))
        .await
        .expect_err("second save fails at the record persist");
    assert!(matches!(error, TelegramSetupError::StoreUnavailable));
    match bot_api.calls().last().expect("calls recorded") {
        BotApiCall::SetWebhook { url, secret } => {
            assert_eq!(url, &first.webhook_url);
            assert_eq!(
                secret, &first_secret,
                "compensation must re-register the previous secret"
            );
        }
        other => panic!("expected a compensating SetWebhook, got {other:?}"),
    }
    // The surviving record still verifies with its own secret.
    assert_eq!(current_webhook_secret(&service).await, first_secret);
}

/// Activation rollback after a bot swap: the OLD bot's registration was
/// never touched by the failed save, so the compensation deletes the NEW
/// bot's registration instead of re-registering anything.
#[tokio::test]
async fn rollback_after_bot_swap_deletes_the_new_bots_webhook() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );
    let (_, first) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("first save");
    bot_api.set_bot_identity(5555, "other_bot");
    let (previous, second) = service
        .save_with_previous(update_with_token("555:swap"))
        .await
        .expect("bot swap save");
    assert_ne!(second.bot_id, first.bot_id);

    service
        .rollback_failed_activation_save(&second, previous.as_ref())
        .await
        .expect("rollback");
    assert_eq!(service.current_setup().await.expect("read"), Some(first));
    assert!(
        matches!(bot_api.calls().last(), Some(BotApiCall::DeleteWebhook)),
        "bot-swap rollback must delete the new bot's registration, got {:?}",
        bot_api.calls()
    );
}
