//! mem0's wiring of the shared provider contract suite
//! (`ironclaw_memory::test_support`).
//!
//! mem0 declares only the long-term retrieval lane (no thread partitioning,
//! no interaction recording), so it wires the retrieval-only suite: scope
//! isolation across tenant/user/agent/project. The backing is a STATEFUL fake
//! mem0 server (not the scripted `MockMem0Transport`): it stores added
//! memories and filters search/list by the exact `user_id` namespace — the
//! same key the real self-hosted server enforces — so the contract proves the
//! provider derives a distinct namespace per scope and round-trips through
//! it, end to end at this crate's seam.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_memory_mem0::{
    Mem0Config, Mem0HttpRequest, Mem0HttpResponse, Mem0MemoryService, Mem0Transport,
    Mem0TransportError,
};
use serde_json::{Value, json};

/// One stored fake-server memory row: the enforced `user_id` namespace plus
/// the verbatim content and metadata the provider added.
#[derive(Clone)]
struct FakeMemoryRow {
    user_id: String,
    content: String,
    metadata: Value,
}

/// A stateful in-memory mem0 OSS server: `POST /memories` stores,
/// `POST /search` and `GET /memories` filter by the exact `user_id`
/// namespace (the isolation key the real server enforces). Search is a
/// naive shared-token match — enough for the contract's marker queries.
#[derive(Default)]
struct FakeMem0Server {
    rows: Mutex<Vec<FakeMemoryRow>>,
}

impl FakeMem0Server {
    fn items(rows: &[&FakeMemoryRow]) -> Value {
        json!({
            "results": rows
                .iter()
                .map(|row| json!({ "memory": row.content, "metadata": row.metadata }))
                .collect::<Vec<_>>()
        })
    }
}

#[async_trait]
impl Mem0Transport for FakeMem0Server {
    async fn execute(
        &self,
        request: Mem0HttpRequest,
    ) -> Result<Mem0HttpResponse, Mem0TransportError> {
        let mut rows = self.rows.lock().expect("fake server lock");
        let body = request.body.unwrap_or(Value::Null);
        let ok = |body: Value| Ok(Mem0HttpResponse { status: 200, body });
        match request.path.as_str() {
            "/memories" if body.is_object() => {
                // POST add: store the first message's content under the
                // request's user_id namespace, verbatim (infer=false shape).
                let user_id = body["user_id"].as_str().unwrap_or_default().to_string();
                let content = body["messages"][0]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                rows.push(FakeMemoryRow {
                    user_id,
                    content,
                    metadata: body["metadata"].clone(),
                });
                ok(json!({ "results": [] }))
            }
            "/memories" => {
                // GET list: filter by the user_id query parameter.
                let user_id = request
                    .query
                    .iter()
                    .find(|(key, _)| key == "user_id")
                    .map(|(_, value)| value.as_str())
                    .unwrap_or_default();
                let matched: Vec<&FakeMemoryRow> =
                    rows.iter().filter(|row| row.user_id == user_id).collect();
                ok(Self::items(&matched))
            }
            "/search" => {
                let user_id = body["user_id"].as_str().unwrap_or_default();
                let query = body["query"].as_str().unwrap_or_default();
                let matched: Vec<&FakeMemoryRow> = rows
                    .iter()
                    .filter(|row| row.user_id == user_id)
                    .filter(|row| {
                        query
                            .split_whitespace()
                            .any(|token| row.content.contains(token))
                    })
                    .collect();
                ok(Self::items(&matched))
            }
            other => panic!("fake mem0 server: unexpected path {other}"),
        }
    }
}

// mem0 declares only `read_long_term` (+ profile reads); the lane-disjointness
// and record round-trip contracts are for hooks it does not declare — the
// host never calls them (`read_short_term_stays_at_the_unavailable_default`
// in the unit suite pins the fail-closed default).
ironclaw_memory::memory_service_contract_retrieval_only!(
    mem0_provider,
    || Mem0MemoryService::new(
        Arc::new(FakeMem0Server::default()),
        Mem0Config { app_id: None },
    ),
    async |service: &Mem0MemoryService, invocation, request| {
        service
            .write(invocation, request)
            .await
            .expect("seed write through mem0's own write operation");
    }
);
