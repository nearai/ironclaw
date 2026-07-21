use std::sync::Arc;

use ironclaw_filesystem::{Fault, FaultInjecting, FilesystemOperation, InMemoryBackend};
use ironclaw_host_api::{AgentId, TenantId, UserId};
use ironclaw_secrets::{FilesystemSecretStore, SecretStore};
use secrecy::{ExposeSecret, SecretString};

use super::*;
use crate::state::FilesystemTelegramHostState;
use crate::test_support::{
    RecordedBotApiCall as BotApiCall, RecordingBotApi, fault_injecting_telegram_state,
    telegram_state,
};

fn service_with(
    state: Arc<FilesystemTelegramHostState>,
    bot_api: Arc<RecordingBotApi>,
    public_base_url: Option<&str>,
) -> TelegramSetupService {
    service_with_secret_store(
        state,
        Arc::new(FilesystemSecretStore::ephemeral()),
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

/// The real [`FilesystemSecretStore`] over a shared [`FaultInjecting`] backend,
/// armed to fail secret **deletes** on the failed revision. Replaces the former
/// whole-trait `FaultInjectingSecretStore` fake: the store now runs its genuine
/// serialization, lease/CAS mechanics, and
/// `FilesystemError::Backend -> SecretStoreError::StoreUnavailable` mapping
/// under the injected backend fault. `nth(1)` targets the first (fail-closed)
/// compensation delete; the caller's retry then deletes cleanly (the fault is a
/// one-shot), mirroring the old fake's `fail_deletes()` / `accept_deletes()`
/// toggle without needing to disarm.
fn secret_store_failing_first_delete() -> Arc<FilesystemSecretStore<FaultInjecting<InMemoryBackend>>>
{
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()).with_fault(
        Fault::on(FilesystemOperation::Delete)
            .path("secrets")
            .nth(1)
            .backend("injected secret deletion outage"),
    ));
    Arc::new(FilesystemSecretStore::ephemeral_over(backend))
}

