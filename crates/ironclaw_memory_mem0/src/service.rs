//! The mem0-backed [`MemoryService`] adapter.
//!
//! `Mem0MemoryService` maps the provider-neutral IronClaw memory operations onto
//! the mem0 REST API (`POST /v1/memories/`, `POST /v1/memories/search/`,
//! `GET /v1/memories/`). It owns no HTTP client: every call goes through the
//! injected [`Mem0Transport`], so the same provider is exercised by the real
//! [`crate::Mem0HttpTransport`] in production and an in-memory mock in tests.
//!
//! ## Mapping fidelity
//!
//! mem0 models *discrete, semantically-indexed memories keyed by `user_id`*, not
//! an addressable document tree. Some IronClaw operations therefore map cleanly
//! and some do not. Each non-clean mapping is marked `MAPPING GAP` at its call
//! site:
//!
//! | IronClaw op        | mem0 mapping                                   | fidelity |
//! |--------------------|------------------------------------------------|----------|
//! | `search`           | `POST /search/`                                | clean    |
//! | `retrieve_context` | `POST /search/` → snippets                     | clean    |
//! | `write` (add)      | `POST /memories/` add                          | good     |
//! | `read`             | `GET /memories/` + filter by `target` metadata | loose    |
//! | `tree`             | `GET /memories/` → distinct `target` tags      | loose    |
//! | `write` (patch)    | unsupported (no substring patch in mem0)       | none     |
//! | `write` (clear)    | unsupported (no addressable doc to truncate)   | none     |
//! | `profile_set`      | add a `kind=profile` memory (no merge / CAS)   | loose    |
//! | `profile_read`     | latest `kind=profile` memory bytes             | loose    |

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::ResourceScope;
use ironclaw_memory::{
    MemoryInvocation, MemoryProfileSetStatus, MemoryService, MemoryServiceContextRequest,
    MemoryServiceContextSnippet, MemoryServiceError, MemoryServiceProfileReadResponse,
    MemoryServiceProfileSetRequest, MemoryServiceProfileSetResponse, MemoryServiceReadRequest,
    MemoryServiceReadResponse, MemoryServiceSearchRequest, MemoryServiceSearchResponse,
    MemoryServiceSearchResult, MemoryServiceTreeRequest, MemoryServiceTreeResponse,
    MemoryServiceWriteRequest, MemoryServiceWriteResponse, MemoryWriteStatus,
    memory_context_disabled,
};
use serde_json::{Value, json};

use crate::config::Mem0Config;
use crate::error::Mem0Error;
use crate::transport::{Mem0HttpRequest, Mem0HttpResponse, Mem0Transport};

const ADD_PATH: &str = "/v1/memories/";
const SEARCH_PATH: &str = "/v1/memories/search/";
const LIST_PATH: &str = "/v1/memories/";
const USER_ID_QUERY: &str = "user_id";
const TARGET_KEY: &str = "target";
const SOURCE_KEY: &str = "source";
const SOURCE_VALUE: &str = "ironclaw.memory";
const KIND_KEY: &str = "kind";
const PROFILE_KIND: &str = "profile";
/// Mirrors the native provider's profile document location so a deployment that
/// swaps providers keeps a recognizable profile target tag.
const PROFILE_TARGET: &str = "context/profile.json";

/// A [`MemoryService`] backed by the mem0 REST API over an injected transport.
pub struct Mem0MemoryService {
    transport: Arc<dyn Mem0Transport>,
    config: Mem0Config,
}

impl std::fmt::Debug for Mem0MemoryService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Mem0MemoryService")
            .field("transport", &"<mem0-transport>")
            .field("config", &self.config)
            .finish()
    }
}

impl Mem0MemoryService {
    /// Build the provider over a transport + behavior config.
    pub fn new(transport: Arc<dyn Mem0Transport>, config: Mem0Config) -> Self {
        Self { transport, config }
    }

    fn add_body(&self, namespace: &str, content: &str, metadata: Value) -> Value {
        let mut body = json!({
            "messages": [{ "role": "user", "content": content }],
            "user_id": namespace,
            "metadata": metadata,
        });
        self.stamp_app_id(&mut body);
        body
    }

    fn search_body(&self, namespace: &str, query: &str, limit: usize) -> Value {
        let mut body = json!({ "query": query, "user_id": namespace, "limit": limit });
        self.stamp_app_id(&mut body);
        body
    }

    fn stamp_app_id(&self, body: &mut Value) {
        if let (Some(app_id), Some(object)) = (self.config.app_id.as_deref(), body.as_object_mut())
        {
            object.insert("app_id".to_string(), json!(app_id));
        }
    }
}

