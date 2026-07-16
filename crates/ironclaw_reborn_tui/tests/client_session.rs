mod support;

use support::{MockServer, ScriptedResponse};

#[tokio::test]
async fn session_parses_tenant_and_user_ignoring_extra_fields() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/session",
        ScriptedResponse::ok(serde_json::json!({
            "tenant_id": "tenant-1",
            "user_id": "user-1",
            "capabilities": {"operator_webui_config": false},
            "features": {"reborn_projects": false, "global_auto_approve": false},
            "attachments": {"accept": [], "max_file_bytes": 0, "max_total_bytes": 0, "max_files": 0}
        })),
    );

    let client = server.client();
    let session = client.session().await.expect("session");
    assert_eq!(session.tenant_id, "tenant-1");
    assert_eq!(session.user_id, "user-1");
}

#[tokio::test]
async fn session_maps_connection_refused_to_transport_error() {
    // No mock server started — hitting an unbound port must surface as
    // ClientError::Transport, distinguishing "serve isn't up" from a 401/404.
    let client = ironclaw_reborn_tui::client::ApiClient::new(
        "http://127.0.0.1:1".to_string(),
        "token".to_string(),
    );
    let error = client
        .session()
        .await
        .expect_err("expected transport error");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::Transport(_)
    ));
}
