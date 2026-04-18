use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use ironclaw_engine::Store;
use tower::ServiceExt;

use crate::bridge::{
    install_test_engine_state, reset_test_engine_state, with_test_engine_state_lock,
};
use crate::channels::web::auth::{MultiAuthState, UserIdentity, auth_middleware};
use crate::channels::web::handlers::engine::{
    engine_missions_handler, engine_missions_summary_handler,
};
use crate::channels::web::server::{
    ActiveConfigSnapshot, GatewayState, PerUserRateLimiter, RateLimiter,
};
use crate::channels::web::sse::SseManager;

#[derive(Default)]
struct MissionStore {
    projects: tokio::sync::RwLock<Vec<ironclaw_engine::Project>>,
    missions: tokio::sync::RwLock<Vec<ironclaw_engine::Mission>>,
}

#[async_trait]
impl ironclaw_engine::Store for MissionStore {
    async fn save_thread(
        &self,
        _thread: &ironclaw_engine::Thread,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_thread(
        &self,
        _id: ironclaw_engine::ThreadId,
    ) -> Result<Option<ironclaw_engine::Thread>, ironclaw_engine::EngineError> {
        Ok(None)
    }

    async fn list_threads(
        &self,
        _project_id: ironclaw_engine::ProjectId,
        _user_id: &str,
    ) -> Result<Vec<ironclaw_engine::Thread>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn update_thread_state(
        &self,
        _id: ironclaw_engine::ThreadId,
        _state: ironclaw_engine::ThreadState,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn save_step(
        &self,
        _step: &ironclaw_engine::Step,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_steps(
        &self,
        _thread_id: ironclaw_engine::ThreadId,
    ) -> Result<Vec<ironclaw_engine::Step>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn append_events(
        &self,
        _events: &[ironclaw_engine::ThreadEvent],
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_events(
        &self,
        _thread_id: ironclaw_engine::ThreadId,
    ) -> Result<Vec<ironclaw_engine::ThreadEvent>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn save_project(
        &self,
        project: &ironclaw_engine::Project,
    ) -> Result<(), ironclaw_engine::EngineError> {
        let mut projects = self.projects.write().await;
        projects.retain(|existing| existing.id != project.id);
        projects.push(project.clone());
        Ok(())
    }

    async fn load_project(
        &self,
        id: ironclaw_engine::ProjectId,
    ) -> Result<Option<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
        Ok(self
            .projects
            .read()
            .await
            .iter()
            .find(|project| project.id == id)
            .cloned())
    }

    async fn list_projects(
        &self,
        user_id: &str,
    ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
        Ok(self
            .projects
            .read()
            .await
            .iter()
            .filter(|project| project.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn list_all_projects(
        &self,
    ) -> Result<Vec<ironclaw_engine::Project>, ironclaw_engine::EngineError> {
        Ok(self.projects.read().await.clone())
    }

    async fn save_conversation(
        &self,
        _conversation: &ironclaw_engine::ConversationSurface,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_conversation(
        &self,
        _id: ironclaw_engine::ConversationId,
    ) -> Result<Option<ironclaw_engine::ConversationSurface>, ironclaw_engine::EngineError> {
        Ok(None)
    }

    async fn list_conversations(
        &self,
        _user_id: &str,
    ) -> Result<Vec<ironclaw_engine::ConversationSurface>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn save_memory_doc(
        &self,
        _doc: &ironclaw_engine::MemoryDoc,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_memory_doc(
        &self,
        _id: ironclaw_engine::DocId,
    ) -> Result<Option<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
        Ok(None)
    }

    async fn list_memory_docs(
        &self,
        _project_id: ironclaw_engine::ProjectId,
        _user_id: &str,
    ) -> Result<Vec<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn list_memory_docs_by_owner(
        &self,
        _user_id: &str,
    ) -> Result<Vec<ironclaw_engine::MemoryDoc>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn save_lease(
        &self,
        _lease: &ironclaw_engine::CapabilityLease,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn load_active_leases(
        &self,
        _thread_id: ironclaw_engine::ThreadId,
    ) -> Result<Vec<ironclaw_engine::CapabilityLease>, ironclaw_engine::EngineError> {
        Ok(vec![])
    }

    async fn revoke_lease(
        &self,
        _lease_id: ironclaw_engine::LeaseId,
        _reason: &str,
    ) -> Result<(), ironclaw_engine::EngineError> {
        Ok(())
    }

    async fn save_mission(
        &self,
        mission: &ironclaw_engine::Mission,
    ) -> Result<(), ironclaw_engine::EngineError> {
        let mut missions = self.missions.write().await;
        missions.retain(|existing| existing.id != mission.id);
        missions.push(mission.clone());
        Ok(())
    }

    async fn load_mission(
        &self,
        id: ironclaw_engine::MissionId,
    ) -> Result<Option<ironclaw_engine::Mission>, ironclaw_engine::EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .iter()
            .find(|mission| mission.id == id)
            .cloned())
    }

    async fn list_missions(
        &self,
        project_id: ironclaw_engine::ProjectId,
        user_id: &str,
    ) -> Result<Vec<ironclaw_engine::Mission>, ironclaw_engine::EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .iter()
            .filter(|mission| mission.project_id == project_id && mission.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn update_mission_status(
        &self,
        id: ironclaw_engine::MissionId,
        status: ironclaw_engine::MissionStatus,
    ) -> Result<(), ironclaw_engine::EngineError> {
        if let Some(mission) = self
            .missions
            .write()
            .await
            .iter_mut()
            .find(|mission| mission.id == id)
        {
            mission.status = status;
        }
        Ok(())
    }
}

fn auth_state() -> MultiAuthState {
    let mut tokens = HashMap::new();
    tokens.insert(
        "tok-alice".to_string(),
        UserIdentity {
            user_id: "alice".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        },
    );
    tokens.insert(
        "tok-bob".to_string(),
        UserIdentity {
            user_id: "bob".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: vec![],
        },
    );
    MultiAuthState::multi(tokens)
}

fn build_state() -> Arc<GatewayState> {
    Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: Arc::new(SseManager::new()),
        workspace: None,
        workspace_pool: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        settings_cache: None,
        job_manager: None,
        prompt_queue: None,
        owner_id: "test".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: None,
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        auth_manager: None,
        scheduler: None,
        chat_rate_limiter: PerUserRateLimiter::new(30, 60),
        oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
        webhook_rate_limiter: RateLimiter::new(10, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
        startup_time: std::time::Instant::now(),
        active_config: ActiveConfigSnapshot::default(),
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
    })
}

fn engine_missions_router() -> Router {
    let state = build_state();
    let auth = auth_state();

    Router::new()
        .route("/api/engine/missions", get(engine_missions_handler))
        .route(
            "/api/engine/missions/summary",
            get(engine_missions_summary_handler),
        )
        .layer(middleware::from_fn_with_state(
            crate::channels::web::auth::CombinedAuthState::from(auth),
            auth_middleware,
        ))
        .with_state(state)
}

async fn setup_engine_state() -> Arc<MissionStore> {
    let store = Arc::new(MissionStore::default());

    let alice_default = ironclaw_engine::Project::new("alice", "default", "Default project");
    let alice_default_id = alice_default.id;
    store.save_project(&alice_default).await.unwrap();

    let alice_research = ironclaw_engine::Project::new("alice", "research", "Research project");
    let alice_research_id = alice_research.id;
    store.save_project(&alice_research).await.unwrap();

    let bob_default = ironclaw_engine::Project::new("bob", "default", "Bob project");
    let bob_default_id = bob_default.id;
    store.save_project(&bob_default).await.unwrap();

    let mut default_mission = ironclaw_engine::Mission::new(
        alice_default_id,
        "alice",
        "default-mission",
        "watch default project",
        ironclaw_engine::MissionCadence::Manual,
    );
    default_mission.updated_at = chrono::Utc::now() - chrono::Duration::hours(1);
    store.save_mission(&default_mission).await.unwrap();

    let mut research_mission = ironclaw_engine::Mission::new(
        alice_research_id,
        "alice",
        "research-mission",
        "watch research project",
        ironclaw_engine::MissionCadence::Manual,
    );
    research_mission.status = ironclaw_engine::MissionStatus::Paused;
    research_mission.updated_at = chrono::Utc::now() - chrono::Duration::minutes(20);
    store.save_mission(&research_mission).await.unwrap();

    let mut shared_mission = ironclaw_engine::Mission::new(
        alice_research_id,
        ironclaw_engine::types::shared_owner_id(),
        "shared-mission",
        "shared learning",
        ironclaw_engine::MissionCadence::Manual,
    );
    shared_mission.status = ironclaw_engine::MissionStatus::Completed;
    shared_mission.updated_at = chrono::Utc::now() - chrono::Duration::minutes(5);
    store.save_mission(&shared_mission).await.unwrap();

    let mut bob_mission = ironclaw_engine::Mission::new(
        bob_default_id,
        "bob",
        "bob-mission",
        "watch bob project",
        ironclaw_engine::MissionCadence::Manual,
    );
    bob_mission.status = ironclaw_engine::MissionStatus::Failed;
    store.save_mission(&bob_mission).await.unwrap();

    install_test_engine_state(
        Arc::clone(&store) as Arc<dyn ironclaw_engine::Store>,
        alice_default_id,
    )
    .await;

    store
}

#[tokio::test]
async fn missions_api_lists_visible_missions_across_projects() {
    with_test_engine_state_lock(async {
        reset_test_engine_state().await;
        let _store = setup_engine_state().await;

        let app = engine_missions_router();
        let req = Request::builder()
            .uri("/api/engine/missions")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let missions = json["missions"].as_array().unwrap();

        assert_eq!(missions.len(), 3);
        assert_eq!(missions[0]["name"], "shared-mission");
        assert_eq!(missions[1]["name"], "research-mission");
        assert_eq!(missions[2]["name"], "default-mission");
        assert!(
            !missions
                .iter()
                .any(|mission| mission["name"] == "bob-mission")
        );

        reset_test_engine_state().await;
    })
    .await;
}

#[tokio::test]
async fn missions_summary_api_matches_aggregated_visible_missions() {
    with_test_engine_state_lock(async {
        reset_test_engine_state().await;
        let _store = setup_engine_state().await;

        let app = engine_missions_router();

        let alice_req = Request::builder()
            .uri("/api/engine/missions/summary")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let alice_resp = app.clone().oneshot(alice_req).await.unwrap();
        assert_eq!(alice_resp.status(), StatusCode::OK);
        let alice_body = axum::body::to_bytes(alice_resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let alice_json: serde_json::Value = serde_json::from_slice(&alice_body).unwrap();

        assert_eq!(alice_json["total"], 3);
        assert_eq!(alice_json["active"], 1);
        assert_eq!(alice_json["paused"], 1);
        assert_eq!(alice_json["completed"], 1);
        assert_eq!(alice_json["failed"], 0);

        let bob_req = Request::builder()
            .uri("/api/engine/missions/summary")
            .header("Authorization", "Bearer tok-bob")
            .body(Body::empty())
            .unwrap();
        let bob_resp = app.oneshot(bob_req).await.unwrap();
        assert_eq!(bob_resp.status(), StatusCode::OK);
        let bob_body = axum::body::to_bytes(bob_resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let bob_json: serde_json::Value = serde_json::from_slice(&bob_body).unwrap();

        assert_eq!(bob_json["total"], 1);
        assert_eq!(bob_json["active"], 0);
        assert_eq!(bob_json["paused"], 0);
        assert_eq!(bob_json["completed"], 0);
        assert_eq!(bob_json["failed"], 1);

        reset_test_engine_state().await;
    })
    .await;
}
