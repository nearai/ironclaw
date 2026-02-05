//! Integration tests for the WASM channel system.
//!
//! These tests verify the full flow of WASM channel operations:
//! - Channel loading from filesystem
//! - HTTP webhook routing
//! - Message emission and delivery
//! - Response handling

use std::collections::HashMap;
use std::sync::Arc;

use ironclaw::channels::Channel;
use ironclaw::channels::wasm::{
    ChannelCapabilities, EmitRateLimitConfig, PreparedChannelModule, RegisteredEndpoint,
    WasmChannel, WasmChannelRouter, WasmChannelRuntime, WasmChannelRuntimeConfig,
};
use tempfile::TempDir;

/// Create a test runtime for WASM channel operations.
fn create_test_runtime() -> Arc<WasmChannelRuntime> {
    let config = WasmChannelRuntimeConfig::for_testing();
    Arc::new(WasmChannelRuntime::new(config).expect("Failed to create runtime"))
}

/// Create a test channel with minimal configuration.
fn create_test_channel(
    runtime: Arc<WasmChannelRuntime>,
    name: &str,
    paths: Vec<&str>,
) -> WasmChannel {
    let prepared = Arc::new(PreparedChannelModule::for_testing(
        name,
        format!("Test channel: {}", name),
    ));

    let mut capabilities = ChannelCapabilities::for_channel(name);
    for path in paths {
        capabilities = capabilities.with_path(path.to_string());
    }

    WasmChannel::new(runtime, prepared, capabilities, "{}".to_string())
}

mod router_tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_route_channel() {
        let router = WasmChannelRouter::new();
        let runtime = create_test_runtime();

        let channel = Arc::new(create_test_channel(
            runtime,
            "test-channel",
            vec!["/webhook/test"],
        ));

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "test-channel".to_string(),
            path: "/webhook/test".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router
            .register(channel.clone(), endpoints, None, None)
            .await;

        // Verify channel is found by path
        let found = router.get_channel_for_path("/webhook/test").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().channel_name(), "test-channel");

        // Verify non-existent path returns None
        let not_found = router.get_channel_for_path("/webhook/nonexistent").await;
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_secret_validation() {
        let router = WasmChannelRouter::new();
        let runtime = create_test_runtime();

        let channel = Arc::new(create_test_channel(
            runtime,
            "secure-channel",
            vec!["/webhook/secure"],
        ));

        router
            .register(channel, vec![], Some("my-secret-123".to_string()), None)
            .await;

        // Correct secret validates
        assert!(
            router
                .validate_secret("secure-channel", "my-secret-123")
                .await
        );

        // Wrong secret fails
        assert!(
            !router
                .validate_secret("secure-channel", "wrong-secret")
                .await
        );

        // Non-existent channel without secret always validates
        assert!(router.validate_secret("nonexistent", "anything").await);
    }

    #[tokio::test]
    async fn test_unregister_channel() {
        let router = WasmChannelRouter::new();
        let runtime = create_test_runtime();

        let channel = Arc::new(create_test_channel(
            runtime,
            "temp-channel",
            vec!["/webhook/temp"],
        ));

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "temp-channel".to_string(),
            path: "/webhook/temp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router.register(channel, endpoints, None, None).await;

        // Channel exists
        assert!(router.get_channel_for_path("/webhook/temp").await.is_some());

        // Unregister
        router.unregister("temp-channel").await;

        // Channel no longer exists
        assert!(router.get_channel_for_path("/webhook/temp").await.is_none());
    }

    #[tokio::test]
    async fn test_multiple_channels() {
        let router = WasmChannelRouter::new();
        let runtime = create_test_runtime();

        // Register multiple channels
        for name in &["slack", "telegram", "discord"] {
            let channel = Arc::new(create_test_channel(
                Arc::clone(&runtime),
                name,
                vec![&format!("/webhook/{}", name)],
            ));

            let endpoints = vec![RegisteredEndpoint {
                channel_name: name.to_string(),
                path: format!("/webhook/{}", name),
                methods: vec!["POST".to_string()],
                require_secret: false,
            }];

            router.register(channel, endpoints, None, None).await;
        }

        // Verify all channels are registered
        let channels = router.list_channels().await;
        assert_eq!(channels.len(), 3);
        assert!(channels.contains(&"slack".to_string()));
        assert!(channels.contains(&"telegram".to_string()));
        assert!(channels.contains(&"discord".to_string()));

        // Verify all paths work
        for name in &["slack", "telegram", "discord"] {
            let found = router
                .get_channel_for_path(&format!("/webhook/{}", name))
                .await;
            assert!(found.is_some());
            assert_eq!(found.unwrap().channel_name(), *name);
        }
    }
}

