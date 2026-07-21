//! Session-specific provider loopback tests.

use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::codex_test_helpers::make_test_jwt;

fn provider_with_session_capability(api_base_url: &str) -> OpenAiCodexProvider {
    let jwt = make_test_jwt("acct_test");
    let mut provider = OpenAiCodexProvider::new("gpt-5.3-codex", api_base_url, &jwt, 30).unwrap();
    provider.responses_sessions = Some(ResponsesSessionRegistry::new());
    provider
}

fn agent_loop_metadata() -> HashMap<String, String> {
    HashMap::from([(
        "agent_loop_session_id".to_string(),
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
    )])
}

fn completed_text_sse() -> String {
    let delta = serde_json::json!({
        "type": "response.output_text.delta",
        "delta": "done",
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": "resp_after_tool",
            "status": "completed",
            "output": [{
                "id": "msg_server",
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "done",
                    "annotations": [],
                }],
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1},
        },
    });
    format!("data: {delta}\n\ndata: {completed}\n\n")
}

#[tokio::test]
async fn loopback_session_matches_normalized_function_call_before_suffix() {
    let function_call = serde_json::json!({
        "id": "fc_server",
        "type": "function_call",
        "status": "completed",
        "call_id": "call_search",
        "name": "search",
        "arguments": "{\"query\":\"rust\"}",
    });
    let added = serde_json::json!({
        "type": "response.output_item.added",
        "item": function_call.clone(),
    });
    let arguments_done = serde_json::json!({
        "type": "response.function_call_arguments.done",
        "item_id": "fc_server",
        "arguments": "{\"query\":\"rust\"}",
    });
    let done = serde_json::json!({
        "type": "response.output_item.done",
        "item": function_call.clone(),
    });
    let completed = serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": "resp_tool",
            "status": "completed",
            "output": [{
                "id": "rs_server",
                "type": "reasoning",
                "summary": [],
                "encrypted_content": "opaque",
            }, function_call],
            "usage": {"input_tokens": 1, "output_tokens": 1},
        },
    });
    let tool_sse =
        format!("data: {added}\n\ndata: {arguments_done}\n\ndata: {done}\n\ndata: {completed}\n\n");
    let (base_url, mut requests) = test_server::spawn(vec![tool_sse, completed_text_sse()]).await;
    let provider = provider_with_session_capability(&base_url);
    let tools = vec![ToolDefinition {
        name: "search".to_string(),
        description: "Search".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
        }),
    }];

    let mut first =
        ToolCompletionRequest::new(vec![ChatMessage::user("search for rust")], tools.clone());
    first.metadata = agent_loop_metadata();
    let response = provider.complete_with_tools(first).await.unwrap();
    let first_body = requests.recv().await.unwrap();
    assert!(first_body.get("previous_response_id").is_none());
    assert_eq!(
        first_body["input"],
        serde_json::Value::Array(normalized_input(&[ChatMessage::user("search for rust")]))
    );

    let tool_result = ChatMessage::tool_result("call_search", "search", "found rust");
    let mut second = ToolCompletionRequest::new(
        vec![
            ChatMessage::user("search for rust"),
            ChatMessage::assistant_with_tool_calls(None, response.tool_calls),
            tool_result.clone(),
        ],
        tools,
    );
    second.metadata = agent_loop_metadata();
    provider.complete_with_tools(second).await.unwrap();

    let second_body = requests.recv().await.unwrap();
    assert_eq!(second_body["previous_response_id"], "resp_tool");
    assert_eq!(
        second_body["input"],
        serde_json::Value::Array(normalized_input(&[tool_result]))
    );
}

#[tokio::test]
async fn account_change_clears_enabled_response_sessions() {
    let provider = provider_with_session_capability("http://127.0.0.1:1");
    let metadata = agent_loop_metadata();
    let registry = provider.responses_sessions.as_ref().unwrap();
    let session = registry.session_for_metadata(&metadata).await.unwrap();
    let first = normalized_input(&[ChatMessage::user("first")]);
    let output = [serde_json::json!({
        "id": "msg_server",
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": "answer",
            "annotations": [],
        }],
    })];
    session
        .lock()
        .await
        .commit(&first, Some("resp_first"), Some("completed"), Some(&output));
    drop(session);

    provider
        .update_token(&make_test_jwt("acct_replacement"))
        .await
        .unwrap();

    let replacement = registry.session_for_metadata(&metadata).await.unwrap();
    let full = normalized_input(&[
        ChatMessage::user("first"),
        ChatMessage::assistant("answer"),
        ChatMessage::user("second"),
    ]);
    let plan = replacement.lock().await.plan(&full);
    assert!(plan.previous_response_id.is_none());
    assert_eq!(plan.input, full);
}

