mod support;

use support::{MockServer, RecordedRequest, ScriptedResponse};

#[tokio::test]
async fn list_threads_sends_bearer_auth_and_parses_thread_summaries() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/threads",
        ScriptedResponse::ok(serde_json::json!({
            "threads": [
                {"thread_id": "thread-1", "title": "Alpha", "updated_at": "2026-07-15T00:00:00Z"},
                {"thread_id": "thread-2", "title": null, "updated_at": null}
            ],
            "next_cursor": null
        })),
    );

    let client = server.client();
    let threads = client.list_threads().await.expect("list threads");

    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].thread_id, "thread-1");
    assert_eq!(threads[0].title.as_deref(), Some("Alpha"));
    assert_eq!(threads[1].title, None);

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let RecordedRequest { authorization, .. } = &requests[0];
    assert_eq!(
        authorization.as_deref(),
        Some(format!("Bearer {}", support::TEST_TOKEN).as_str())
    );
}

#[tokio::test]
async fn list_threads_maps_401_to_unauthorized() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/threads",
        ScriptedResponse::status(401, serde_json::json!({"error": "unauthorized"})),
    );

    let client = server.client();
    let error = client.list_threads().await.expect_err("expected error");
    assert!(matches!(
        error,
        ironclaw_reborn_tui::client::ClientError::Unauthorized
    ));
}

#[tokio::test]
async fn create_thread_posts_client_action_id_and_returns_new_thread() {
    let server = MockServer::start().await;
    server.queue(
        "POST /api/webchat/v2/threads",
        ScriptedResponse::ok(serde_json::json!({
            "thread": {"thread_id": "thread-new", "title": null}
        })),
    );

    let client = server.client();
    let thread = client.create_thread().await.expect("create thread");
    assert_eq!(thread.thread_id, "thread-new");

    let requests = server.requests();
    let body = requests[0].body.clone().expect("body");
    let client_action_id = body["client_action_id"]
        .as_str()
        .expect("client_action_id is a string");
    assert!(
        !client_action_id.is_empty(),
        "client_action_id must be non-empty"
    );
}

#[tokio::test]
async fn delete_thread_sends_delete_to_thread_path() {
    let server = MockServer::start().await;
    server.queue(
        "DELETE /api/webchat/v2/threads/thread-1",
        ScriptedResponse::ok(serde_json::json!({"thread_id": "thread-1", "deleted": true})),
    );

    let client = server.client();
    client
        .delete_thread("thread-1")
        .await
        .expect("delete thread");
    assert_eq!(server.requests()[0].method, "DELETE");
}

#[tokio::test]
async fn timeline_sends_limit_and_cursor_query_params() {
    let server = MockServer::start().await;
    server.queue(
        "GET /api/webchat/v2/threads/thread-1/timeline",
        ScriptedResponse::ok(serde_json::json!({
            "thread": {"thread_id": "thread-1", "title": "Alpha"},
            "messages": [
                {"message_id": "m1", "sequence": 1, "kind": "user", "status": "finalized", "content": "hi"}
            ],
            "next_cursor": "cursor-2"
        })),
    );

    let client = server.client();
    let page = client
        .timeline("thread-1", 50, Some("cursor-1".to_string()))
        .await
        .expect("timeline");

    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].content.as_deref(), Some("hi"));
    assert_eq!(page.next_cursor.as_deref(), Some("cursor-2"));

    let requests = server.requests();
    let request = &requests[0];
    assert_eq!(request.path, "/api/webchat/v2/threads/thread-1/timeline");
    assert_eq!(
        request.query.as_deref(),
        Some("limit=50&cursor=cursor-1"),
        "timeline must preserve the public caller's exact limit and cursor"
    );
}