mod channel_lifecycle_tests {
    use super::*;

    #[tokio::test]
    async fn test_channel_start_and_shutdown() {
        let runtime = create_test_runtime();
        let channel = create_test_channel(runtime, "lifecycle-test", vec!["/webhook/lifecycle"]);

        // Start channel
        let stream = channel.start().await;
        assert!(stream.is_ok());

        // Health check should pass
        assert!(channel.health_check().await.is_ok());

        // Shutdown
        assert!(channel.shutdown().await.is_ok());

        // Health check should fail after shutdown
        assert!(channel.health_check().await.is_err());
    }

    #[tokio::test]
    async fn test_channel_http_callback() {
        let runtime = create_test_runtime();
        let channel = create_test_channel(runtime, "http-test", vec!["/webhook/http"]);

        // Start channel
        let _stream = channel.start().await.expect("Failed to start channel");

        // Call HTTP callback (stub implementation returns 200 OK)
        let response = channel
            .call_on_http_request(
                "POST",
                "/webhook/http",
                &HashMap::new(),
                &HashMap::new(),
                b"{}",
                true,
            )
            .await
            .expect("HTTP callback failed");

        assert_eq!(response.status, 200);

        // Cleanup
        channel.shutdown().await.expect("Shutdown failed");
    }
}

mod loader_tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_discover_channels_empty_dir() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        let channels = ironclaw::channels::wasm::discover_channels(dir.path())
            .await
            .expect("Discovery failed");

        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_channels_with_wasm_files() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        // Create fake WASM files
        std::fs::File::create(dir.path().join("slack.wasm")).expect("Failed to create file");
        std::fs::File::create(dir.path().join("telegram.wasm")).expect("Failed to create file");

        let channels = ironclaw::channels::wasm::discover_channels(dir.path())
            .await
            .expect("Discovery failed");

        assert_eq!(channels.len(), 2);
        assert!(channels.contains_key("slack"));
        assert!(channels.contains_key("telegram"));
    }

    #[tokio::test]
    async fn test_discover_channels_with_capabilities() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        // Create WASM and capabilities file
        std::fs::File::create(dir.path().join("custom.wasm")).expect("Failed to create wasm");

        let mut cap_file = std::fs::File::create(dir.path().join("custom.capabilities.json"))
            .expect("Failed to create capabilities");
        cap_file
            .write_all(
                br#"{
                "name": "custom",
                "capabilities": {
                    "channel": {
                        "allowed_paths": ["/webhook/custom"]
                    }
                }
            }"#,
            )
            .expect("Failed to write capabilities");

        let channels = ironclaw::channels::wasm::discover_channels(dir.path())
            .await
            .expect("Discovery failed");

        assert_eq!(channels.len(), 1);
        assert!(channels["custom"].capabilities_path.is_some());
    }

    #[tokio::test]
    async fn test_discover_channels_ignores_non_wasm() {
        let dir = TempDir::new().expect("Failed to create temp dir");

        // Create various non-WASM files
        std::fs::File::create(dir.path().join("readme.md")).expect("Failed to create file");
        std::fs::File::create(dir.path().join("config.json")).expect("Failed to create file");
        std::fs::File::create(dir.path().join("channel.wasm")).expect("Failed to create file");

        let channels = ironclaw::channels::wasm::discover_channels(dir.path())
            .await
            .expect("Discovery failed");

        // Only the .wasm file should be discovered
        assert_eq!(channels.len(), 1);
        assert!(channels.contains_key("channel"));
    }
}

