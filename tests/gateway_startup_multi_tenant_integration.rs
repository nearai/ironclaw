//! Startup-level regression test for config-driven gateway multi-tenant mode.
//!
//! Verifies the real gateway construction path derives `multi_tenant_mode`
//! from `Config::is_multi_tenant_deployment()` even when the database is
//! freshly migrated and contains no tenant users yet.

#[cfg(feature = "libsql")]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use ironclaw::channels::web::GatewayChannel;
    use ironclaw::channels::web::auth::MultiAuthState;
    use ironclaw::channels::web::platform::router::start_server;
    use ironclaw::config::{Config, GatewayConfig};
    use ironclaw::db::Database;

    async fn create_test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        use ironclaw::db::libsql::LibSqlBackend;

        let dir = tempfile::tempdir().expect("temp db dir");
        let path = dir.path().join("gateway-startup-test.db");
        let backend = LibSqlBackend::new_local(&path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");
        (Arc::new(backend) as Arc<dyn Database>, dir)
    }

    #[tokio::test]
    async fn gateway_startup_honors_config_multi_tenant_mode_with_empty_db() {
        let (db, _dir) = create_test_db().await;
        let skills_dir = tempfile::tempdir().expect("skills dir");
        let installed_skills_dir = tempfile::tempdir().expect("installed skills dir");

        let mut config = Config::for_testing(
            PathBuf::from("ignored.db"),
            skills_dir.path().to_path_buf(),
            installed_skills_dir.path().to_path_buf(),
        );
        config.owner_id = "startup-owner".to_string();
        config.agent.multi_tenant = true;
        config.channels.gateway = Some(GatewayConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            auth_token: Some("startup-token".to_string()),
            max_connections: 16,
            broadcast_buffer: 64,
            workspace_read_scopes: Vec::new(),
            memory_layers: Vec::new(),
            oidc: None,
        });

        let gateway_config = config
            .channels
            .gateway
            .clone()
            .expect("gateway config missing");
        let gateway = GatewayChannel::new(gateway_config, config.owner_id.clone())
            .with_db_backing_from_config(&config, Arc::clone(&db), None)
            .with_store(Arc::clone(&db));

        let auth =
            MultiAuthState::single(gateway.auth_token().to_string(), config.owner_id.clone());
        let addr = start_server(
            "127.0.0.1:0".parse().expect("localhost addr"),
            gateway.state().clone(),
            auth.into(),
        )
        .await
        .expect("start gateway server");

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{addr}/api/admin/tool-policy"))
            .bearer_auth(gateway.auth_token())
            .send()
            .await
            .expect("request admin tool policy");

        assert_eq!(
            response.status(),
            reqwest::StatusCode::OK,
            "config-driven multi_tenant_mode should expose multi-tenant-only endpoints even before any users exist"
        );

        let body: serde_json::Value = response
            .json()
            .await
            .expect("parse admin tool policy response");
        assert_eq!(
            body,
            serde_json::json!({
                "disabled_tools": [],
                "user_disabled_tools": {}
            })
        );
    }
}
