//! Integration coverage for MCP prompt support (issue #2160).
//!
//! Drives the public `ExtensionManager::list_prompts_for_user` /
//! `get_prompt_for_user` surface end-to-end against the real mock MCP
//! server. The multi-tenant scoping test is the caller-level regression
//! for the nearai/ironclaw#1948 shape — a unit test on
//! `McpClient::list_prompts` alone would not catch a wrapper that silently
//! drops `user_id` between the handler and the client-store lookup.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use ironclaw::db::{Database, libsql::LibSqlBackend};
    use ironclaw::extensions::{
        ExtensionKind, ExtensionManager, ServerPromptsEntry, ServerPromptsResult,
    };
    use ironclaw::secrets::{
        CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore,
    };
    use ironclaw::tools::ToolRegistry;
    use ironclaw::tools::mcp::{McpProcessManager, McpServerConfig, McpSessionManager};
    use secrecy::SecretString;

    use crate::support::mock_mcp_server::{
        MockPromptArgDef, MockPromptDef, MockToolResponse, PromptsConfig,
        start_mock_mcp_server_with_prompts,
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

    fn build_manager(
        db: Arc<dyn Database>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        tool_registry: Arc<ToolRegistry>,
        ext_dirs: &tempfile::TempDir,
    ) -> ExtensionManager {
        ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(McpProcessManager::new()),
            secrets,
            tool_registry,
            None,
            None,
            ext_dirs.path().join("tools"),
            ext_dirs.path().join("channels"),
            None,
            "owner".to_string(),
            Some(db),
            Vec::new(),
        )
    }

    async fn activate_for_user(
        manager: &ExtensionManager,
        secrets: &Arc<dyn SecretsStore + Send + Sync>,
        server: &McpServerConfig,
        user_id: &str,
        token: &str,
    ) {
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
                CreateSecretParams::new(server.token_secret_name(), token)
                    .with_provider(SERVER_NAME.to_string()),
            )
            .await
            .expect("store user-specific MCP token");
        manager
            .activate(SERVER_NAME, user_id)
            .await
            .expect("activate shared MCP server");
    }

    /// Exercises the `/prompts` handler path end-to-end. If a wrapper ever
    /// drops `user_id` between the caller and the client-store lookup,
    /// both users' `/prompts` output would collapse to the same list —
    /// that's the nearai/ironclaw#1948 failure shape at the prompts layer.
    #[tokio::test]
    async fn list_prompts_is_scoped_to_calling_user() {
        let server_a = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts: vec![
                    MockPromptDef {
                        name: "create-page".to_string(),
                        description: Some("Create a Notion page".to_string()),
                        arguments: Some(vec![MockPromptArgDef {
                            name: "parent_id".to_string(),
                            required: true,
                        }]),
                    },
                    MockPromptDef {
                        name: "search".to_string(),
                        description: None,
                        arguments: None,
                    },
                ],
                rendered: HashMap::new(),
                instructions: Some("review the changelog before creating pages".to_string()),
            },
        )
        .await;
        let server_b = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts: vec![MockPromptDef {
                    name: "archive".to_string(),
                    description: Some("Archive an object".to_string()),
                    arguments: None,
                }],
                rendered: HashMap::new(),
                instructions: None,
            },
        )
        .await;

        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = build_manager(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            &ext_dirs,
        );

        // Install+activate with user-A's URL first, then with user-B's
        // URL — both under the same user-facing server name. This is
        // the shape that exposes per-user scoping bugs: one tool-registry
        // key, two MCP backends.
        let cfg_a = McpServerConfig::new(SERVER_NAME, server_a.mcp_url());
        activate_for_user(&manager, &secrets, &cfg_a, USER_A, "token-a").await;

        // user-B's install uses a different backend URL under the same
        // server name — this is the shape that exposes a per-user scope
        // bug (if `list_prompts_for_user` dropped `user_id`, A and B
        // would collapse to the same backend).
        let cfg_b = McpServerConfig::new(SERVER_NAME, server_b.mcp_url());
        activate_for_user(&manager, &secrets, &cfg_b, USER_B, "token-b").await;

        let a_entries = manager
            .list_prompts_for_user(USER_A)
            .await
            .expect("list_prompts_for_user A");
        let b_entries = manager
            .list_prompts_for_user(USER_B)
            .await
            .expect("list_prompts_for_user B");

        let a_names = collect_prompt_names(&a_entries);
        let b_names = collect_prompt_names(&b_entries);

        // Each user must see ONLY their own backend's prompts. Asserting
        // both halves with AND is load-bearing: an OR would pass if either
        // half happened to be correct, hiding a real leak where A sees
        // B's list.
        assert!(
            a_names.contains(&"create-page".to_string()) && a_names.contains(&"search".to_string()),
            "user-a should see their own prompts (create-page, search), got: {a_names:?}"
        );
        assert!(
            b_names.contains(&"archive".to_string()),
            "user-b should see their own prompt (archive), got: {b_names:?}"
        );
        assert!(
            !a_names.contains(&"archive".to_string()),
            "user-a leaked user-b's prompt: {a_names:?}"
        );
        assert!(
            !b_names.contains(&"create-page".to_string())
                && !b_names.contains(&"search".to_string()),
            "user-b leaked user-a's prompts: {b_names:?}"
        );
    }

    /// Required-argument validation rejects the caller-layer request
    /// BEFORE hitting the wire. This is the layer-three test for the
    /// nearai/ironclaw#1948 shape: the client's arg-check helper is
    /// called via `ExtensionManager::get_prompt_for_user`, and a
    /// regression that drops the arg check would only show up at this
    /// tier — a helper-only test on `McpClient::get_prompt` would not
    /// exercise the `ExtensionManager` wrapper.
    #[tokio::test]
    async fn get_prompt_required_args_rejected_at_manager_layer() {
        let mock_server = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts: vec![MockPromptDef {
                    name: "create-page".to_string(),
                    description: None,
                    arguments: Some(vec![
                        MockPromptArgDef {
                            name: "parent_id".to_string(),
                            required: true,
                        },
                        MockPromptArgDef {
                            name: "title".to_string(),
                            required: true,
                        },
                    ]),
                }],
                rendered: HashMap::new(),
                instructions: None,
            },
        )
        .await;

        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = build_manager(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            &ext_dirs,
        );

        let cfg = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());
        activate_for_user(&manager, &secrets, &cfg, USER_A, "token-a").await;

        // Pre-warm the prompts cache so the server_get_calls counter
        // below reflects ONLY prompts/get traffic, not the pre-flight
        // prompts/list from get_prompt's internal validation.
        manager
            .list_prompts_for_user(USER_A)
            .await
            .expect("warm prompt cache");
        let before = mock_server.recorded_prompt_get_calls().len();

        let err = manager
            .get_prompt_for_user(
                USER_A,
                SERVER_NAME,
                "create-page",
                serde_json::json!({ "title": "Q2 Review" }),
            )
            .await
            .expect_err("must fail with missing required arg");
        // Typed variant, not a substring match on a stringly-typed
        // message — the HTTP boundary dispatches on this shape.
        match &err {
            ironclaw::extensions::ExtensionError::MissingRequiredArgs { prompt, missing } => {
                assert_eq!(prompt, "create-page");
                assert!(
                    missing.iter().any(|a| a == "parent_id"),
                    "missing list must contain parent_id, got: {missing:?}"
                );
            }
            other => panic!("expected MissingRequiredArgs, got: {other:?}"),
        }
        let after = mock_server.recorded_prompt_get_calls().len();
        assert_eq!(
            after, before,
            "prompts/get must NOT have been sent; the arg-check should reject client-side"
        );
    }

    /// When every required argument is supplied, `get_prompt_for_user`
    /// forwards the entire `arguments` object to the server verbatim.
    /// This is the regression check that the map-stringify pipeline
    /// between the mention extractor / HTTP handler / ExtensionManager /
    /// McpClient doesn't silently drop keys along the way.
    #[tokio::test]
    async fn get_prompt_forwards_all_arguments_end_to_end() {
        let mut rendered = HashMap::new();
        rendered.insert("create-page".to_string(), "RENDERED: new page".to_string());

        let mock_server = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts: vec![MockPromptDef {
                    name: "create-page".to_string(),
                    description: None,
                    arguments: Some(vec![
                        MockPromptArgDef {
                            name: "parent_id".to_string(),
                            required: true,
                        },
                        MockPromptArgDef {
                            name: "title".to_string(),
                            required: true,
                        },
                    ]),
                }],
                rendered,
                instructions: None,
            },
        )
        .await;

        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = build_manager(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            &ext_dirs,
        );

        let cfg = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());
        activate_for_user(&manager, &secrets, &cfg, USER_A, "token-a").await;

        let result = manager
            .get_prompt_for_user(
                USER_A,
                SERVER_NAME,
                "create-page",
                serde_json::json!({
                    "parent_id": "abc",
                    "title": "Q2 Review",
                }),
            )
            .await
            .expect("get_prompt_for_user should succeed");
        assert_eq!(result.messages.len(), 1);

        let calls = mock_server.recorded_prompt_get_calls();
        let get_call = calls
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("create-page"))
            .expect("prompts/get was sent");
        let args = get_call
            .get("arguments")
            .and_then(|v| v.as_object())
            .expect("arguments object present");
        assert_eq!(args["parent_id"], serde_json::json!("abc"));
        assert_eq!(args["title"], serde_json::json!("Q2 Review"));
    }

    /// Regression for the unknown-prompt 500 bug: referencing a prompt
    /// name that the server doesn't advertise must surface
    /// `ExtensionError::PromptNotFound`, not a generic
    /// `ActivationFailed` that the HTTP boundary then folds into a 500.
    /// The server is active, so the correct HTTP shape is 404.
    #[tokio::test]
    async fn get_prompt_unknown_name_returns_prompt_not_found() {
        let mock_server = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts: vec![MockPromptDef {
                    name: "search".to_string(),
                    description: None,
                    arguments: None,
                }],
                rendered: HashMap::new(),
                instructions: None,
            },
        )
        .await;

        let (db, _db_dir) = test_db().await;
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = build_manager(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            &ext_dirs,
        );

        let cfg = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());
        activate_for_user(&manager, &secrets, &cfg, USER_A, "token-a").await;

        let err = manager
            .get_prompt_for_user(
                USER_A,
                SERVER_NAME,
                "does-not-exist",
                serde_json::json!({}),
            )
            .await
            .expect_err("unknown prompt name must fail");
        match &err {
            ironclaw::extensions::ExtensionError::PromptNotFound { server, prompt } => {
                assert_eq!(server, SERVER_NAME);
                assert_eq!(prompt, "does-not-exist");
            }
            other => panic!("expected PromptNotFound, got: {other:?}"),
        }
    }

    fn collect_prompt_names(entries: &[ServerPromptsEntry]) -> Vec<String> {
        let mut out = Vec::new();
        for e in entries {
            if let ServerPromptsResult::Ok { prompts } = &e.result {
                for p in prompts {
                    out.push(p.name.clone());
                }
            }
        }
        out
    }

    // ── HTTP handler coverage (/api/prompts, /api/prompts/get) ────────────
    //
    // Drives the feature slice end-to-end: `start_server` → auth middleware
    // → handler → `ExtensionManager` → real mock MCP server. This catches
    // wire-contract regressions (status codes, error-body leak boundaries,
    // argument forwarding through the HTTP layer) that the manager-layer
    // tests above can't observe.

    use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
    use ironclaw::channels::web::test_helpers::TestGatewayBuilder;

    const API_TOKEN: &str = "tok-test";

    fn single_user_auth(user_id: &str) -> MultiAuthState {
        let mut tokens = HashMap::new();
        tokens.insert(
            API_TOKEN.to_string(),
            UserIdentity {
                user_id: user_id.to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            },
        );
        MultiAuthState::multi(tokens)
    }

    /// Shared setup: spin up a mock MCP server, install + activate it for
    /// `user_id`, and start a real gateway with the resulting
    /// `ExtensionManager` wired in. Returns the bound address so tests can
    /// drive HTTP requests against real routes.
    async fn start_gateway_with_activated_server(
        user_id: &str,
        prompts: Vec<MockPromptDef>,
        rendered: HashMap<String, String>,
    ) -> (
        std::net::SocketAddr,
        Arc<ExtensionManager>,
        crate::support::mock_mcp_server::MockMcpServer,
    ) {
        let mock_server = start_mock_mcp_server_with_prompts(
            vec![MockToolResponse {
                name: "noop".to_string(),
                content: serde_json::json!({"ok": true}),
            }],
            PromptsConfig {
                prompts,
                rendered,
                instructions: None,
            },
        )
        .await;

        let (db, _db_dir) = test_db().await;
        // Leak the tempdir so the DB outlives the test; the OS will clean
        // up /tmp on test exit. Simpler than plumbing lifetimes through.
        Box::leak(Box::new(_db_dir));
        let ext_dirs = tempfile::tempdir().expect("extension tempdir");
        let ext_dirs_ref = Box::leak(Box::new(ext_dirs));
        let secrets = test_secrets_store();
        let tool_registry = Arc::new(ToolRegistry::new());
        let manager = Arc::new(build_manager(
            Arc::clone(&db),
            Arc::clone(&secrets),
            Arc::clone(&tool_registry),
            ext_dirs_ref,
        ));

        let cfg = McpServerConfig::new(SERVER_NAME, mock_server.mcp_url());
        activate_for_user(&manager, &secrets, &cfg, user_id, "token-x").await;

        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(64);
        let (addr, _state) = TestGatewayBuilder::new()
            .msg_tx(agent_tx)
            .extension_manager(Arc::clone(&manager))
            .start_multi(single_user_auth(user_id))
            .await
            .expect("start gateway");

        (addr, manager, mock_server)
    }

    /// `GET /api/prompts` returns the caller's active servers' prompts as
    /// a JSON envelope. Confirms the wire shape the web UI consumes.
    #[tokio::test]
    async fn http_list_prompts_returns_callers_active_servers() {
        let (addr, _mgr, _mock) = start_gateway_with_activated_server(
            USER_A,
            vec![
                MockPromptDef {
                    name: "create-page".to_string(),
                    description: Some("Create a Notion page".to_string()),
                    arguments: None,
                },
                MockPromptDef {
                    name: "search".to_string(),
                    description: None,
                    arguments: None,
                },
            ],
            HashMap::new(),
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/api/prompts", addr))
            .header("Authorization", format!("Bearer {}", API_TOKEN))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("json");
        let servers = body
            .get("servers")
            .and_then(|v| v.as_array())
            .expect("servers array");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["server"], SERVER_NAME);
        let names: Vec<&str> = servers[0]["prompts"]
            .as_array()
            .expect("prompts array")
            .iter()
            .map(|p| p["name"].as_str().unwrap_or_default())
            .collect();
        assert!(names.contains(&"create-page"));
        assert!(names.contains(&"search"));
    }

    /// No auth → 401; other handlers can trust that the middleware rejects
    /// unauthenticated callers before they reach the extension manager.
    #[tokio::test]
    async fn http_list_prompts_rejects_unauthenticated() {
        let (addr, _mgr, _mock) = start_gateway_with_activated_server(
            USER_A,
            vec![MockPromptDef {
                name: "search".to_string(),
                description: None,
                arguments: None,
            }],
            HashMap::new(),
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/api/prompts", addr))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 401);
    }

    /// `POST /api/prompts/get` with all required args succeeds and forwards
    /// the arguments map to the MCP server verbatim. This is the wire-level
    /// regression for the "HTTP arg map dropped keys" bug class.
    #[tokio::test]
    async fn http_get_prompt_forwards_arguments() {
        let mut rendered = HashMap::new();
        rendered.insert("create-page".to_string(), "RENDERED".to_string());

        let (addr, _mgr, mock) = start_gateway_with_activated_server(
            USER_A,
            vec![MockPromptDef {
                name: "create-page".to_string(),
                description: None,
                arguments: Some(vec![
                    MockPromptArgDef {
                        name: "parent_id".to_string(),
                        required: true,
                    },
                    MockPromptArgDef {
                        name: "title".to_string(),
                        required: true,
                    },
                ]),
            }],
            rendered,
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{}/api/prompts/get", addr))
            .header("Authorization", format!("Bearer {}", API_TOKEN))
            .json(&serde_json::json!({
                "server": SERVER_NAME,
                "name": "create-page",
                "arguments": {
                    "parent_id": "abc",
                    "title": "Q2 Review",
                }
            }))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("json");
        assert_eq!(body["server"], SERVER_NAME);
        assert_eq!(body["name"], "create-page");
        assert!(body["result"]["messages"].as_array().is_some());

        // The wire body carried both keys and the server received both.
        let calls = mock.recorded_prompt_get_calls();
        let call = calls
            .iter()
            .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("create-page"))
            .expect("prompts/get call recorded");
        let args = call
            .get("arguments")
            .and_then(|v| v.as_object())
            .expect("arguments object");
        assert_eq!(args["parent_id"], serde_json::json!("abc"));
        assert_eq!(args["title"], serde_json::json!("Q2 Review"));
    }

    /// A missing required argument produces a 400 at the HTTP layer. The
    /// body MUST name the missing arg so UI can render a useful error.
    /// This is the HTTP-tier counterpart of the manager-level test above —
    /// a regression that bubbled the check up as a generic 500 would only
    /// trip at this tier.
    #[tokio::test]
    async fn http_get_prompt_missing_required_arg_returns_400() {
        let (addr, _mgr, _mock) = start_gateway_with_activated_server(
            USER_A,
            vec![MockPromptDef {
                name: "create-page".to_string(),
                description: None,
                arguments: Some(vec![
                    MockPromptArgDef {
                        name: "parent_id".to_string(),
                        required: true,
                    },
                    MockPromptArgDef {
                        name: "title".to_string(),
                        required: true,
                    },
                ]),
            }],
            HashMap::new(),
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{}/api/prompts/get", addr))
            .header("Authorization", format!("Bearer {}", API_TOKEN))
            .json(&serde_json::json!({
                "server": SERVER_NAME,
                "name": "create-page",
                "arguments": { "title": "Q2 Review" }
            }))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 400);
        let body = resp.text().await.expect("body");
        assert!(
            body.contains("parent_id"),
            "400 body should name the missing arg, got: {body}"
        );
    }

    /// Referencing a server the caller has not activated yields 404, not
    /// 500. Covers the `NotActive` branch of `get_prompt_error_to_status`.
    #[tokio::test]
    async fn http_get_prompt_inactive_server_returns_404() {
        let (addr, _mgr, _mock) = start_gateway_with_activated_server(
            USER_A,
            vec![MockPromptDef {
                name: "search".to_string(),
                description: None,
                arguments: None,
            }],
            HashMap::new(),
        )
        .await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://{}/api/prompts/get", addr))
            .header("Authorization", format!("Bearer {}", API_TOKEN))
            .json(&serde_json::json!({
                "server": "never_activated",
                "name": "search",
                "arguments": {}
            }))
            .send()
            .await
            .expect("send");
        assert_eq!(resp.status(), 404);
    }
}
