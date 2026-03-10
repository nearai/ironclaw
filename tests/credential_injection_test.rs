//! Tests for WASM channel credential injection.

use std::sync::Arc;
use ironclaw::channels::wasm::{
    ChannelCapabilities, PreparedChannelModule, WasmChannel, WasmChannelRuntime,
    WasmChannelRuntimeConfig,
};
use ironclaw::extensions::manager::inject_channel_credentials_from_secrets;
use ironclaw::pairing::PairingStore;

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

#[tokio::test]
async fn test_inject_without_secrets_store() {
    let channel = create_test_channel("test");

    let count =
        inject_channel_credentials_from_secrets(&channel, None, "test", "default")
            .await
            .unwrap();

    // Without secrets store and no matching env vars, count should be 0
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_case_insensitive_channel_name() {
    let channel = create_test_channel("MyChannel");

    let count =
        inject_channel_credentials_from_secrets(&channel, None, "mychannel", "default")
            .await
            .unwrap();

    assert_eq!(count, 0);
}
