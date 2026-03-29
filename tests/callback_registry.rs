use ironclaw::tools::callback::{CallbackMetadata, ToolCallbackRegistry};
use std::time::Duration;

#[tokio::test]
async fn test_register_and_check_pending() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let meta = CallbackMetadata {
        tool_name: "wallet_transact".into(),
        user_id: "user-1".into(),
        thread_id: Some("thread-1".into()),
        channel: "web".into(),
    };

    registry.register("corr-123".into(), meta).await;
    assert!(registry.is_pending("corr-123").await);
    assert!(!registry.is_pending("nonexistent").await);
}

#[tokio::test]
async fn test_cancel_removes_entry() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));

    let meta = CallbackMetadata {
        tool_name: "wallet_transact".into(),
        user_id: "user-1".into(),
        thread_id: None,
        channel: "cli".into(),
    };

    registry.register("corr-456".into(), meta).await;
    assert!(registry.is_pending("corr-456").await);

    registry.cancel("corr-456").await;
    assert!(!registry.is_pending("corr-456").await);
}

#[tokio::test]
async fn test_cancel_nonexistent_is_noop() {
    let registry = ToolCallbackRegistry::new(Duration::from_secs(300));
    registry.cancel("does-not-exist").await; // should not panic
}
