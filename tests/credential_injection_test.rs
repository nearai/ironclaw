//! Tests for WASM channel credential injection.

use std::sync::Arc;
use ironclaw::channels::wasm::{
    ChannelCapabilities, PreparedChannelModule, WasmChannel, WasmChannelRuntime,
    WasmChannelRuntimeConfig,
};
use ironclaw::extensions::manager::inject_channel_credentials_from_secrets;
use ironclaw::pairing::PairingStore;
use ironclaw::secrets::{CreateSecretParams, SecretsStore};

fn create_test_channel(name: &str) -> Arc<WasmChannel> {
    let runtime = Arc::new(
        WasmChannelRuntime::new(WasmChannelRuntimeConfig::for_testing())
            .expect("Failed to create runtime"),
    );
    let prepared = Arc::new(PreparedChannelModule::for_testing(name, format!("Test: {}", name)));
    let capabilities = ChannelCapabilities::for_channel(name);

    Arc::new(WasmChannel::new(
        runtime,
        prepared,
        capabilities,
        "{}".to_string(),
        Arc::new(PairingStore::new()),
        None,
    ))
}

#[cfg(feature = "integration")]
mod with_secrets_store {
    use super::*;

    async fn create_test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        let db = ironclaw::db::create_test_database().await;
        let store = ironclaw::secrets::create_secrets_store(db).await.unwrap();
        Arc::new(store)
    }

    #[tokio::test]
    async fn test_inject_from_secrets_store() {
        let secrets = create_test_secrets_store().await;

        secrets
            .create(CreateSecretParams {
                user_id: "default".to_string(),
                name: "telegram_bot_token".to_string(),
                value: "test_token_123".to_string(),
                description: None,
            })
            .await
            .unwrap();

        secrets
            .create(CreateSecretParams {
                user_id: "default".to_string(),
                name: "telegram_api_key".to_string(),
                value: "test_key_456".to_string(),
                description: None,
            })
            .await
            .unwrap();

        let channel = create_test_channel("telegram");

        let count = inject_channel_credentials_from_secrets(
            &channel,
            Some(secrets.as_ref()),
            "telegram",
            "default",
        )
        .await
        .unwrap();

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_case_insensitive_matching() {
        let secrets = create_test_secrets_store().await;

        secrets
            .create(CreateSecretParams {
                user_id: "default".to_string(),
                name: "MyChannel_Token".to_string(),
                value: "test_value".to_string(),
                description: None,
            })
            .await
            .unwrap();

        let channel = create_test_channel("mychannel");

        let count = inject_channel_credentials_from_secrets(
            &channel,
            Some(secrets.as_ref()),
            "mychannel",
            "default",
        )
        .await
        .unwrap();

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_no_matching_secrets() {
        let secrets = create_test_secrets_store().await;

        secrets
            .create(CreateSecretParams {
                user_id: "default".to_string(),
                name: "other_secret".to_string(),
                value: "value".to_string(),
                description: None,
            })
            .await
            .unwrap();

        let channel = create_test_channel("telegram");

        let count = inject_channel_credentials_from_secrets(
            &channel,
            Some(secrets.as_ref()),
            "telegram",
            "default",
        )
        .await
        .unwrap();

        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn test_inject_without_secrets_store() {
    let channel = create_test_channel("test");

    // Set an environment variable that matches the channel prefix
    unsafe {
        std::env::set_var("TEST_API_KEY", "env-value-123");
    }

    let count =
        inject_channel_credentials_from_secrets(&channel, None, "test", "default")
            .await
            .unwrap();

    assert_eq!(count, 1);
    let creds = channel.get_credentials().await;
    assert_eq!(creds.get("TEST_API_KEY").unwrap(), "env-value-123");

    // Clean up
    unsafe {
        std::env::remove_var("TEST_API_KEY");
    }
}
