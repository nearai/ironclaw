//! Integration tests for assistant-thread bootstrap and cookie-based auth.
//!
//! Verifies:
//! - Newly provisioned users start with one persisted assistant greeting
//! - Listing /api/chat/threads does not duplicate that greeting
//! - Concurrent requests don't create duplicate assistant greetings
//! - Multiple users each get their own assistant thread and greeting
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
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(db),
            settings_store: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "test-owner".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
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

    /// Helper: call /api/chat/threads and return the JSON response.
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

    /// Helper: get messages for a conversation via /api/chat/history.
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

    // ── Tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fresh_user_gets_single_initial_assistant_greeting() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice")]);
        let addr = start_test_server(db, auth).await;
        let c = client();

        // First call should load the already-provisioned assistant thread.
        let threads1 = get_threads(&c, addr, ALICE_TOKEN).await;
        let assistant1 = threads1["assistant_thread"]
            .as_object()
            .expect("assistant thread");
        let thread_id = assistant1["id"].as_str().expect("thread id");

        // A fresh provisioned user should have exactly one greeting turn.
        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns array");
        assert_eq!(
            turns.len(),
            1,
            "fresh assistant thread should have one greeting"
        );
        assert_eq!(turns[0]["response"].as_str(), Some(GREETING_SEED));

        // Second call should remain a pure read.
        let _threads2 = get_threads(&c, addr, ALICE_TOKEN).await;
        let history2 = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns2 = history2["turns"].as_array().expect("turns array");
        assert_eq!(
            turns2.len(),
            1,
            "second call should not duplicate the greeting"
        );
        assert_eq!(turns2[0]["response"].as_str(), Some(GREETING_SEED));
    }

    #[tokio::test]
    async fn test_threads_listing_does_not_duplicate_greeting_on_rapid_calls() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-rapid").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-rapid")]);
        let addr = start_test_server(db, auth).await;
        let c = client();

        // Fire 5 concurrent requests.
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

        // Check that the assistant thread still has exactly the original greeting.
        let threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let thread_id = threads["assistant_thread"]["id"]
            .as_str()
            .expect("thread id");
        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns");
        assert_eq!(
            turns.len(),
            1,
            "concurrent calls should not duplicate the assistant greeting"
        );
        assert_eq!(turns[0]["response"].as_str(), Some(GREETING_SEED));
    }

    #[tokio::test]
    async fn test_each_user_gets_own_single_assistant_greeting() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-multi").await;
        create_user(&db, "bob-multi").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-multi"), (BOB_TOKEN, "bob-multi")]);
        let addr = start_test_server(db, auth).await;
        let c = client();

        // Alice's first request.
        let alice_threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let alice_id = alice_threads["assistant_thread"]["id"]
            .as_str()
            .expect("alice thread id");

        // Bob's first request.
        let bob_threads = get_threads(&c, addr, BOB_TOKEN).await;
        let bob_id = bob_threads["assistant_thread"]["id"]
            .as_str()
            .expect("bob thread id");

        // Different thread IDs.
        assert_ne!(
            alice_id, bob_id,
            "each user should have their own assistant thread"
        );

        // Both threads have the single greeting created at provisioning time.
        let alice_history = get_history(&c, addr, ALICE_TOKEN, alice_id).await;
        let bob_history = get_history(&c, addr, BOB_TOKEN, bob_id).await;

        assert_eq!(
            alice_history["turns"].as_array().unwrap().len(),
            1,
            "alice should start with exactly one assistant greeting"
        );
        assert_eq!(
            bob_history["turns"].as_array().unwrap().len(),
            1,
            "bob should start with exactly one assistant greeting"
        );
        assert_eq!(
            alice_history["turns"][0]["response"].as_str(),
            Some(GREETING_SEED)
        );
        assert_eq!(
            bob_history["turns"][0]["response"].as_str(),
            Some(GREETING_SEED)
        );
    }

    #[tokio::test]
    async fn test_cookie_auth_works_for_threads() {
        let (db, _dir) = create_test_db().await;
        create_user(&db, "alice-cookie").await;
        let auth = auth_state(vec![(ALICE_TOKEN, "alice-cookie")]);
        let addr = start_test_server(db, auth).await;

        // Use a cookie instead of Bearer token.
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

        // Pre-populate the assistant conversation with a user message after the
        // initial greeting was provisioned.
        let conv_id = db
            .get_or_create_assistant_conversation("alice-existing", "gateway")
            .await
            .expect("create conv");
        db.add_conversation_message(conv_id, "user", "Hello!")
            .await
            .expect("add message");

        // Now call /api/chat/threads — should leave the existing conversation untouched.
        let threads = get_threads(&c, addr, ALICE_TOKEN).await;
        let thread_id = threads["assistant_thread"]["id"]
            .as_str()
            .expect("thread id");

        let history = get_history(&c, addr, ALICE_TOKEN, thread_id).await;
        let turns = history["turns"].as_array().expect("turns");
        assert_eq!(
            turns.len(),
            2,
            "should preserve the greeting and the pre-existing message"
        );
        assert_eq!(turns[0]["response"].as_str(), Some(GREETING_SEED));
        // A standalone user message with no assistant response shows as user_input.
        let user_input = turns[1]["user_input"].as_str().unwrap_or("");
        assert_eq!(user_input, "Hello!", "should be the original message");
    }
}