#[async_trait]
impl MemoryService for Mem0MemoryService {
    async fn search(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceSearchRequest,
    ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
        let namespace = scope_namespace(&invocation.scope);
        let body = self.search_body(&namespace, &request.query, request.limit);
        let response = self
            .transport
            .execute(Mem0HttpRequest::post(SEARCH_PATH, body))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "search").map_err(MemoryServiceError::operation_from)?;
        let results = response_items(&response.body)
            .into_iter()
            .filter_map(|item| {
                Some(MemoryServiceSearchResult {
                    content: item_text(item)?.to_string(),
                    score: item_score(item),
                    path: result_path(item),
                    // mem0 search is semantic-only; there is no FTS+vector fusion
                    // to report, so a result is never a "hybrid" match.
                    is_hybrid_match: false,
                })
            })
            .take(request.limit)
            .collect();
        Ok(MemoryServiceSearchResponse {
            query: request.query,
            results,
        })
    }

    async fn write(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceWriteRequest,
    ) -> Result<MemoryServiceWriteResponse, MemoryServiceError> {
        // MAPPING GAP: mem0 stores discrete memories, not addressable documents,
        // so it has no in-place substring patch. Fail explicitly rather than
        // silently adding a new memory that ignores the requested edit.
        if request.old_string.is_some() || request.new_string.is_some() {
            return Err(MemoryServiceError::operation_from(Mem0Error::Unsupported {
                operation: "write.patch",
                detail: "mem0 has no in-place substring patch",
            }));
        }
        // MAPPING GAP: an empty write is the native `bootstrap` clear, which has
        // no mem0 analogue (no addressable document to truncate). Treat it as an
        // invalid request, matching the native provider's empty-content rule.
        if request.content.trim().is_empty() {
            return Err(MemoryServiceError::input());
        }
        let namespace = scope_namespace(&invocation.scope);
        let metadata = json!({ TARGET_KEY: request.target, SOURCE_KEY: SOURCE_VALUE });
        let body = self.add_body(&namespace, &request.content, metadata);
        let response = self
            .transport
            .execute(Mem0HttpRequest::post(ADD_PATH, body))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "write").map_err(MemoryServiceError::operation_from)?;
        Ok(MemoryServiceWriteResponse {
            status: MemoryWriteStatus::Written,
            path: request.target,
            // mem0 is inherently additive: every write adds a memory.
            append: true,
            content_length: request.content.len(),
            replacements: None,
            message: Some("stored as a mem0 memory".to_string()),
        })
    }

    async fn read(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceReadRequest,
    ) -> Result<MemoryServiceReadResponse, MemoryServiceError> {
        let namespace = scope_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(
                LIST_PATH,
                vec![(USER_ID_QUERY.to_string(), namespace)],
            ))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "read").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: mem0 is not path-addressable. Reconstruct a "document" by
        // concatenating every memory tagged with the requested `target`.
        let parts: Vec<String> = response_items(&response.body)
            .into_iter()
            .filter(|item| item_metadata_str(item, TARGET_KEY) == Some(request.path.as_str()))
            .filter_map(|item| item_text(item).map(str::to_string))
            .collect();
        if parts.is_empty() {
            return Err(MemoryServiceError::input());
        }
        let content = parts.join("\n");
        Ok(MemoryServiceReadResponse {
            word_count: content.split_whitespace().count(),
            path: request.path,
            content,
        })
    }

    async fn tree(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceTreeRequest,
    ) -> Result<MemoryServiceTreeResponse, MemoryServiceError> {
        let namespace = scope_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(
                LIST_PATH,
                vec![(USER_ID_QUERY.to_string(), namespace)],
            ))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "tree").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: mem0 has no document hierarchy. Best-effort: surface the
        // distinct `target` tags (optionally prefix-filtered) as a flat list.
        let mut targets = std::collections::BTreeSet::new();
        for item in response_items(&response.body) {
            if let Some(target) = item_metadata_str(item, TARGET_KEY)
                && (request.path.is_empty() || target.starts_with(request.path.as_str()))
            {
                targets.insert(target.to_string());
            }
        }
        Ok(MemoryServiceTreeResponse {
            entries: targets.into_iter().map(Value::String).collect(),
        })
    }

    async fn profile_set(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceProfileSetRequest,
    ) -> Result<MemoryServiceProfileSetResponse, MemoryServiceError> {
        // MAPPING GAP: mem0 has no structured profile document and no
        // compare-and-set. Best-effort: serialize the supplied fields and add
        // them as a `kind=profile` memory. This is last-writer-wins with NO
        // field-level merge, unlike the native provider's read-modify-write of
        // `context/profile.json`.
        let namespace = owner_namespace(&invocation.scope);
        let serialized = Value::Object(request.fields).to_string();
        let metadata = json!({
            KIND_KEY: PROFILE_KIND,
            TARGET_KEY: PROFILE_TARGET,
            SOURCE_KEY: SOURCE_VALUE,
        });
        let body = self.add_body(&namespace, &serialized, metadata);
        let response = self
            .transport
            .execute(Mem0HttpRequest::post(ADD_PATH, body))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "profile_set").map_err(MemoryServiceError::operation_from)?;
        Ok(MemoryServiceProfileSetResponse {
            status: MemoryProfileSetStatus::Ok,
        })
    }

    async fn profile_read(
        &self,
        invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        let namespace = owner_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(
                LIST_PATH,
                vec![(USER_ID_QUERY.to_string(), namespace)],
            ))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "profile_read").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: return the last-listed `kind=profile` memory's raw bytes,
        // if any. The host parses + size-caps them, exactly as for native.
        let document = response_items(&response.body)
            .into_iter()
            .filter(|item| item_metadata_str(item, KIND_KEY) == Some(PROFILE_KIND))
            .filter_map(item_text)
            .next_back()
            .map(|text| text.as_bytes().to_vec());
        Ok(MemoryServiceProfileReadResponse { document })
    }

    async fn retrieve_context(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        if request.max_snippets == 0 || memory_context_disabled(request.context_profile_id.as_str())
        {
            return Ok(Vec::new());
        }
        let namespace = scope_namespace(&invocation.scope);
        let body = self.search_body(&namespace, &request.query, request.max_snippets);
        // Context retrieval degrades to "no memory context" (Unavailable) on a
        // backend failure rather than aborting the turn, matching native.
        let response = self
            .transport
            .execute(Mem0HttpRequest::post(SEARCH_PATH, body))
            .await
            .map_err(MemoryServiceError::unavailable_from)?;
        ensure_success(&response, "retrieve_context")
            .map_err(MemoryServiceError::unavailable_from)?;
        let scope = &invocation.scope;
        // Return raw, unsanitized snippet bodies plus the resolved scope/path
        // components. The host sanitizes the text, wraps it in the
        // untrusted-memory envelope, hashes the reference, and enforces the
        // model-visible budgets — this provider never shapes model-visible
        // content (see `MemoryServiceContextSnippet` docs).
        let snippets = response_items(&response.body)
            .into_iter()
            .filter_map(|item| {
                Some(MemoryServiceContextSnippet {
                    tenant_id: scope.tenant_id.as_str().to_string(),
                    user_id: scope.user_id.as_str().to_string(),
                    agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
                    project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
                    relative_path: result_path(item),
                    text: item_text(item)?.to_string(),
                })
            })
            .take(request.max_snippets)
            .collect();
        Ok(snippets)
    }
}

