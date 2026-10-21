//! Exercise the built-in default through resolution, provider construction and HTTP.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::{
    ChatMessage, CompletionRequest, ProviderSelection, ToolCompletionRequest, ToolDefinition,
    build_registry_provider_config_from_resolved_provider, create_registry_provider,
    registry::ProviderRegistry, resolve_provider_config_from_selection,
};

#[tokio::test]
async fn minimax_default_and_explicit_models_reach_chat_completions() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let builtin = ProviderRegistry::load_from_path(None);
        let minimax = builtin.find("minimax").unwrap();
        assert_eq!(minimax.default_model, "MiniMax-M3.1-Flash");
        assert_eq!(
            minimax.default_base_url.as_deref(),
            Some("https://api.minimax.io/v1")
        );

        for (provider_id, selected_model, expected) in [
            ("minimax", None, "MiniMax-M3.1-Flash"),
            ("minimax", Some("MiniMax-M3"), "MiniMax-M3"),
            ("minimax", Some("MiniMax-M2.7"), "MiniMax-M2.7"),
            ("minimax", Some("custom-model"), "custom-model"),
            ("openai", Some("gpt-4o"), "gpt-4o"),
        ] {
            // Keep built-in model/protocol selection but isolate host credentials
            // and environment overrides from this local HTTP regression test.
            let mut definition = builtin.find(provider_id).unwrap().clone();
            definition.api_key_env = None;
            definition.api_key_required = false;
            definition.model_env.clear();
            definition.base_url_env = None;
            definition.extra_headers_env = None;
            let registry = ProviderRegistry::new(vec![definition]);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .unwrap();
            let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                for with_tools in [false, true] {
                    let (stream, _) = listener.accept().await.unwrap();
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    assert_eq!(line.trim(), "POST /v1/chat/completions HTTP/1.1");
                    let mut length = None;
                    loop {
                        line.clear();
                        assert!(reader.read_line(&mut line).await.unwrap() > 0);
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("content-length")
                        {
                            length = Some(value.trim().parse::<usize>().unwrap());
                        }
                    }
                    let mut bytes = vec![0; length.expect("request content length")];
                    reader.read_exact(&mut bytes).await.unwrap();
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(body["model"], expected);
                    assert!(body["messages"].is_array());
                    for key in ["reasoning", "reasoning_effort", "thinking", "input"] {
                        assert!(body.get(key).is_none(), "unexpected {key}: {body}");
                    }
                    if with_tools {
                        assert_eq!(body["tools"][0]["function"]["name"], "lookup");
                    }
                    let response = json!({
                        "id": "test", "object": "chat.completion", "created": 0,
                        "model": expected,
                        "choices": [{"index": 0, "message": {
                            "role": "assistant", "content": "ok"
                        }, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                    }).to_string();
                    reader.get_mut().write_all(format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    ).as_bytes()).await.unwrap();
                }
            });
            let resolved = resolve_provider_config_from_selection(
                ProviderSelection {
                    provider_id: provider_id.into(),
                    api_key_env: None,
                    base_url: Some(base_url),
                    model: selected_model.map(str::to_owned),
                },
                &registry,
            ).unwrap();
            let config = build_registry_provider_config_from_resolved_provider(resolved).unwrap();
            let provider = create_registry_provider(&config, 5).unwrap();
            assert_eq!(provider.model_name(), expected);
            let messages = vec![ChatMessage::user("hello")];
            assert_eq!(provider.complete(CompletionRequest::new(messages.clone()))
                .await.unwrap().content, "ok");
            provider.complete_with_tools(ToolCompletionRequest::new(messages, vec![
                ToolDefinition {
                    name: "lookup".into(),
                    description: "Look up a value".into(),
                    parameters: json!({"type": "object", "properties": {}}),
                },
            ])).await.unwrap();
            server.await.unwrap();
        }
    }).await.expect("local Chat Completions requests must complete");
}

#[test]
fn minimax_custom_catalog_default_and_regional_endpoints_are_preserved() {
    let builtin = ProviderRegistry::load_from_path(None);
    let mut custom = builtin.find("minimax").unwrap().clone();
    custom.default_model = "my-saved-model".into();
    let registry = ProviderRegistry::new(vec![builtin.find("minimax").unwrap().clone(), custom]);
    assert_eq!(
        registry.find("minimax").unwrap().default_model,
        "my-saved-model"
    );
    for url in ["https://api.minimax.io/v1", "https://api.minimaxi.com/v1"] {
        assert_eq!(crate::normalize_openai_base_url(url), url);
    }
}
