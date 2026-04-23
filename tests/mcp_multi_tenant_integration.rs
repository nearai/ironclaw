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

    use crate::support::mock_mcp_server::{
        MockToolResponse, MockToolSpec, start_mock_mcp_server, start_mock_mcp_server_with_specs,
    };

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

    /// Regression for the activate-vs-remove TOCTOU flagged in review of
    /// the `McpClientStore` PR. Before the per-server lifecycle lock:
    ///
    /// - user A's `remove("notion")` saw "no users left", started
    ///   `tool_registry.unregister`,
    /// - user B's `activate("notion")` ran concurrently, inserted a
    ///   client and re-registered wrappers,
    /// - user A's unregister loop then deleted user B's freshly
    ///   registered wrappers — end state: B's client present in store,
    ///   B's tool wrappers missing from registry. Any of B's tool calls
    ///   would then fail with "tool not found".
    ///
    /// The invariant we assert: after a concurrent remove-and-activate
    /// settles, either the server is wholly torn down (no client, no
    /// wrappers) or it is wholly alive (client present, wrappers
    /// registered) — never half-and-half. We don't try to reproduce the
    /// timing window (non-deterministic); instead we run the scenario
    /// enough times to give the scheduler many chances to interleave and
    /// assert the invariant every iteration.
    #[tokio::test]
    async fn concurrent_activate_and_remove_preserve_registry_invariant() {
        const ITERATIONS: usize = 50;

        let mock_server = start_mock_mcp_server(vec![MockToolResponse {
            name: "mock_search".to_string(),
            content: serde_json::json!({"ok": true}),
        }])
        .await;
        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = Arc::new(ExtensionManager::new(
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
        ));
        let server = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());

        let tool_name =
            activate_for_user(&manager, &secrets, &server, USER_A, "token-user-a").await;

        for iteration in 0..ITERATIONS {
            // Seed user B's token so the interleaved activate can
            // succeed. The activation call is idempotent for an already-
            // active user; we don't care about the ordering, only the
            // final consistency.
            secrets
                .create(
                    USER_B,
                    CreateSecretParams::new(server.token_secret_name(), "token-user-b")
                        .with_provider(SERVER_NAME.to_string()),
                )
                .await
                .ok();
            manager
                .install(
                    SERVER_NAME,
                    Some(&server.url),
                    Some(ExtensionKind::McpServer),
                    USER_B,
                )
                .await
                .ok();

            let manager_a = Arc::clone(&manager);
            let manager_b = Arc::clone(&manager);
            let remove_task =
                tokio::spawn(async move { manager_a.remove(SERVER_NAME, USER_A).await });
            let activate_task =
                tokio::spawn(async move { manager_b.activate(SERVER_NAME, USER_B).await });

            let _ = remove_task.await.expect("remove task join");
            let _ = activate_task.await.expect("activate task join");

            // Invariant: the registry state matches the store state.
            // If user B's client made it into the store, B's tool
            // wrapper must also be in the registry — otherwise the
            // next tool dispatch from user B would fail spuriously.
            let b_listed = manager
                .list(Some(ExtensionKind::McpServer), false, USER_B)
                .await
                .expect("list for user-b");
            let b_client_present = b_listed
                .iter()
                .any(|ext| ext.name == SERVER_NAME && ext.active);
            let wrapper_present = tool_registry.has(&tool_name).await;

            if b_client_present {
                assert!(
                    wrapper_present,
                    "iteration {iteration}: user-b has a live client but the \
                     shared MCP tool wrapper is missing from the registry — \
                     concurrent remove/activate torn down half-state"
                );
            }

            // Reset back to "user-a active, user-b inactive" for the
            // next iteration so every loop exercises the same shape.
            manager.remove(SERVER_NAME, USER_B).await.ok();
            let a_listed = manager
                .list(Some(ExtensionKind::McpServer), false, USER_A)
                .await
                .expect("list for user-a");
            let a_still_active = a_listed
                .iter()
                .any(|ext| ext.name == SERVER_NAME && ext.active);
            if !a_still_active {
                activate_for_user(&manager, &secrets, &server, USER_A, "token-user-a").await;
            }
        }

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

    /// Regression for the reviewer's concern that MCP tool registration
    /// was still coarse-grained after the per-user client store landed.
    /// The `ToolRegistry` is keyed by tool name only, so if user A
    /// activates `SERVER_NAME` against backend X with one tool surface
    /// and user B activates the same `SERVER_NAME` against backend Y
    /// with a DIFFERENT surface, user B's `list_tools()` result would
    /// silently shadow user A's in the global registry.
    ///
    /// The fix is to reject user B's activation when the surface
    /// fingerprint disagrees with any other user's active entry for
    /// the same `server_name`. This test drives `ExtensionManager`
    /// activation end-to-end for both users and asserts:
    /// - User A's activation succeeds.
    /// - User B's activation fails with a clear ActivationFailed
    ///   explaining the surface conflict.
    /// - After the rejection, the registry still contains user A's
    ///   wrappers (unshadowed), and user A can still dispatch.
    #[tokio::test]
    async fn activate_rejects_divergent_tool_surface_on_shared_server_name() {
        let mock_server_a = start_mock_mcp_server(vec![MockToolResponse {
            name: "mock_search".to_string(),
            content: serde_json::json!({"ok": true}),
        }])
        .await;
        let mock_server_b = start_mock_mcp_server(vec![MockToolResponse {
            name: "different_tool".to_string(),
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
        let server_a = McpServerConfig::new(SERVER_NAME, mock_server_a.mcp_url());
        let server_b = McpServerConfig::new(SERVER_NAME, mock_server_b.mcp_url());

        let tool_name_a =
            activate_for_user(&manager, &secrets, &server_a, USER_A, "token-user-a").await;
        assert!(
            tool_registry.get(&tool_name_a).await.is_some(),
            "user-a's wrapper must be registered after successful activation",
        );

        // User B attempts to install + activate the SAME server name
        // pointing at a backend with a different tool surface.
        manager
            .install(
                SERVER_NAME,
                Some(&server_b.url),
                Some(ExtensionKind::McpServer),
                USER_B,
            )
            .await
            .expect("install (distinct url) for user-b should succeed — install is per-user");

        secrets
            .create(
                USER_B,
                CreateSecretParams::new(server_b.token_secret_name(), "token-user-b")
                    .with_provider(SERVER_NAME.to_string()),
            )
            .await
            .expect("store user-b token");

        let activation = manager.activate(SERVER_NAME, USER_B).await;
        let err = activation
            .expect_err("user-b activation with a divergent tool surface must be rejected");
        let message = format!("{err:?}");
        assert!(
            message.contains("different tool surface") || message.contains("tool surface"),
            "rejection message should explain the surface conflict, got: {message}"
        );

        // User A's wrappers must still be live and dispatchable — the
        // rejection must not have unregistered or shadowed them.
        assert!(
            tool_registry.get(&tool_name_a).await.is_some(),
            "rejecting user-b must leave user-a's wrapper intact in the registry",
        );
        assert!(
            tool_registry.get("different_tool").await.is_none(),
            "user-b's divergent tool name must NOT have leaked into the registry",
        );

        mock_server_a.shutdown().await;
        mock_server_b.shutdown().await;
    }

    /// Regression: the tool-surface conflict check must reject a
    /// cross-user activation that agrees on name / description /
    /// schema but disagrees on MCP annotations. `destructive_hint`
    /// drives `McpTool::requires_approval`, and `ToolRegistry` holds
    /// a SINGLE globally-registered wrapper per tool name — so if
    /// two tenants' backends advertised the same tool with different
    /// annotation hints, one user's approval policy would silently
    /// leak into the other's dispatches. The fingerprint must treat
    /// annotation-only divergence as a conflict.
    #[tokio::test]
    async fn activate_rejects_divergent_annotations_on_shared_server_name() {
        // Same name + description + schema; only `destructiveHint`
        // differs between the two backends.
        let shared_name = "mock_search";
        let shared_description = "Mock tool: mock_search".to_string();
        let shared_schema = serde_json::json!({"type": "object", "properties": {}});
        let shared_content = serde_json::json!({"ok": true});

        let mock_server_a = start_mock_mcp_server_with_specs(vec![MockToolSpec {
            name: shared_name.to_string(),
            description: shared_description.clone(),
            input_schema: shared_schema.clone(),
            annotations: Some(serde_json::json!({"destructiveHint": false})),
            content: shared_content.clone(),
        }])
        .await;
        let mock_server_b = start_mock_mcp_server_with_specs(vec![MockToolSpec {
            name: shared_name.to_string(),
            description: shared_description,
            input_schema: shared_schema,
            annotations: Some(serde_json::json!({"destructiveHint": true})),
            content: shared_content,
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

        let server_a = McpServerConfig::new(SERVER_NAME, mock_server_a.mcp_url());
        let server_b = McpServerConfig::new(SERVER_NAME, mock_server_b.mcp_url());

        // User A activates with destructive_hint=false.
        let tool_name_a =
            activate_for_user(&manager, &secrets, &server_a, USER_A, "token-user-a").await;
        assert!(
            tool_registry.get(&tool_name_a).await.is_some(),
            "user-a's wrapper must be live after the first activation",
        );

        // User B attempts activation against a backend whose only
        // divergence is `destructiveHint=true`.
        manager
            .install(
                SERVER_NAME,
                Some(&server_b.url),
                Some(ExtensionKind::McpServer),
                USER_B,
            )
            .await
            .expect("install (distinct url) for user-b should succeed");
        secrets
            .create(
                USER_B,
                CreateSecretParams::new(server_b.token_secret_name(), "token-user-b")
                    .with_provider(SERVER_NAME.to_string()),
            )
            .await
            .expect("store user-b token");

        let activation = manager.activate(SERVER_NAME, USER_B).await;
        let err = activation
            .expect_err("user-b activation must be rejected when only the MCP annotations diverge");
        let message = format!("{err:?}");
        assert!(
            message.contains("tool surface"),
            "rejection message should reference the surface conflict, got: {message}"
        );

        // User A's wrapper and approval policy must be intact.
        assert!(
            tool_registry.get(&tool_name_a).await.is_some(),
            "rejection must leave user-a's wrapper in the registry",
        );

        mock_server_a.shutdown().await;
        mock_server_b.shutdown().await;
    }
}