/// mem0 partitions memories by a free-form `user_id`. Compose the full owner
/// identity so tenants/agents/projects never share a memory pool.
fn scope_namespace(scope: &ResourceScope) -> String {
    let mut namespace = format!("{}/{}", scope.tenant_id.as_str(), scope.user_id.as_str());
    if let Some(agent) = scope.agent_id.as_ref() {
        namespace.push_str("/agent=");
        namespace.push_str(agent.as_str());
    }
    if let Some(project) = scope.project_id.as_ref() {
        namespace.push_str("/project=");
        namespace.push_str(project.as_str());
    }
    namespace
}

/// The profile document is keyed to the human user only (agent/project cleared),
/// matching the native provider's profile scope.
fn owner_namespace(scope: &ResourceScope) -> String {
    format!("{}/{}", scope.tenant_id.as_str(), scope.user_id.as_str())
}

fn ensure_success(response: &Mem0HttpResponse, operation: &'static str) -> Result<(), Mem0Error> {
    if response.is_success() {
        Ok(())
    } else {
        Err(Mem0Error::Api {
            operation,
            status: response.status,
        })
    }
}

/// Extract the list of memory objects from a mem0 response, tolerating the
/// documented shapes: a bare array, or an object wrapping `results`/`memories`/
/// `data`.
fn response_items(body: &Value) -> Vec<&Value> {
    if let Some(array) = body.as_array() {
        return array.iter().collect();
    }
    for key in ["results", "memories", "data"] {
        if let Some(array) = body.get(key).and_then(Value::as_array) {
            return array.iter().collect();
        }
    }
    Vec::new()
}

