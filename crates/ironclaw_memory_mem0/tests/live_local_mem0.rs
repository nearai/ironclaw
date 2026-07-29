//! Live end-to-end probe of the mem0 provider against a **real, self-hosted
//! mem0 OSS server** — no mocks, no hosted cloud, no API key.
//!
//! This drives the production [`Mem0HttpTransport`] (the actual `reqwest` client)
//! through the [`Mem0MemoryService`] adapter against a mem0 OSS server running on
//! localhost (mem0's `Memory` engine behind its REST contract, backed by a local
//! Ollama embedder + local Qdrant vector store). It proves the adapted provider
//! stores and recalls a memory in the LOCAL mem0, and that `profile_set` is
//! field-preserving against the real server.
//!
//! Ignored by default because it needs the local server. Run it with:
//!
//! ```text
//! # stand up: qdrant (:6333) + ollama (:11434, nomic-embed-text) + the mem0 server (:8888)
//! MEM0_TEST_BASE_URL=http://localhost:8888 \
//!   cargo test -p ironclaw_memory_mem0 --test live_local_mem0 -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope, TenantId, UserId};
use ironclaw_memory_mem0::{
    Mem0Config, Mem0HttpTransport, Mem0MemoryService, Mem0Transport, MemoryInvocation,
    MemoryService, MemoryServiceProfileSetRequest, MemoryServiceReadRequest,
    MemoryServiceSearchRequest, MemoryServiceWriteRequest,
};
use serde_json::{Map, json};

fn base_url() -> String {
    std::env::var("MEM0_TEST_BASE_URL").unwrap_or_else(|_| "http://localhost:8888".to_string())
}

/// A unique user per run so repeated runs never read each other's memories.
fn unique_invocation() -> MemoryInvocation {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    MemoryInvocation {
        scope: ResourceScope {
            tenant_id: TenantId::new("probe-tenant").expect("tenant id"),
            user_id: UserId::new(format!("probe-user-{nonce}")).expect("user id"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        correlation_id: CorrelationId::new(),
    }
}

/// Build the provider over the REAL transport with NO API key (the self-hosted
/// server runs with `AUTH_DISABLED=true`).
fn live_service() -> Mem0MemoryService {
    let transport = Mem0HttpTransport::new(&base_url(), None)
        .expect("real transport builds for the local mem0 server");
    Mem0MemoryService::new(
        Arc::new(transport) as Arc<dyn Mem0Transport>,
        Mem0Config::new(),
    )
}

fn write_request(target: &str, content: &str) -> MemoryServiceWriteRequest {
    MemoryServiceWriteRequest {
        target: target.to_string(),
        content: content.to_string(),
        append: true,
        old_string: None,
        new_string: None,
        replace_all: false,
        metadata: None,
        timezone: None,
    }
}

#[tokio::test]
#[ignore = "requires a running local mem0 OSS server at MEM0_TEST_BASE_URL (default http://localhost:8888)"]
async fn store_then_recall_against_local_mem0() {
    let service = live_service();
    let invocation = unique_invocation();
    let target = "notes/probe.md";
    let content = "my favorite color is blue and my deadline is Friday";

    // STORE.
    let write = service
        .write(invocation.clone(), write_request(target, content))
        .await
        .expect("write to local mem0 succeeds");
    assert_eq!(write.path, target);
    eprintln!("stored -> {write:?}");

    // RECALL #1: favorite color.
    let color = service
        .search(
            invocation.clone(),
            MemoryServiceSearchRequest {
                query: "what is my favorite color".to_string(),
                limit: 5,
            },
        )
        .await
        .expect("color search succeeds");
    eprintln!("color recall -> {color:?}");
    assert!(
        color
            .results
            .iter()
            .any(|r| r.content.to_lowercase().contains("blue")),
        "favorite-color recall must surface 'blue' from local mem0"
    );

    // RECALL #2: deadline.
    let deadline = service
        .search(
            invocation.clone(),
            MemoryServiceSearchRequest {
                query: "when is my deadline".to_string(),
                limit: 5,
            },
        )
        .await
        .expect("deadline search succeeds");
    eprintln!("deadline recall -> {deadline:?}");
    assert!(
        deadline
            .results
            .iter()
            .any(|r| r.content.to_lowercase().contains("friday")),
        "deadline recall must surface 'Friday' from local mem0"
    );

    // READ-BACK the verbatim document (infer=false round-trip via metadata.target).
    let read = service
        .read(
            invocation.clone(),
            MemoryServiceReadRequest {
                path: target.to_string(),
            },
        )
        .await
        .expect("read of the stored target succeeds");
    eprintln!("read-back -> {read:?}");
    let lower = read.content.to_lowercase();
    assert!(
        lower.contains("blue") && lower.contains("friday"),
        "verbatim read-back must contain both stored facts"
    );
}

#[tokio::test]
#[ignore = "requires a running local mem0 OSS server at MEM0_TEST_BASE_URL (default http://localhost:8888)"]
async fn profile_set_is_field_preserving_against_local_mem0() {
    let service = live_service();
    let invocation = unique_invocation();

    // First write sets one field.
    let mut first = Map::new();
    first.insert("timezone".to_string(), json!("PST"));
    service
        .profile_set(
            invocation.clone(),
            MemoryServiceProfileSetRequest { fields: first },
        )
        .await
        .expect("first profile_set succeeds");

    // Second write sets a DIFFERENT field; the first must survive (read-merge-write).
    let mut second = Map::new();
    second.insert("language".to_string(), json!("en"));
    service
        .profile_set(
            invocation.clone(),
            MemoryServiceProfileSetRequest { fields: second },
        )
        .await
        .expect("second profile_set succeeds");

    let read = service
        .profile_read(invocation.clone())
        .await
        .expect("profile_read succeeds");
    let bytes = read.document.expect("a profile document is present");
    let object: serde_json::Value =
        serde_json::from_slice(&bytes).expect("profile bytes are valid JSON");
    eprintln!("merged profile -> {object}");
    assert_eq!(object["timezone"], json!("PST"), "earlier field preserved");
    assert_eq!(object["language"], json!("en"), "later field applied");
}
