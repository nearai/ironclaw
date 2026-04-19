//! Regression: `rebuild_state` must preserve every existing `GatewayState`
//! field across `with_*` builder calls.
//!
//! Before #2049 was fixed, `rebuild_state` initialized `tool_dispatcher: None`,
//! so the dispatcher injected by `with_tool_dispatcher` was silently dropped
//! by every subsequent `with_*` call (extension manager, store, etc.). The
//! gateway started up with `state.tool_dispatcher == None`, making the
//! channel-agnostic dispatch path unusable.

use std::sync::Arc;

use crate::channels::web::GatewayChannel;
use crate::channels::web::sse::DEFAULT_BROADCAST_BUFFER;
use crate::config::GatewayConfig;
use crate::config::SafetyConfig;
use crate::db::libsql::LibSqlBackend;
use crate::tools::ToolRegistry;
use crate::tools::dispatch::ToolDispatcher;
use ironclaw_safety::SafetyLayer;

fn test_gateway() -> GatewayChannel {
    GatewayChannel::new(
        GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("test-token".to_string()),
            workspace_read_scopes: vec![],
            memory_layers: vec![],
            oidc: None,
            max_connections: 100,
            broadcast_buffer: DEFAULT_BROADCAST_BUFFER,
        },
        "test-user".to_string(),
    )
}

#[tokio::test]
async fn tool_dispatcher_survives_subsequent_with_calls() {
    // Build a minimal real `ToolDispatcher`. We're not exercising the
    // dispatch path here — only verifying it stays attached to GatewayState
    // across `rebuild_state`.
    let registry = Arc::new(ToolRegistry::new());
    let safety = Arc::new(SafetyLayer::new(&SafetyConfig {
        max_output_length: 1024,
        injection_check_enabled: false,
    }));
    let dir = tempfile::tempdir().expect("tempdir");
    let backend = LibSqlBackend::new_local(&dir.path().join("test.db"))
        .await
        .expect("libsql backend");
    use crate::db::Database;
    backend.run_migrations().await.expect("migrations");
    let db: Arc<dyn crate::db::Database> = Arc::new(backend);
    let dispatcher = Arc::new(ToolDispatcher::new(registry, safety, Arc::clone(&db), None));

    // Inject the dispatcher first, then run several other `with_*` builder
    // calls (each of which goes through `rebuild_state`).
    let gw = test_gateway()
        .with_tool_dispatcher(Arc::clone(&dispatcher))
        .with_store(Arc::clone(&db))
        .with_db_auth(Arc::clone(&db));

    assert!(
        gw.state().tool_dispatcher.is_some(),
        "tool_dispatcher must survive subsequent `with_*` builder calls"
    );

    // And the surviving Arc should still point at the very dispatcher we
    // injected (not a freshly-built replacement).
    let preserved = gw.state().tool_dispatcher.as_ref().unwrap();
    assert!(
        Arc::ptr_eq(preserved, &dispatcher),
        "preserved dispatcher must be the same Arc instance, not a replacement"
    );
}
