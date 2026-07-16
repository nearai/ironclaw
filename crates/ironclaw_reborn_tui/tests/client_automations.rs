mod support;

use support::{MockServer, ScriptedResponse};

fn automation_json(id: &str, state: &str) -> serde_json::Value {
    serde_json::json!({
        "automation_id": id,
        "name": "Daily digest",
        "source": {"type": "schedule", "cron": "0 9 * * *", "timezone": "UTC"},
        "state": state,
        "next_run_at": "2026-07-16T09:00:00Z",
        "last_run_at": "2026-07-15T09:00:00Z",
        "last_status": "ok",
        "recent_runs": [],
        "is_active": state == "active"
    })
}

#[tokio::test]
async fn list_automations_parses_summaries() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/automations",
        ScriptedResponse::ok(serde_json::json!({
            "automations": [automation_json("auto-1", "active")],
            "scheduler_enabled": true
        })),
    );

    let client = server.client();
    let automations = client.list_automations().await.expect("list automations");
    assert_eq!(automations.len(), 1);
    assert_eq!(automations[0].automation_id, "auto-1");
    assert_eq!(automations[0].state, "active");
}

#[tokio::test]
async fn pause_automation_posts_to_pause_path_and_returns_updated_summary() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/automations/auto-1/pause",
        ScriptedResponse::ok(serde_json::json!({
            "updated": true,
            "automation": automation_json("auto-1", "paused")
        })),
    );

    let client = server.client();
    let automation = client.pause_automation("auto-1").await.expect("pause");
    assert_eq!(automation.state, "paused");
}

#[tokio::test]
async fn resume_automation_posts_to_resume_path_and_returns_updated_summary() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/automations/auto-1/resume",
        ScriptedResponse::ok(serde_json::json!({
            "updated": true,
            "automation": automation_json("auto-1", "active")
        })),
    );

    let client = server.client();
    let automation = client.resume_automation("auto-1").await.expect("resume");
    assert_eq!(automation.state, "active");
}

#[tokio::test]
async fn rename_automation_sends_name_body() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/automations/auto-1",
        ScriptedResponse::ok(serde_json::json!({
            "updated": true,
            "automation": automation_json("auto-1", "active")
        })),
    );

    let client = server.client();
    client
        .rename_automation("auto-1", "Renamed digest")
        .await
        .expect("rename");

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body, serde_json::json!({"name": "Renamed digest"}));
}
