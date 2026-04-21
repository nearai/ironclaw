//! Integration coverage for multi-user MCP isolation on the same server.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use ironclaw::context::JobContext;
    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::extensions::{ExtensionKind, ExtensionManager};
    use ironclaw::secrets::{
        CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore,
    };
    use ironclaw::tools::ToolRegistry;
    use ironclaw::tools::mcp::{McpProcessManager, McpServerConfig, McpSessionManager};
    use secrecy::SecretString;

    use crate::support::mock_mcp_server::{MockToolResponse, start_mock_mcp_server};

    const SERVER_NAME: &str = "shared_mcp";
    const USER_A: &str = "user-a";
    const USER_B: &str = "user-b";
    const TEST_CRYPTO_KEY: &str = "0123456789abcdef0123456789abcdef";

    fn test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(TEST_CRYPTO_KEY.to_string()))
                .expect("test crypto"),
        );
        Arc::new(InMemorySecretsStore::new(crypto))
    }

    async fn test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("test.db");
        let backend = LibSqlBackend::new_local(&path)
            .await
            .expect("failed to create test LibSqlBackend");
        backend
            .run_migrations()
            .await
            .expect("failed to run migrations");
        (Arc::new(backend) as Arc<dyn Database>, dir)
    }

    async fn activate_for_user(
        manager: &ExtensionManager,
        secrets: &Arc<dyn SecretsStore + Send + Sync>,
        server: &McpServerConfig,
        user_id: &str,
        access_token: &str,
    ) -> String {
        manager
            .install(
                SERVER_NAME,
                Some(&server.url),
                Some(ExtensionKind::McpServer),
                user_id,
            )
            .await
            .expect("install shared MCP server");

        secrets
            .create(
                user_id,
                CreateSecretParams::new(server.token_secret_name(), access_token)
                    .with_provider(SERVER_NAME.to_string()),
            )
            .await
            .expect("store user-specific MCP token");

        let activated = manager
            .activate(SERVER_NAME, user_id)
            .await
            .expect("activate shared MCP server");
        activated
            .tools_loaded
            .into_iter()
            .find(|tool| tool.contains("mock_search"))
            .expect("mock_search tool should be registered")
    }

    #[tokio::test]
    async fn same_mcp_tool_execution_uses_runtime_users_token() {
        let mock_server = start_mock_mcp_server(vec![MockToolResponse {
            name: "mock_search".to_string(),
            content: serde_json::json!({"ok": true}),
        }])
        .await;
        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            None,
            None,
            ext_dirs.path().join("tools"),
            ext_dirs.path().join("channels"),
            None,
            "owner".to_string(),
            Some(db),
            Vec::new(),
        );
        let server = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());

        let tool_name =
            activate_for_user(&manager, &secrets, &server, USER_A, "token-user-a").await;
        let tool_name_b =
            activate_for_user(&manager, &secrets, &server, USER_B, "token-user-b").await;
        assert_eq!(tool_name_b, tool_name);

        let tool = tool_registry
            .get(&tool_name)
            .await
            .expect("registered shared MCP tool");

        mock_server.clear_recorded_requests();
        tool.execute(
            serde_json::json!({"query": "alpha"}),
            &JobContext::with_user(USER_A, "user a job", "run as user a"),
        )
        .await
        .expect("user-a MCP tool execution");

        let user_a_requests = mock_server.recorded_requests();
        assert!(
            user_a_requests.iter().any(|req| req.method == "tools/call"),
            "expected a tools/call request, got {user_a_requests:?}"
        );
        assert!(
            user_a_requests
                .iter()
                .all(|req| req.authorization.as_deref() == Some("Bearer token-user-a")),
            "all MCP requests for user-a should use user-a's token: {user_a_requests:?}"
        );

        mock_server.clear_recorded_requests();
        tool.execute(
            serde_json::json!({"query": "beta"}),
            &JobContext::with_user(USER_B, "user b job", "run as user b"),
        )
        .await
        .expect("user-b MCP tool execution");

        let user_b_requests = mock_server.recorded_requests();
        assert!(
            user_b_requests.iter().any(|req| req.method == "tools/call"),
            "expected a tools/call request, got {user_b_requests:?}"
        );
        assert!(
            user_b_requests
                .iter()
                .all(|req| req.authorization.as_deref() == Some("Bearer token-user-b")),
            "all MCP requests for user-b should use user-b's token: {user_b_requests:?}"
        );

        mock_server.shutdown().await;
    }

    /// Regression for the cross-tenant session-ID collision found in review
    /// of the `McpClientStore` PR. An MCP server issues a fresh
    /// `Mcp-Session-Id` on every `initialize` handshake; if the session
    /// manager were keyed on server name alone, user-B's activation would
    /// overwrite user-A's slot and user-A's next `tools/call` would echo
    /// user-B's session id back — potential cross-tenant access to
    /// server-side session state.
    ///
    /// This test drives both users end-to-end (activate → `tools/call` →
    /// inspect what the mock actually received) and asserts that:
    /// - The two users receive **distinct** `Mcp-Session-Id` values.
    /// - Each user's `tools/call` request echoes their **own** session id,
    ///   never the other user's.
    #[tokio::test]
    async fn session_id_is_partitioned_per_user_on_shared_mcp_server() {
        let mock_server = start_mock_mcp_server(vec![MockToolResponse {
            name: "mock_search".to_string(),
            content: serde_json::json!({"ok": true}),
        }])
        .await;
        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            None,
            None,
            ext_dirs.path().join("tools"),
            ext_dirs.path().join("channels"),
            None,
            "owner".to_string(),
            Some(db),
            Vec::new(),
        );
        let server = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());

        let tool_name =
            activate_for_user(&manager, &secrets, &server, USER_A, "token-user-a").await;
        activate_for_user(&manager, &secrets, &server, USER_B, "token-user-b").await;

        // Capture the initialize responses — each handshake should have
        // stamped a distinct session id via the mock's counter.
        let init_requests: Vec<_> = mock_server
            .recorded_requests()
            .into_iter()
            .filter(|r| r.method == "initialize")
            .collect();
        assert!(
            init_requests.len() >= 2,
            "expected at least two initialize handshakes (one per user), got {init_requests:?}"
        );
        let user_a_session_id = "mock-session-1".to_string();
        let user_b_session_id = "mock-session-2".to_string();

        // Now drive a tools/call for each user and verify the session id
        // they echo back is their OWN. Under the pre-fix bug both users
        // would echo `mock-session-2` (whichever user activated last).
        let tool = tool_registry
            .get(&tool_name)
            .await
            .expect("registered shared MCP tool");

        mock_server.clear_recorded_requests();
        tool.execute(
            serde_json::json!({"query": "alpha"}),
            &JobContext::with_user(USER_A, "user a job", "run as user a"),
        )
        .await
        .expect("user-a MCP tool execution");

        let user_a_tool_calls: Vec<_> = mock_server
            .recorded_requests()
            .into_iter()
            .filter(|r| r.method == "tools/call")
            .collect();
        assert!(
            user_a_tool_calls
                .iter()
                .all(|r| r.session_id.as_deref() == Some(user_a_session_id.as_str())),
            "user-a's tools/call must echo user-a's session id ({user_a_session_id}); got {user_a_tool_calls:?}"
        );

        mock_server.clear_recorded_requests();
        tool.execute(
            serde_json::json!({"query": "beta"}),
            &JobContext::with_user(USER_B, "user b job", "run as user b"),
        )
        .await
        .expect("user-b MCP tool execution");

        let user_b_tool_calls: Vec<_> = mock_server
            .recorded_requests()
            .into_iter()
            .filter(|r| r.method == "tools/call")
            .collect();
        assert!(
            user_b_tool_calls
                .iter()
                .all(|r| r.session_id.as_deref() == Some(user_b_session_id.as_str())),
            "user-b's tools/call must echo user-b's session id ({user_b_session_id}); got {user_b_tool_calls:?}"
        );

        mock_server.shutdown().await;
    }

    #[tokio::test]
    async fn removing_one_user_from_shared_mcp_keeps_other_user_tool_live() {
        let mock_server = start_mock_mcp_server(vec![MockToolResponse {
            name: "mock_search".to_string(),
            content: serde_json::json!({"ok": true}),
        }])
        .await;
        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            None,
            None,
            ext_dirs.path().join("tools"),
            ext_dirs.path().join("channels"),
            None,
            "owner".to_string(),
            Some(db),
            Vec::new(),
        );
        let server = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());

        let tool_name =
            activate_for_user(&manager, &secrets, &server, USER_A, "token-user-a").await;
        activate_for_user(&manager, &secrets, &server, USER_B, "token-user-b").await;

        manager
            .remove(SERVER_NAME, USER_A)
            .await
            .expect("remove shared MCP server for user-a");

        assert!(
            tool_registry.has(&tool_name).await,
            "removing one user must not unregister the shared MCP tool while another user is still active"
        );

        let tool = tool_registry
            .get(&tool_name)
            .await
            .expect("shared MCP tool should remain registered for user-b");
        mock_server.clear_recorded_requests();
        tool.execute(
            serde_json::json!({"query": "still-live"}),
            &JobContext::with_user(USER_B, "user b job", "run as user b"),
        )
        .await
        .expect("user-b MCP tool execution after user-a removal");

        let requests = mock_server.recorded_requests();
        assert!(
            requests.iter().any(|req| req.method == "tools/call"),
            "expected a tools/call request, got {requests:?}"
        );
        assert!(
            requests
                .iter()
                .all(|req| req.authorization.as_deref() == Some("Bearer token-user-b")),
            "remaining MCP requests should stay bound to user-b: {requests:?}"
        );

        manager
            .remove(SERVER_NAME, USER_B)
            .await
            .expect("remove shared MCP server for user-b");
        assert!(
            !tool_registry.has(&tool_name).await,
            "removing the last active user should unregister shared MCP tools"
        );

        mock_server.shutdown().await;
    }
}