fn item_text(item: &Value) -> Option<&str> {
    ["memory", "content", "text", "data"]
        .into_iter()
        .find_map(|key| item.get(key).and_then(Value::as_str))
}

fn item_score(item: &Value) -> f32 {
    item.get("score")
        .and_then(Value::as_f64)
        .map(|score| score as f32)
        .unwrap_or(0.0)
}

fn item_metadata_str<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get(key))
        .and_then(Value::as_str)
}

fn item_id(item: &Value) -> Option<&str> {
    item.get("id").and_then(Value::as_str)
}

fn result_path(item: &Value) -> String {
    item_metadata_str(item, TARGET_KEY)
        .or_else(|| item_id(item))
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Mem0HttpMethod, MockMem0Transport};
    use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope, TenantId, UserId};
    use ironclaw_memory::{MemoryContextProfileId, MemoryServiceErrorKind};
    use serde_json::Map;

    fn invocation() -> MemoryInvocation {
        MemoryInvocation {
            scope: ResourceScope {
                tenant_id: TenantId::new("tenant-mem0").unwrap(),
                user_id: UserId::new("user-mem0").unwrap(),
                agent_id: None,
                project_id: None,
                mission_id: None,
                thread_id: None,
                invocation_id: InvocationId::new(),
            },
            correlation_id: CorrelationId::new(),
        }
    }

    fn service_with(transport: MockMem0Transport) -> (Mem0MemoryService, Arc<MockMem0Transport>) {
        let transport = Arc::new(transport);
        let service = Mem0MemoryService::new(
            Arc::clone(&transport) as Arc<dyn Mem0Transport>,
            Mem0Config::new(),
        );
        (service, transport)
    }

    #[tokio::test]
    async fn write_routes_an_add_through_the_transport() {
        let (service, transport) =
            service_with(MockMem0Transport::always_ok(json!({ "id": "m-1" })));
        let response = service
            .write(
                invocation(),
                MemoryServiceWriteRequest {
                    target: "notes/alpha.md".to_string(),
                    content: "alpha mem0 marker".to_string(),
                    append: true,
                    old_string: None,
                    new_string: None,
                    replace_all: false,
                    metadata: None,
                    timezone: None,
                },
            )
            .await
            .expect("write should succeed");
        assert_eq!(response.status, MemoryWriteStatus::Written);
        assert_eq!(response.path, "notes/alpha.md");

        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 1);
        let request = &recorded[0];
        assert_eq!(request.method, Mem0HttpMethod::Post);
        assert_eq!(request.path, ADD_PATH);
        let body = request.body.as_ref().expect("add body");
        assert_eq!(body["user_id"], json!("tenant-mem0/user-mem0"));
        assert_eq!(body["messages"][0]["content"], json!("alpha mem0 marker"));
        assert_eq!(body["metadata"]["target"], json!("notes/alpha.md"));
    }

    #[tokio::test]
    async fn write_patch_is_unsupported() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!({})));
        let error = service
            .write(
                invocation(),
                MemoryServiceWriteRequest {
                    target: "notes/alpha.md".to_string(),
                    content: "ignored".to_string(),
                    append: false,
                    old_string: Some("old".to_string()),
                    new_string: Some("new".to_string()),
                    replace_all: false,
                    metadata: None,
                    timezone: None,
                },
            )
            .await
            .expect_err("patch must be rejected");
        assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
    }

    #[tokio::test]
    async fn search_parses_results_object_shape() {
        let (service, transport) = service_with(MockMem0Transport::always_ok(json!({
            "results": [
                { "id": "m-1", "memory": "first", "score": 0.91, "metadata": { "target": "notes/a.md" } },
                { "id": "m-2", "memory": "second", "score": 0.42 }
            ]
        })));
        let response = service
            .search(
                invocation(),
                MemoryServiceSearchRequest {
                    query: "first".to_string(),
                    limit: 5,
                },
            )
            .await
            .expect("search should succeed");
        assert_eq!(response.results.len(), 2);
        assert_eq!(response.results[0].content, "first");
        assert!((response.results[0].score - 0.91).abs() < 1e-6);
        assert_eq!(response.results[0].path, "notes/a.md");
        // Fell back to the mem0 id when no target tag is present.
        assert_eq!(response.results[1].path, "m-2");

        let request = &transport.recorded()[0];
        assert_eq!(request.path, SEARCH_PATH);
        assert_eq!(
            request.body.as_ref().expect("search body")["query"],
            json!("first")
        );
    }

    #[tokio::test]
    async fn search_parses_bare_array_shape() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            { "id": "m-1", "content": "bare", "score": 0.5 }
        ])));
        let response = service
            .search(
                invocation(),
                MemoryServiceSearchRequest {
                    query: "bare".to_string(),
                    limit: 5,
                },
            )
            .await
            .expect("search should succeed");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].content, "bare");
    }

    #[tokio::test]
    async fn read_filters_by_target_and_reports_not_found() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            { "memory": "kept", "metadata": { "target": "notes/a.md" } },
            { "memory": "other", "metadata": { "target": "notes/b.md" } }
        ])));
        let read = service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/a.md".to_string(),
                },
            )
            .await
            .expect("read should succeed");
        assert_eq!(read.content, "kept");
        assert_eq!(read.path, "notes/a.md");

        let missing = service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/missing.md".to_string(),
                },
            )
            .await
            .expect_err("absent target is not found");
        assert_eq!(missing.kind(), MemoryServiceErrorKind::Input);
    }

    #[tokio::test]
    async fn retrieve_context_maps_search_hits_to_snippets() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!({
            "results": [ { "id": "m-1", "memory": "ctx hit", "metadata": { "target": "notes/a.md" } } ]
        })));
        let snippets = service
            .retrieve_context(
                invocation(),
                MemoryServiceContextRequest {
                    query: "ctx".to_string(),
                    max_snippets: 3,
                    context_profile_id: MemoryContextProfileId::new("default").unwrap(),
                },
            )
            .await
            .expect("retrieve_context should succeed");
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].text, "ctx hit");
        assert_eq!(snippets[0].tenant_id, "tenant-mem0");
        assert_eq!(snippets[0].user_id, "user-mem0");
        assert_eq!(snippets[0].relative_path, "notes/a.md");
    }

    #[tokio::test]
    async fn retrieve_context_short_circuits_when_disabled_or_empty() {
        let (service, transport) =
            service_with(MockMem0Transport::always_ok(json!({ "results": [] })));
        let disabled = service
            .retrieve_context(
                invocation(),
                MemoryServiceContextRequest {
                    query: "ctx".to_string(),
                    max_snippets: 3,
                    context_profile_id: MemoryContextProfileId::new("memory_disabled").unwrap(),
                },
            )
            .await
            .expect("disabled profile yields no snippets");
        assert!(disabled.is_empty());

        let zero = service
            .retrieve_context(
                invocation(),
                MemoryServiceContextRequest {
                    query: "ctx".to_string(),
                    max_snippets: 0,
                    context_profile_id: MemoryContextProfileId::new("default").unwrap(),
                },
            )
            .await
            .expect("zero snippets yields no snippets");
        assert!(zero.is_empty());
        // Neither short-circuit should have touched the transport.
        assert!(transport.recorded().is_empty());
    }

    #[tokio::test]
    async fn tree_lists_distinct_targets() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            { "memory": "1", "metadata": { "target": "notes/a.md" } },
            { "memory": "2", "metadata": { "target": "notes/b.md" } },
            { "memory": "3", "metadata": { "target": "notes/a.md" } }
        ])));
        let tree = service
            .tree(
                invocation(),
                MemoryServiceTreeRequest {
                    path: String::new(),
                    depth: 2,
                },
            )
            .await
            .expect("tree should succeed");
        assert_eq!(tree.entries, vec![json!("notes/a.md"), json!("notes/b.md")]);
    }

    #[tokio::test]
    async fn profile_set_then_read_round_trips_bytes() {
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            {
                "memory": "{\"timezone\":\"UTC\"}",
                "metadata": { "kind": "profile", "target": "context/profile.json" }
            }
        ])));
        let mut fields = Map::new();
        fields.insert("timezone".to_string(), json!("UTC"));
        let set = service
            .profile_set(invocation(), MemoryServiceProfileSetRequest { fields })
            .await
            .expect("profile_set should succeed");
        assert_eq!(set.status, MemoryProfileSetStatus::Ok);

        let read = service
            .profile_read(invocation())
            .await
            .expect("profile_read should succeed");
        assert_eq!(read.document, Some(b"{\"timezone\":\"UTC\"}".to_vec()));
    }

    #[tokio::test]
    async fn non_success_status_is_an_operation_error() {
        let (service, _transport) = service_with(MockMem0Transport::new(Box::new(|_request| {
            Some(Mem0HttpResponse {
                status: 503,
                body: json!({ "error": "unavailable" }),
            })
        })));
        let error = service
            .search(
                invocation(),
                MemoryServiceSearchRequest {
                    query: "x".to_string(),
                    limit: 5,
                },
            )
            .await
            .expect_err("503 should surface as an error");
        assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
    }
}