/// The real [`FilesystemSecretStore`] over a shared [`FaultInjecting`] backend,
/// armed to fail every secret **write**. Replaces the former whole-trait fake's
/// `fail_puts()`: the first secret persist hits the injected backend fault and
/// the store surfaces it as `SecretStoreError::StoreUnavailable`.
fn secret_store_failing_writes() -> Arc<FilesystemSecretStore<FaultInjecting<InMemoryBackend>>> {
    let backend = Arc::new(FaultInjecting::new(InMemoryBackend::new()).with_fault(
        Fault::on(FilesystemOperation::WriteFile)
            .path("secrets")
            .backend("injected secret put outage"),
    ));
    Arc::new(FilesystemSecretStore::ephemeral_over(backend))
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
async fn save_uses_explicit_https_webhook_override_instead_of_public_base() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://default.example"),
    );
    let override_url = "https://tunnel.example/custom/telegram";

    let (_, saved) = service
        .save_with_previous(TelegramInstallationSetupUpdate {
            bot_token: Some(SecretString::from("123:abc".to_string())),
            webhook_url_override: Some(format!("  {override_url}  ")),
        })
        .await
        .expect("override save succeeds");

    assert_eq!(saved.webhook_url, override_url);
    assert!(matches!(
        bot_api.calls().get(1),
        Some(BotApiCall::SetWebhook { url, .. }) if url == override_url
    ));
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
async fn concurrent_same_bot_saves_publish_one_winner_without_deleting_its_secrets() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    bot_api.hold_next_set_webhooks_at(2, Arc::new(tokio::sync::Barrier::new(2)));
    let service = Arc::new(service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    ));

    let first_service = Arc::clone(&service);
    let second_service = Arc::clone(&service);
    let (first, second) = tokio::join!(
        async move {
            first_service
                .save_with_previous(update_with_token("123:first"))
                .await
        },
        async move {
            second_service
                .save_with_previous(update_with_token("123:second"))
                .await
        }
    );

    let successes = [&first, &second]
        .into_iter()
        .filter(|result| result.is_ok())
        .count();
    let conflicts = [&first, &second]
        .into_iter()
        .filter(|result| matches!(result, Err(TelegramSetupError::ConcurrentUpdate)))
        .count();
    assert_eq!(successes, 1, "exactly one setup CAS may publish");
    assert_eq!(conflicts, 1, "the losing caller receives a conflict");

    let winning_token = service
        .bot_token()
        .await
        .expect("winner token resolves")
        .expect("winner token remains present");
    let token = winning_token.expose_secret();
    assert!(
        token == "123:first" || token == "123:second",
        "the losing save must clean only its attempt-scoped handles"
    );
    assert_eq!(
        service
            .current_setup()
            .await
            .expect("winner setup reads")
            .expect("winner setup exists")
            .revision,
        1
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
async fn clear_keeps_retryable_intent_when_delete_webhook_fails() {
    let store = telegram_state();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with(
        Arc::clone(&store),
        Arc::clone(&bot_api),
        Some("https://ironclaw.example"),
    );
    let (_, saved) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("save");
    bot_api.reject_delete_webhook(503);

    assert!(matches!(
        service
            .clear()
            .await
            .expect_err("provider outage fails clear"),
        TelegramSetupError::BotApi { .. }
    ));
    assert!(
        service
            .current_setup()
            .await
            .expect("normal read")
            .is_none(),
        "clearing setup must fail closed"
    );
    assert_eq!(
        store
            .telegram_installation_setup_for_cleanup()
            .await
            .expect("cleanup intent reads"),
        Some(saved),
        "provider failure must retain the handles needed for restart-safe retry"
    );

    bot_api.accept_delete_webhook();
    service.clear().await.expect("retry completes cleanup");
    assert!(
        store
            .telegram_installation_setup_for_cleanup()
            .await
            .expect("cleanup state reads")
            .is_none()
    );
}

#[tokio::test]
async fn clear_keeps_retryable_intent_when_secret_deletion_fails() {
    let store = telegram_state();
    // Pre-armed one-shot: the save writes secrets cleanly (no deletes), then the
    // first fail-closed compensation delete during `clear()` hits the injected
    // backend fault.
    let secret_store = secret_store_failing_first_delete();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with_secret_store(
        Arc::clone(&store),
        Arc::clone(&secret_store) as Arc<dyn SecretStore>,
        bot_api,
        Some("https://ironclaw.example"),
    );
    let (_, saved) = service
        .save_with_previous(update_with_token("123:abc"))
        .await
        .expect("save");

    assert!(matches!(
        service
            .clear()
            .await
            .expect_err("secret outage fails clear"),
        TelegramSetupError::SecretStoreUnavailable { .. }
    ));
    assert_eq!(
        store
            .telegram_installation_setup_for_cleanup()
            .await
            .expect("cleanup intent reads"),
        Some(saved),
        "secret deletion failure must retain durable cleanup metadata"
    );

    // The Delete fault was a one-shot (`nth(1)`); the retry deletes cleanly.
    service.clear().await.expect("retry completes cleanup");
    assert!(
        store
            .telegram_installation_setup_for_cleanup()
            .await
            .expect("cleanup state reads")
            .is_none()
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

#[tokio::test]
async fn rollback_keeps_intent_until_compensating_set_webhook_succeeds() {
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
    let (previous, second) = service
        .save_with_previous(update_with_token("123:rotated"))
        .await
        .expect("second save");
    bot_api.reject_set_webhook(503);

    assert!(matches!(
        service
            .rollback_failed_activation_save(&second, previous.as_ref())
            .await
            .expect_err("provider rollback outage surfaces"),
        TelegramSetupError::BotApi { .. }
    ));
    assert!(
        service
            .current_setup()
            .await
            .expect("normal read")
            .is_none()
    );
    assert_eq!(
        store
            .telegram_installation_setup_rollback_intent()
            .await
            .expect("rollback intent reads"),
        Some((second.clone(), Some(first.clone()), false))
    );

    bot_api.accept_set_webhook();
    service
        .rollback_failed_activation_save(&second, previous.as_ref())
        .await
        .expect("retry completes rollback");
    assert_eq!(service.current_setup().await.expect("read"), Some(first));
}

#[tokio::test]
async fn rollback_keeps_intent_until_failed_revision_secrets_are_deleted() {
    let store = telegram_state();
    // Both saves write secrets cleanly (no deletes); the first fail-closed
    // compensation delete during rollback hits the pre-armed one-shot fault.
    let secret_store = secret_store_failing_first_delete();
    let bot_api = Arc::new(RecordingBotApi::default());
    let service = service_with_secret_store(
        Arc::clone(&store),
        Arc::clone(&secret_store) as Arc<dyn SecretStore>,
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

    assert!(matches!(
        service
            .rollback_failed_activation_save(&second, previous.as_ref())
            .await
            .expect_err("secret cleanup outage surfaces"),
        TelegramSetupError::SecretStoreUnavailable { .. }
    ));
    assert_eq!(
        store
            .telegram_installation_setup_rollback_intent()
            .await
            .expect("rollback intent reads"),
        Some((second.clone(), Some(first.clone()), true))
    );

    // The Delete fault was a one-shot (`nth(1)`); the retry deletes cleanly.
    service
        .rollback_failed_activation_save(&second, previous.as_ref())
        .await
        .expect("retry completes rollback and secret cleanup");
    assert_eq!(service.current_setup().await.expect("read"), Some(first));
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
    // Pre-armed to fail every secret write, so the first persist fails.
    let secret_store = secret_store_failing_writes();
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
    let (store, backend) = fault_injecting_telegram_state();
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

    backend.add_fault(
        Fault::on(FilesystemOperation::WriteFile).backend("test-injected filesystem failure"),
    );
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

#[tokio::test]
async fn bot_swap_rollback_keeps_intent_until_compensating_delete_webhook_succeeds() {
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
        .expect("bot swap saves");
    bot_api.reject_delete_webhook(503);

    assert!(matches!(
        service
            .rollback_failed_activation_save(&second, previous.as_ref())
            .await
            .expect_err("provider rollback outage surfaces"),
        TelegramSetupError::BotApi { .. }
    ));
    assert_eq!(
        store
            .telegram_installation_setup_rollback_intent()
            .await
            .expect("rollback intent reads"),
        Some((second.clone(), Some(first.clone()), false))
    );

    bot_api.accept_delete_webhook();
    service
        .rollback_failed_activation_save(&second, previous.as_ref())
        .await
        .expect("retry completes bot-swap rollback");
    assert_eq!(service.current_setup().await.expect("read"), Some(first));
}
