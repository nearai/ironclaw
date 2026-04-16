//! Integration tests for assistant-thread bootstrap and cookie-based auth.
//!
//! Verifies:
//! - Newly provisioned users expose the configured bootstrap assistant state
//! - An empty bootstrap seed does not surface blank assistant turns
//! - Listing /api/chat/threads does not duplicate bootstrap state
//! - Multiple users each get their own assistant thread
//! - Cookie-based session auth works for protected endpoints
//! - Pre-existing conversations are not overwritten

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;

    use ironclaw::agent::SessionManager;
    use ironclaw::channels::web::auth::{MultiAuthState, UserIdentity};
    use ironclaw::channels::web::server::{
        GatewayState, PerUserRateLimiter, RateLimiter, start_server,
    };
    use ironclaw::channels::web::sse::SseManager;
    use ironclaw::channels::web::ws::WsConnectionTracker;
    use ironclaw::db::Database;
    use ironclaw::workspace::GREETING_SEED;

    const ALICE_TOKEN: &str = "tok-alice-greeting-test";
    const BOB_TOKEN: &str = "tok-bob-greeting-test";

    async fn create_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        use ironclaw::db::libsql::LibSqlBackend;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = temp_dir.path().join("greeting_test.db");
        let backend = LibSqlBackend::new_local(&db_path)
            .await
            .expect("LibSqlBackend");
        backend.run_migrations().await.expect("migrations");
        (Arc::new(backend) as Arc<dyn Database>, temp_dir)
    }

    fn auth_state(tokens: Vec<(&str, &str)>) -> MultiAuthState {
        let mut map = std::collections::HashMap::new();
        for (token, user_id) in tokens {
            map.insert(
                token.to_string(),
                UserIdentity {
                    user_id: user_id.to_string(),
                    role: "admin".to_string(),
                    workspace_read_scopes: Vec::new(),
                },
            );
        }
        MultiAuthState::multi(map)
    }

    async fn start_test_server(
        db: Arc<dyn Database>,
        auth: MultiAuthState,
    ) -> std::net::SocketAddr {
        let (agent_tx, _agent_rx) = tokio::sync::mpsc::channel(64);
        let session_manager = Arc::new(SessionManager::new());

        let state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: Some(session_manager),
            channel_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(db),
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "test-owner".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            server_started: std::sync::atomic::AtomicBool::new(false),
            ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Default::default(),
            secrets_store: None,
            db_auth: None,
            pairing_store: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
            standby_control: None,
            runtime_overrides: Default::default(),
            channel_reconnect_notify: None,
            server_handle: tokio::sync::RwLock::new(None),
        });

        let addr: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        start_server(addr, state, auth.into())
            .await
            .expect("start server")
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    }

    async fn create_user(db: &Arc<dyn Database>, user_id: &str) {
        let now = chrono::Utc::now();
        db.create_user(&ironclaw::db::UserRecord {
            id: user_id.to_string(),
            email: Some(format!("{user_id}@example.com")),
            display_name: user_id.to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        })
        .await
        .expect("create user");
    }

    async fn get_threads(
        client: &reqwest::Client,
        addr: std::net::SocketAddr,
        token: &str,
    ) -> serde_json::Value {
        let resp = client
            .get(format!("http://{addr}/api/chat/threads"))
            .bearer_auth(token)
            .send()
            .await
            .expect("threads request");
        assert_eq!(resp.status(), 200);
        resp.json().await.expect("parse threads JSON")
    }

    async fn get_history(
        client: &reqwest::Client,
        addr: std::net::SocketAddr,
        token: &str,
        thread_id: &str,
    ) -> serde_json::Value {
        let resp = client
            .get(format!(
                "http://{addr}/api/chat/history?thread_id={thread_id}"
            ))
            .bearer_auth(token)
            .send()
            .await
            .expect("history request");
        assert_eq!(resp.status(), 200);
        resp.json().await.expect("parse history JSON")
    }

    fn bootstrap_greeting_enabled() -> bool {
        !GREETING_SEED.trim().is_empty()
    }

    fn expected_bootstrap_turn_count() -> usize {
        usize::from(bootstrap_greeting_enabled())
    }

    fn assert_bootstrap_history(turns: &[serde_json::Value], context: &str) {
        assert_eq!(turns.len(), expected_bootstrap_turn_count(), "{context}");
        if bootstrap_greeting_enabled() {
            assert_eq!(
                turns[0]["response"].as_str(),
                Some(GREETING_SEED),
                "{context}"
            );
        }
    }

    #[tokio::test]
    async fn test_fresh_user_gets_single_initial_assistant_greeting() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice").await;

        let seeded_thread_id = db
            .get_or_create_assistant_conversation("alice", "gateway")
            .await
            .expect("seeded assistant thread");
        let (seeded_messages, _) = db
            .list_conversation_messages_paginated(seeded_thread_id, None, 50)
            .await
            .expect("seeded assistant messages");
        assert_eq!(
            seeded_messages.len(),
            expected_bootstrap_turn_count(),
            "user provisioning should persist only the configured bootstrap greeting"
        );
        if bootstrap_greeting_enabled() {
            assert_eq!(seeded_messages[0].content, GREETING_SEED);
        }

        let auth = auth_state(vec![(ALICE_TOKEN, "alice")]);
        let addr = start_test_server(Arc::clone(&db), auth).await;
        let c = client();

        let threads1 = get_threads(&c, addr, ALICE_TOKEN).await;
        let assistant1 = threads1["assistant_thread"]
            .as_object()
            .expect("assistant thread");
        let thread_id = assistant1["id"].as_str().expect("thread id");
        assert_eq!(
            thread_id,
            seeded_thread_id.to_string(),
            "/api/chat/threads should return the provisioned assistant thread"
        );

        let (messages_after_threads, _) = db
            .list_conversation_messages_paginated(seeded_thread_id, None, 50)
            .await
            .expect("assistant messages after /threads");
        assert_eq!(
            messages_after_threads.len(),
            expected_bootstrap_turn_count(),
            "/api/chat/threads should not mutate bootstrap greeting state"
        );
        if bootstrap_greeting_enabled() {
            assert_eq!(messages_after_threads[0].content, GREETING_SEED);
        }

        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns array");
        assert_bootstrap_history(
            turns,
            "fresh assistant thread should mirror the configured bootstrap greeting",
        );

        let _threads2 = get_threads(&c, addr, ALICE_TOKEN).await;
        let history2 = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns2 = history2["turns"].as_array().expect("turns array");
        assert_bootstrap_history(
            turns2,
            "second call should not duplicate the bootstrap greeting",
        );
    }

    #[tokio::test]
    async fn test_threads_listing_does_not_duplicate_greeting_on_rapid_calls() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-rapid").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-rapid")]);
        let addr = start_test_server(db, auth).await;
        let c = client();

        let mut handles = Vec::new();
        for _ in 0..5 {
            let c2 = c.clone();
            let addr2 = addr;
            handles.push(tokio::spawn(async move {
                get_threads(&c2, addr2, ALICE_TOKEN).await
            }));
        }
        for h in handles {
            h.await.expect("join");
        }

        let threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let thread_id = threads["assistant_thread"]["id"]
            .as_str()
            .expect("thread id");
        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns");
        assert_bootstrap_history(
            turns,
            "concurrent calls should not duplicate the bootstrap greeting",
        );
    }

    #[tokio::test]
    async fn test_each_user_gets_own_single_assistant_greeting() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-multi").await;
        create_user(&db, "bob-multi").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-multi"), (BOB_TOKEN, "bob-multi")]);
        let addr = start_test_server(db, auth).await;
        let c = client();

        let alice_threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let alice_id = alice_threads["assistant_thread"]["id"]
            .as_str()
            .expect("alice thread id");

        let bob_threads = get_threads(&c, addr, BOB_TOKEN).await;
        let bob_id = bob_threads["assistant_thread"]["id"]
            .as_str()
            .expect("bob thread id");

        assert_ne!(
            alice_id, bob_id,
            "each user should have their own assistant thread"
        );

        let alice_history = get_history(&c, addr, ALICE_TOKEN, alice_id).await;
        let bob_history = get_history(&c, addr, BOB_TOKEN, bob_id).await;
        assert_bootstrap_history(
            alice_history["turns"].as_array().unwrap(),
            "alice should see exactly the configured bootstrap greeting state",
        );
        assert_bootstrap_history(
            bob_history["turns"].as_array().unwrap(),
            "bob should see exactly the configured bootstrap greeting state",
        );
    }

    #[tokio::test]
    async fn test_cookie_auth_works_for_threads() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-cookie").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-cookie")]);
        let addr = start_test_server(db, auth).await;

        let c = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();

        let resp = c
            .get(format!("http://{addr}/api/chat/threads"))
            .header("Cookie", format!("ironclaw_session={ALICE_TOKEN}"))
            .send()
            .await
            .expect("cookie auth request");

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.expect("parse");
        assert!(
            body["assistant_thread"].is_object(),
            "should have assistant thread via cookie auth"
        );
    }

    #[tokio::test]
    async fn test_existing_conversation_is_preserved() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-existing").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-existing")]);
        let addr = start_test_server(Arc::clone(&db), auth).await;
        let c = client();

        let conv_id = db
            .get_or_create_assistant_conversation("alice-existing", "gateway")
            .await
            .expect("create conv");
        db.add_conversation_message(conv_id, "user", "Hello!")
            .await
            .expect("add message");

        let threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let thread_id = threads["assistant_thread"]["id"]
            .as_str()
            .expect("thread id");

        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns");
        assert_eq!(
            turns.len(),
            expected_bootstrap_turn_count() + 1,
            "should preserve the bootstrap state and the pre-existing message"
        );

        let user_turn_index = expected_bootstrap_turn_count();
        if bootstrap_greeting_enabled() {
            assert_eq!(turns[0]["response"].as_str(), Some(GREETING_SEED));
        }

        let user_input = turns[user_turn_index]["user_input"].as_str().unwrap_or("");
        assert_eq!(user_input, "Hello!", "should be the original message");
    }
}