mod capabilities_tests {
    use super::*;

    #[test]
    fn test_capabilities_path_validation() {
        let caps = ChannelCapabilities::for_channel("test")
            .with_path("/webhook/test")
            .with_path("/api/events");

        assert!(caps.is_path_allowed("/webhook/test"));
        assert!(caps.is_path_allowed("/api/events"));
        assert!(!caps.is_path_allowed("/other/path"));
    }

    #[test]
    fn test_capabilities_workspace_prefix() {
        let caps = ChannelCapabilities::for_channel("slack");

        assert_eq!(caps.workspace_prefix, "channels/slack/");

        // Validate path prefixing
        let prefixed = caps.prefix_workspace_path("state.json");
        assert_eq!(prefixed, "channels/slack/state.json");
    }

    #[test]
    fn test_capabilities_workspace_path_validation() {
        let caps = ChannelCapabilities::for_channel("test");

        // Valid paths
        assert!(caps.validate_workspace_path("state.json").is_ok());
        assert!(caps.validate_workspace_path("data/file.txt").is_ok());

        // Invalid paths (traversal attempts)
        assert!(caps.validate_workspace_path("../escape.txt").is_err());
        assert!(caps.validate_workspace_path("/absolute/path").is_err());
        assert!(caps.validate_workspace_path("data/../escape").is_err());
    }

    #[test]
    fn test_capabilities_poll_interval_validation() {
        let caps = ChannelCapabilities::for_channel("test").with_polling(30_000);

        // Valid interval (returns as-is)
        let result = caps.validate_poll_interval(60_000);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 60_000);

        // Too short interval is clamped to minimum (not rejected)
        let result = caps.validate_poll_interval(1_000);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 30_000);

        // Minimum interval passes as-is
        let result = caps.validate_poll_interval(30_000);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 30_000);

        // Polling disabled returns error
        let no_poll_caps = ChannelCapabilities::for_channel("no-poll");
        assert!(no_poll_caps.validate_poll_interval(60_000).is_err());
    }

    #[test]
    fn test_emit_rate_limit_config() {
        let config = EmitRateLimitConfig {
            messages_per_minute: 100,
            messages_per_hour: 5000,
        };

        assert_eq!(config.messages_per_minute, 100);
        assert_eq!(config.messages_per_hour, 5000);
    }
}

mod message_emission_tests {
    use super::*;
    use ironclaw::channels::wasm::{ChannelHostState, EmittedMessage};

    #[test]
    fn test_emit_message_basic() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello, world!");
        state.emit_message(msg).expect("Emit should succeed");

        assert_eq!(state.emitted_count(), 1);

        let messages = state.take_emitted_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].user_id, "user123");
        assert_eq!(messages[0].content, "Hello, world!");

        // Queue should be cleared
        assert_eq!(state.emitted_count(), 0);
    }

    #[test]
    fn test_emit_message_with_metadata() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        let msg = EmittedMessage::new("user123", "Hello")
            .with_user_name("John Doe")
            .with_thread_id("thread-1")
            .with_metadata(r#"{"channel": "C123"}"#);

        state.emit_message(msg).expect("Emit should succeed");

        let messages = state.take_emitted_messages();
        assert_eq!(messages[0].user_name, Some("John Doe".to_string()));
        assert_eq!(messages[0].thread_id, Some("thread-1".to_string()));
        assert!(messages[0].metadata_json.contains("channel"));
    }

    #[test]
    fn test_emit_rate_limiting() {
        let caps = ChannelCapabilities::for_channel("test");
        let mut state = ChannelHostState::new("test", caps);

        // Emit up to the per-execution limit
        for i in 0..100 {
            let msg = EmittedMessage::new("user", format!("Message {}", i));
            state.emit_message(msg).expect("Emit should succeed");
        }

        // Messages beyond the limit are silently dropped
        let msg = EmittedMessage::new("user", "Should be dropped");
        state.emit_message(msg).expect("Emit should not fail");

        assert_eq!(state.emitted_count(), 100);
        assert_eq!(state.emits_dropped(), 1);
    }
}
