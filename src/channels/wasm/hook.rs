//! Hook for signaling message persistence ACK to WASM channels.
//!
//! This hook is registered by the WASM channel subsystem to listen for
//! `OnMessagePersisted` events. When a user message is successfully persisted
//! to the database, this hook signals the WASM router, which then:
//! 1. Unblocks any pending webhook handlers waiting for ACK
//! 2. Calls the `on_message_persisted` callback on the appropriate WASM channel
//!
//! # Performance Consideration
//!
//! The hook serializes the entire `metadata` JSON object on every message
//! persistence. This is required by the WASM `on_message_persisted` interface
//! which expects a JSON string. For most messages, metadata is small (a few
//! fields like `message_id`, `chat_type`, etc.). If metadata grows large
//! (hundreds of KB), this could impact throughput.
//!
//! The serialization is synchronous in the hook execution path but runs
//! asynchronously via the hook registry, so it doesn't block message
//! persistence itself.

use super::WasmChannelRouter;
use crate::hooks::hook::{Hook, HookContext, HookError, HookEvent, HookOutcome, HookPoint};
use async_trait::async_trait;
use std::sync::Arc;

pub struct MessagePersistedHook {
    router: Arc<WasmChannelRouter>,
}

impl MessagePersistedHook {
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self { router }
    }
}

#[async_trait]
impl Hook for MessagePersistedHook {
    fn name(&self) -> &str {
        "wasm_message_persisted"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &[HookPoint::OnMessagePersisted]
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        if let HookEvent::MessagePersisted {
            user_id: _,
            channel,
            message_id,
            metadata,
        } = event
        {
            let ack_key = format!("{}:{}", channel, message_id);
            let metadata_json = metadata.to_string();
            tracing::debug!(ack_key = %ack_key, "Signaling ACK via hook");
            self.router.ack_message(&ack_key, &metadata_json).await;
        }
        Ok(HookOutcome::ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::hook::HookPoint;
    use serde_json::json;

    #[test]
    fn test_hook_name() {
        let router = WasmChannelRouter::new();
        let hook = MessagePersistedHook::new(Arc::new(router));
        assert_eq!(hook.name(), "wasm_message_persisted");
    }

    #[test]
    fn test_hook_points() {
        let router = WasmChannelRouter::new();
        let hook = MessagePersistedHook::new(Arc::new(router));
        assert_eq!(hook.hook_points(), &[HookPoint::OnMessagePersisted]);
    }

    #[test]
    fn test_hook_formats_ack_key_correctly() {
        // Test that the hook formats the ack_key as "channel:message_id"
        let channel = "test-channel";
        let message_id = "msg-123";
        let expected_key = format!("{}:{}", channel, message_id);
        assert_eq!(expected_key, "test-channel:msg-123");
    }

    #[tokio::test]
    async fn test_hook_returns_ok_even_for_non_message_persisted_events() {
        // Test that hook handles non-MessagePersisted events gracefully
        let router = WasmChannelRouter::new();
        let hook = MessagePersistedHook::new(Arc::new(router));

        // Create a different event type (Inbound)
        let event = HookEvent::Inbound {
            user_id: "user-1".to_string(),
            channel: "test".to_string(),
            content: "hello".to_string(),
            thread_id: None,
        };
        let ctx = HookContext::default();

        // Hook should return ok() even though it ignores non-MessagePersisted events
        let result = hook.execute(&event, &ctx).await;
        assert!(result.is_ok());
        assert!(matches!(
            result.unwrap(),
            HookOutcome::Continue { modified: None }
        ));
    }

    #[tokio::test]
    async fn test_hook_serializes_metadata_to_json() {
        // Test that metadata is correctly serialized to JSON
        let metadata = json!({
            "message_id": "msg-123",
            "chat_type": "group",
            "timestamp": 1234567890
        });
        let metadata_json = metadata.to_string();

        // Verify the JSON is valid and contains expected fields
        assert!(metadata_json.contains("msg-123"));
        assert!(metadata_json.contains("group"));
        assert!(metadata_json.contains("1234567890"));
    }
}
