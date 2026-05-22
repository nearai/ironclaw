//! Read-path integration tests for the Gmail package.
//!
//! Exercises `list_messages` and `get_message` end to end through the
//! `FirstPartyCapabilityHandler` trait, driven by a fake `RuntimeHttpEgress`
//! over recorded fixtures.

mod support;

use std::sync::Arc;

use ironclaw_host_api::RuntimeKind;
use ironclaw_host_runtime::FirstPartyCapabilityHandler;
use ironclaw_native_extensions::google::gmail::handlers::{GetMessageHandler, ListMessagesHandler};
use ironclaw_native_extensions::google::scopes;
use ironclaw_secrets::InMemorySecretStore;
use serde_json::{Value, json};

use support::{FakeEgress, build_gmail_deps, gmail_request, seed_token, test_scope};

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/tests/fixtures/google_api/gmail/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse fixture {path}: {e}"))
}

#[tokio::test]
async fn list_messages_projects_whitelisted_refs_and_paging() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let egress = FakeEgress::single(200, fixture("messages_list.json"));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = ListMessagesHandler::new(deps);

    let result = handler
        .dispatch(gmail_request(
            "list_messages",
            scope,
            json!({ "query": "is:unread from:ada", "max_results": 25 }),
            egress.clone(),
        ))
        .await
        .expect("list_messages succeeds");

    let messages = result
        .output
        .get("messages")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[0].get("id").and_then(Value::as_str),
        Some("msg-001")
    );
    assert_eq!(
        messages[0].get("thread_id").and_then(Value::as_str),
        Some("thr-001")
    );
    assert_eq!(
        result.output.get("next_page_token").and_then(Value::as_str),
        Some("PG-9f3a2")
    );

    // The handler issued one GET to the messages endpoint through the host
    // runtime-egress boundary, with the query string url-encoded.
    let recorded = egress.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].runtime, RuntimeKind::FirstParty);
    let url = &recorded[0].url;
    assert!(url.contains("/users/me/messages"));
    assert!(url.contains("q=is%3Aunread%20from%3Aada"));
    assert!(url.contains("maxResults=25"));
    // The handler never sets an authorization header itself — the host egress
    // injects the staged credential.
    assert!(
        !recorded[0]
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
        "handler must not inject the token itself"
    );
    assert_eq!(recorded[0].credential_injections.len(), 1);
}

#[tokio::test]
async fn get_message_projects_whitelisted_headers_and_drops_internal_ids() {
    let scope = test_scope();
    let secrets = Arc::new(InMemorySecretStore::new());
    seed_token(&secrets, &scope, &[scopes::GMAIL_READONLY]).await;
    let egress = FakeEgress::single(200, fixture("message_get.json"));
    let deps = build_gmail_deps(secrets, &[scopes::GMAIL_READONLY]);
    let handler = GetMessageHandler::new(deps);

    let result = handler
        .dispatch(gmail_request(
            "get_message",
            scope,
            json!({ "message_id": "msg-001" }),
            egress.clone(),
        ))
        .await
        .expect("get_message succeeds");

    assert_eq!(
        result.output.get("id").and_then(Value::as_str),
        Some("msg-001")
    );
    let headers = result.output.get("headers").unwrap();
    assert_eq!(
        headers.get("subject").and_then(Value::as_str),
        Some("Q2 summary")
    );
    assert_eq!(
        headers.get("from").and_then(Value::as_str),
        Some("Ada Lovelace <ada@example.com>")
    );

    let serialized = serde_json::to_string(&result.output).expect("serializes");
    // Internal ids and un-whitelisted raw headers must be stripped.
    assert!(!serialized.contains("internalDate"));
    assert!(!serialized.contains("historyId"));
    assert!(!serialized.contains("X-Internal-Trace-Id"));
    assert!(!serialized.contains("trace-should-not-leak"));

    let recorded = egress.recorded();
    assert!(recorded[0].url.contains("/users/me/messages/msg-001"));
    assert!(recorded[0].url.contains("format=full"));
}
