mod support;

use support::{MockServer, ScriptedResponse};

#[tokio::test]
async fn llm_providers_parses_snapshot() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/llm/providers",
        ScriptedResponse::ok(serde_json::json!({
            "providers": [{
                "id": "openai",
                "description": "OpenAI",
                "adapter": "open_ai_completions",
                "default_model": "gpt-5",
                "base_url": null,
                "builtin": true,
                "active": true,
                "active_model": "gpt-5",
                "api_key_required": true,
                "accepts_api_key": true,
                "api_key_set": true,
                "can_list_models": true
            }],
            "active": {"provider_id": "openai", "model": "gpt-5"}
        })),
    );

    let client = server.client();
    let snapshot = client.llm_providers().await.expect("llm providers");
    assert_eq!(snapshot.providers.len(), 1);
    assert_eq!(snapshot.providers[0].id, "openai");
    assert_eq!(
        snapshot.active.as_ref().map(|a| a.provider_id.as_str()),
        Some("openai")
    );
}

#[tokio::test]
async fn llm_test_connection_sends_adapter_and_provider_id() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/llm/test-connection",
        ScriptedResponse::ok(serde_json::json!({"ok": true, "message": "connected"})),
    );

    let client = server.client();
    let result = client
        .llm_test_connection("openai", "open_ai_completions", None)
        .await
        .expect("test connection");
    assert!(result.ok);

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body["adapter"], "open_ai_completions");
    assert_eq!(body["provider_id"], "openai");
    assert!(body.get("base_url").is_none());
}

#[tokio::test]
async fn llm_list_models_sends_adapter_and_base_url() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/llm/list-models",
        ScriptedResponse::ok(
            serde_json::json!({"ok": true, "models": ["gpt-5", "gpt-5-mini"], "message": ""}),
        ),
    );

    let client = server.client();
    let result = client
        .llm_list_models(
            "custom",
            "open_ai_completions",
            Some("https://example.test"),
        )
        .await
        .expect("list models");
    assert_eq!(result.models, vec!["gpt-5", "gpt-5-mini"]);

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(body["base_url"], "https://example.test");
    assert_eq!(body["provider_id"], "custom");
}

#[tokio::test]
async fn llm_set_active_sends_provider_and_model() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/llm/active",
        ScriptedResponse::ok(serde_json::json!({"providers": [], "active": {"provider_id": "openai", "model": "gpt-5"}})),
    );

    let client = server.client();
    client
        .llm_set_active("openai", "gpt-5")
        .await
        .expect("set active");

    let body = server.requests()[0].body.clone().expect("body");
    assert_eq!(
        body,
        serde_json::json!({"provider_id": "openai", "model": "gpt-5"})
    );
}