#[tokio::test]
async fn account_change_waits_for_in_flight_session_before_publishing_new_auth() {
    let (base_url, mut requests, release_second_response) = test_server::spawn_with_gate(
        vec![
            completed_text_sse(),
            completed_text_sse(),
            completed_text_sse(),
        ],
        1,
    )
    .await;
    let provider = Arc::new(provider_with_session_capability(&base_url));

    let first_messages = vec![ChatMessage::user("first")];
    let mut first = CompletionRequest::new(first_messages.clone());
    first.metadata = agent_loop_metadata();
    let first_response = provider.complete(first).await.unwrap();
    requests.recv().await.unwrap();

    let second_messages = vec![
        ChatMessage::user("first"),
        ChatMessage::assistant(first_response.content),
        ChatMessage::user("second"),
    ];
    let mut second = CompletionRequest::new(second_messages.clone());
    second.metadata = agent_loop_metadata();
    let in_flight_provider = Arc::clone(&provider);
    let in_flight = tokio::spawn(async move { in_flight_provider.complete(second).await });

    let second_body = requests.recv().await.unwrap();
    assert_eq!(second_body["previous_response_id"], "resp_after_tool");
    assert!(
        provider.auth_epoch.try_write().is_err(),
        "session request must retain its account epoch until cursor commit"
    );

    let update_provider = Arc::clone(&provider);
    let update = tokio::spawn(async move {
        update_provider
            .update_token(&make_test_jwt("acct_replacement"))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!update.is_finished());

    release_second_response.notify_one();
    let second_response = in_flight.await.unwrap().unwrap();
    update.await.unwrap().unwrap();

    let third_messages = vec![
        second_messages[0].clone(),
        second_messages[1].clone(),
        second_messages[2].clone(),
        ChatMessage::assistant(second_response.content),
        ChatMessage::user("third"),
    ];
    let mut third = CompletionRequest::new(third_messages.clone());
    third.metadata = agent_loop_metadata();
    provider.complete(third).await.unwrap();

    let third_body = requests.recv().await.unwrap();
    assert!(third_body.get("previous_response_id").is_none());
    assert_eq!(
        third_body["input"],
        serde_json::Value::Array(normalized_input(&third_messages))
    );
}

#[tokio::test]
async fn failed_follow_up_resets_cursor_and_next_request_replays_full() {
    let (base_url, mut requests) = test_server::spawn_with_statuses(vec![
        (200, completed_text_sse()),
        (500, "provider failed".to_string()),
        (200, completed_text_sse()),
    ])
    .await;
    let provider = provider_with_session_capability(&base_url);
    let first_messages = vec![ChatMessage::user("first")];
    let mut first = CompletionRequest::new(first_messages.clone());
    first.metadata = agent_loop_metadata();

    let first_response = provider.complete(first).await.unwrap();
    let first_body = requests.recv().await.unwrap();
    assert!(first_body.get("previous_response_id").is_none());
    assert_eq!(
        first_body["input"],
        serde_json::Value::Array(normalized_input(&first_messages))
    );

    let follow_up_messages = vec![
        ChatMessage::user("first"),
        ChatMessage::assistant(first_response.content),
        ChatMessage::user("second"),
    ];
    let mut failed_follow_up = CompletionRequest::new(follow_up_messages.clone());
    failed_follow_up.metadata = agent_loop_metadata();
    provider.complete(failed_follow_up).await.unwrap_err();

    let failed_body = requests.recv().await.unwrap();
    assert_eq!(failed_body["previous_response_id"], "resp_after_tool");
    assert_eq!(
        failed_body["input"],
        serde_json::Value::Array(normalized_input(&[ChatMessage::user("second")]))
    );

    let mut retry = CompletionRequest::new(follow_up_messages.clone());
    retry.metadata = agent_loop_metadata();
    provider.complete(retry).await.unwrap();

    let retry_body = requests.recv().await.unwrap();
    assert!(retry_body.get("previous_response_id").is_none());
    assert_eq!(
        retry_body["input"],
        serde_json::Value::Array(normalized_input(&follow_up_messages))
    );
}

mod test_server {
    use std::sync::Arc;

    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{Notify, mpsc};

    pub(super) async fn spawn(responses: Vec<String>) -> (String, mpsc::Receiver<Value>) {
        spawn_with_statuses(responses.into_iter().map(|body| (200, body)).collect()).await
    }

    pub(super) async fn spawn_with_gate(
        responses: Vec<String>,
        gated_request_index: usize,
    ) -> (String, mpsc::Receiver<Value>, Arc<Notify>) {
        let release_response = Arc::new(Notify::new());
        let (base_url, requests) = spawn_inner(
            responses.into_iter().map(|body| (200, body)).collect(),
            Some((gated_request_index, Arc::clone(&release_response))),
        )
        .await;
        (base_url, requests, release_response)
    }

    pub(super) async fn spawn_with_statuses(
        responses: Vec<(u16, String)>,
    ) -> (String, mpsc::Receiver<Value>) {
        spawn_inner(responses, None).await
    }

    async fn spawn_inner(
        responses: Vec<(u16, String)>,
        response_gate: Option<(usize, Arc<Notify>)>,
    ) -> (String, mpsc::Receiver<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (requests_tx, requests_rx) = mpsc::channel(responses.len());
        tokio::spawn(async move {
            for (request_index, (status, body)) in responses.into_iter().enumerate() {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests_tx
                    .send(read_body(&mut socket).await)
                    .await
                    .unwrap();
                if let Some((gated_request_index, release_response)) = &response_gate
                    && request_index == *gated_request_index
                {
                    release_response.notified().await;
                }
                let response = format!(
                    "HTTP/1.1 {status} Test\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}"), requests_rx)
    }

    async fn read_body(socket: &mut TcpStream) -> Value {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let (body_start, body_len) = loop {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count != 0, "unexpected EOF while reading HTTP headers");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                let len = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (end + 4, len);
            }
        };
        while bytes.len() < body_start + body_len {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count != 0, "unexpected EOF while reading HTTP body");
            bytes.extend_from_slice(&buffer[..count]);
        }
        serde_json::from_slice(&bytes[body_start..body_start + body_len]).unwrap()
    }

    #[tokio::test]
    async fn read_body_panics_on_eof() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        drop(client);

        let reader = tokio::spawn(async move { read_body(&mut server).await });
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), reader)
            .await
            .expect("EOF handling must fail promptly");
        assert!(result.unwrap_err().is_panic());
    }
}
