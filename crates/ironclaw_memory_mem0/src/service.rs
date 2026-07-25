//! The mem0-backed [`MemoryService`] adapter.
//!
//! `Mem0MemoryService` maps the provider-neutral IronClaw memory operations onto
//! the self-hosted mem0 OSS REST API (`POST /memories`, `POST /search`,
//! `GET /memories`). It owns no HTTP client: every call goes through the
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
//! | `search`           | `POST /search`                                 | clean    |
//! | `retrieve_context` | `POST /search` → snippets                      | clean    |
//! | `write` (add)      | `POST /memories` add (`infer=false`, verbatim) | good     |
//! | `read`             | `GET /memories` + filter by `target` metadata  | loose    |
//! | `tree`             | `GET /memories` → distinct `target` tags       | loose    |
//! | `write` (patch)    | unsupported (no substring patch in mem0)       | none     |
//! | `write` (clear)    | unsupported (no addressable doc to truncate)   | none     |
//! | `profile_set`      | read-merge-write a `kind=profile` memory       | good     |
//! | `profile_read`     | latest `kind=profile` memory bytes             | loose    |
//!
//! ## `infer=false` (verbatim) document-store mapping
//!
//! mem0's `add` defaults to running an LLM to *extract facts* from the message
//! (e.g. "I love pizza" → "Likes pizza"). For a **document store** that must
//! round-trip exactly — `read` reconstructs a document from the memories tagged
//! with a `target`, and `profile_set`/`profile_read` round-trip a structured JSON
//! blob — that rewrite is lossy. Every add this provider issues therefore sets
//! `infer=false`, so mem0 stores the content **verbatim** and uses only its
//! embedder (a self-hosted Ollama embedder in the all-local deployment), never an
//! LLM. Semantic `search` still works over the verbatim text.

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
use serde_json::{Map, Value, json};

use crate::config::Mem0Config;
use crate::error::Mem0Error;
use crate::transport::{Mem0HttpRequest, Mem0HttpResponse, Mem0Transport};

// Self-hosted mem0 OSS REST paths (no `/v1/` prefix; the hosted cloud used
// `/v1/memories/…`). `add` and `list` share `/memories`; search is `/search`.
const ADD_PATH: &str = "/memories";
const SEARCH_PATH: &str = "/search";
const LIST_PATH: &str = "/memories";
const USER_ID_QUERY: &str = "user_id";
const APP_ID_QUERY: &str = "app_id";
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
            // Document-store semantics: store the content verbatim. `infer=false`
            // disables mem0's LLM fact-extraction so `read`/`profile_read` round-trip
            // the exact bytes and the all-local deployment needs only an embedder.
            "infer": false,
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

    /// Fold the workspace partition (`config.app_id`) into the mem0 `user_id`
    /// namespace.
    ///
    /// CRITICAL ISOLATION INVARIANT: the self-hosted mem0 OSS server enforces
    /// search/`get_all` filtering by `user_id` (and `agent_id`), but NOT by
    /// `app_id` — a top-level `app_id` is accepted and stored, yet silently
    /// ignored when filtering (verified empirically: a cross-`app_id` query
    /// returns the other partition's rows). Rather than depend on which secondary
    /// identifiers a given mem0 version actually enforces, the provider encodes
    /// the ENTIRE partition — full scope AND workspace — into the one key that is
    /// guaranteed enforced: `user_id`. So the workspace partition the local-dev
    /// composition passes as `config.app_id` is prefixed into the `user_id`
    /// namespace here; that prefix IS the isolation boundary. (`app_id` is still
    /// stamped via `stamp_app_id` as forward-compatible metadata, but MUST NOT be
    /// relied on for isolation.) Production leaves `app_id` unset → no prefix →
    /// pure scope namespace, so memory persists across restarts for the same scope.
    fn with_workspace_prefix(&self, base: String) -> String {
        match self.config.app_id.as_deref() {
            Some(prefix) => format!("{prefix}/{base}"),
            None => base,
        }
    }

    /// Full-scope memory namespace (tenant/user/agent/project) + workspace prefix.
    fn scoped_namespace(&self, scope: &ResourceScope) -> String {
        self.with_workspace_prefix(scope_namespace(scope))
    }

    /// Owner-only (profile) namespace (tenant/user) + workspace prefix.
    fn owner_scoped_namespace(&self, scope: &ResourceScope) -> String {
        self.with_workspace_prefix(owner_namespace(scope))
    }

    /// Build the query-string parameters for a GET `/memories` (list) request.
    ///
    /// Always includes `user_id=<namespace>`. When `self.config.app_id` is set,
    /// also appends `app_id=<value>` so that list-based operations (read, tree,
    /// profile_read, and the profile_set read-step) scope to the same app_id
    /// partition that search and write already use via [`Self::stamp_app_id`].
    fn list_query(&self, namespace: String) -> Vec<(String, String)> {
        let mut query = vec![(USER_ID_QUERY.to_string(), namespace)];
        if let Some(app_id) = self.config.app_id.as_deref() {
            query.push((APP_ID_QUERY.to_string(), app_id.to_string()));
        }
        query
    }

    /// Fetch the latest `kind=profile` memory for `namespace` and parse it as a
    /// JSON object, for the read step of `profile_set`'s read-merge-write.
    ///
    /// Returns an empty object only when there is no prior profile at all (the
    /// first write starts from empty). A backend/list failure, or a prior profile
    /// blob that does not parse as a JSON object, surfaces as an error — failing
    /// loud rather than silently treating the unavailable/corrupt state as empty,
    /// which would drop every previously-stored field on the merge-write.
    async fn fetch_latest_profile_object(
        &self,
        namespace: &str,
    ) -> Result<Map<String, Value>, MemoryServiceError> {
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(
                LIST_PATH,
                self.list_query(namespace.to_string()),
            ))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "profile_set").map_err(MemoryServiceError::operation_from)?;
        let items = response_items(&response.body).map_err(MemoryServiceError::operation_from)?;
        match latest_profile_text(&items) {
            // No prior profile memory at all: the first write starts from empty.
            None => Ok(Map::new()),
            // A prior profile exists but does not parse as a JSON object. Fail
            // loud: treating it as empty would drop every previously-stored field
            // on the next merge-write.
            Some(text) => serde_json::from_str::<Map<String, Value>>(text).map_err(|error| {
                MemoryServiceError::operation_from(Mem0Error::CorruptProfile {
                    reason: error.to_string(),
                })
            }),
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
        let namespace = self.scoped_namespace(&invocation.scope);
        let body = self.search_body(&namespace, &request.query, request.limit);
        let response = self
            .transport
            .execute(Mem0HttpRequest::post(SEARCH_PATH, body))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "search").map_err(MemoryServiceError::operation_from)?;
        let results = response_items(&response.body)
            .map_err(MemoryServiceError::operation_from)?
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
        // MAPPING GAP: mem0 OSS is append-only — every `add` appends a new memory,
        // and there is no addressable document to overwrite. A replace-style write
        // (`append == false`, no patch) cannot be honored, so reject it explicitly
        // as a stable, recoverable "unsupported" operation rather than silently
        // adding a memory and then misreporting `append: true` back to the caller.
        if !request.append {
            return Err(MemoryServiceError::operation_from(Mem0Error::Unsupported {
                operation: "write.replace",
                detail: "mem0 is append-only; replace-style writes are not supported",
            }));
        }
        // MAPPING GAP: an empty write is the native `bootstrap` clear, which has
        // no mem0 analogue (no addressable document to truncate). Treat it as an
        // invalid request, matching the native provider's empty-content rule.
        if request.content.trim().is_empty() {
            return Err(MemoryServiceError::input());
        }
        let namespace = self.scoped_namespace(&invocation.scope);
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
        let namespace = self.scoped_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(LIST_PATH, self.list_query(namespace)))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "read").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: mem0 is not path-addressable. Reconstruct a "document" by
        // concatenating every memory tagged with the requested `target`. mem0's
        // list order is NOT chronological (mem0/Qdrant `get_all` makes no ordering
        // guarantee — the same caveat the profile path documents), so sort the
        // fragments by `created_at` before joining; otherwise an append-style
        // document reads back scrambled. `sort_by` is stable, so fragments that
        // share (or lack) a `created_at` keep their relative list order.
        let mut fragments: Vec<&Value> = response_items(&response.body)
            .map_err(MemoryServiceError::operation_from)?
            .into_iter()
            .filter(|item| item_metadata_str(item, TARGET_KEY) == Some(request.path.as_str()))
            .collect();
        fragments.sort_by(|left, right| item_created_at(left).cmp(item_created_at(right)));
        let parts: Vec<String> = fragments
            .into_iter()
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
        let namespace = self.scoped_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(LIST_PATH, self.list_query(namespace)))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "tree").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: mem0 has no document hierarchy. Best-effort: surface the
        // distinct `target` tags (optionally prefix-filtered) as a flat list.
        let mut targets = std::collections::BTreeSet::new();
        for item in response_items(&response.body).map_err(MemoryServiceError::operation_from)? {
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
        // Field-preserving read-merge-write, matching the native provider's
        // read-modify-write of `context/profile.json` (rather than last-writer-wins
        // dropping unspecified fields): read the latest profile blob, merge the
        // supplied fields over it, and write the union as a new `kind=profile`
        // memory (`infer=false`, so the JSON round-trips verbatim). mem0 is
        // additive, so the newest memory carries the merged object and
        // `profile_read` (which returns the latest) sees every preserved field.
        // mem0 has no compare-and-set, so concurrent writers still race; this
        // closes the unconditional-drop gap, not the CAS gap.
        let namespace = self.owner_scoped_namespace(&invocation.scope);
        let mut merged = self.fetch_latest_profile_object(&namespace).await?;
        for (key, value) in request.fields {
            merged.insert(key, value);
        }
        let serialized = Value::Object(merged).to_string();
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
        let namespace = self.owner_scoped_namespace(&invocation.scope);
        let response = self
            .transport
            .execute(Mem0HttpRequest::get(LIST_PATH, self.list_query(namespace)))
            .await
            .map_err(MemoryServiceError::operation_from)?;
        ensure_success(&response, "profile_read").map_err(MemoryServiceError::operation_from)?;
        // MAPPING GAP: return the newest `kind=profile` memory's raw bytes, if any
        // (newest by `created_at`, since mem0 list order is not chronological). The
        // host parses + size-caps them, exactly as for native.
        let items = response_items(&response.body).map_err(MemoryServiceError::operation_from)?;
        let document = latest_profile_text(&items).map(|text| text.as_bytes().to_vec());
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
        let namespace = self.scoped_namespace(&invocation.scope);
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
            .map_err(MemoryServiceError::unavailable_from)?
            .into_iter()
            // The `kind=profile` record `profile_set` writes shares the owner
            // namespace (agent/project-less scopes collapse to it), and search
            // can return it. Profile state has its own read path
            // (`profile_read`); raw profile JSON must not enter the prompt as
            // a memory snippet.
            .filter(|item| item_metadata_str(item, KIND_KEY) != Some(PROFILE_KIND))
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
///
/// A 2xx body in *none* of those shapes is a contract violation, not "no
/// memories": fail loud with [`Mem0Error::UnrecognizedResponse`] rather than
/// returning an empty vec. A silent empty would let callers that branch on "no
/// items" — notably `profile_set`'s read-merge-write — mistake an unrecognized
/// body for "no prior data" and overwrite every previously-stored field. A
/// genuinely-empty list is a recognized empty array (or `{"results": []}`) and
/// still returns `Ok(vec![])`.
fn response_items(body: &Value) -> Result<Vec<&Value>, Mem0Error> {
    if let Some(array) = body.as_array() {
        return Ok(array.iter().collect());
    }
    for key in ["results", "memories", "data"] {
        if let Some(array) = body.get(key).and_then(Value::as_array) {
            return Ok(array.iter().collect());
        }
    }
    Err(Mem0Error::UnrecognizedResponse)
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

/// mem0 returns an ISO-8601 `created_at` on each memory. Used to pick the newest
/// profile memory deterministically: mem0/Qdrant `get_all` does not guarantee
/// chronological order, so a `read-merge-write` must not assume "last in list".
fn item_created_at(item: &Value) -> &str {
    item.get("created_at").and_then(Value::as_str).unwrap_or("")
}

/// The text of the newest `kind=profile` memory in a list response, chosen by max
/// `created_at` (ISO-8601 sorts lexicographically). With timestamps absent (e.g.
/// unit-test fixtures) all keys tie and `max_by` returns the last element, which
/// preserves the prior "last memory wins" fallback.
fn latest_profile_text<'a>(items: &[&'a Value]) -> Option<&'a str> {
    items
        .iter()
        .filter(|item| item_metadata_str(item, KIND_KEY) == Some(PROFILE_KIND))
        .max_by(|left, right| item_created_at(left).cmp(item_created_at(right)))
        .and_then(|item| item_text(item))
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
        // Document-store writes are verbatim: mem0's LLM fact-extraction is off.
        assert_eq!(body["infer"], json!(false));
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
    async fn write_replace_is_rejected() {
        // mem0 OSS is append-only, so a replace-style write (`append == false`,
        // with no patch) cannot be honored. It must be rejected as a stable,
        // recoverable operation error — never silently turned into an add that
        // then misreports `append: true`.
        let (service, transport) =
            service_with(MockMem0Transport::always_ok(json!({ "id": "m-1" })));
        let error = service
            .write(
                invocation(),
                MemoryServiceWriteRequest {
                    target: "notes/alpha.md".to_string(),
                    content: "replace me".to_string(),
                    append: false,
                    old_string: None,
                    new_string: None,
                    replace_all: false,
                    metadata: None,
                    timezone: None,
                },
            )
            .await
            .expect_err("a replace-style write must be rejected");
        assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
        // The rejection happens before any add reaches the transport: no memory is
        // silently appended on a replace request.
        assert!(
            transport.recorded().is_empty(),
            "a rejected replace must not POST an add"
        );
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
    async fn read_joins_fragments_in_created_at_order_not_list_order() {
        // mem0/Qdrant list order is not chronological, so a `target`'s fragments
        // arrive out of order (newest in the MIDDLE here). `read` must join them by
        // `created_at` (oldest first) so an append-style document round-trips in the
        // order it was written, not in arbitrary list order.
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            {
                "memory": "second",
                "created_at": "2026-02-01T00:00:00Z",
                "metadata": { "target": "notes/log.md" }
            },
            {
                "memory": "third",
                "created_at": "2026-03-01T00:00:00Z",
                "metadata": { "target": "notes/log.md" }
            },
            {
                "memory": "first",
                "created_at": "2026-01-01T00:00:00Z",
                "metadata": { "target": "notes/log.md" }
            }
        ])));
        let read = service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/log.md".to_string(),
                },
            )
            .await
            .expect("read should succeed");
        assert_eq!(read.content, "first\nsecond\nthird");
    }

    #[tokio::test]
    async fn retrieve_context_maps_search_hits_to_snippets() {
        // The kind=profile record shares the owner namespace with memories, so
        // search can return it; retrieve_context must drop it (profile state
        // has its own read path and must not enter the prompt as a snippet).
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!({
            "results": [
                { "id": "m-1", "memory": "ctx hit", "metadata": { "target": "notes/a.md" } },
                { "id": "m-2", "memory": "{\"timezone\":\"UTC\"}", "metadata": { "kind": "profile" } }
            ]
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
        assert_eq!(
            snippets.len(),
            1,
            "the kind=profile record must be filtered out of context snippets"
        );
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
    async fn profile_set_merges_over_existing_fields_instead_of_dropping_them() {
        // The existing profile has two fields; the caller sets only one. A
        // field-preserving read-merge-write must keep the untouched field and
        // update the supplied one — never last-writer-wins drop `language`.
        let transport = Arc::new(MockMem0Transport::new(Box::new(|request| {
            match request.method {
                // The read step lists the current profile memory.
                Mem0HttpMethod::Get => Some(Mem0HttpResponse {
                    status: 200,
                    body: json!([{
                        "memory": "{\"timezone\":\"UTC\",\"language\":\"en\"}",
                        "metadata": { "kind": "profile", "target": "context/profile.json" }
                    }]),
                }),
                // The write step (the add) just succeeds.
                Mem0HttpMethod::Post => Some(Mem0HttpResponse {
                    status: 200,
                    body: json!({ "results": [{ "id": "p-1", "event": "ADD" }] }),
                }),
            }
        })));
        let service = Mem0MemoryService::new(
            Arc::clone(&transport) as Arc<dyn Mem0Transport>,
            Mem0Config::new(),
        );

        let mut fields = Map::new();
        fields.insert("timezone".to_string(), json!("PST"));
        service
            .profile_set(invocation(), MemoryServiceProfileSetRequest { fields })
            .await
            .expect("profile_set should succeed");

        // The add carried the *merged* object: the untouched `language` is kept
        // and `timezone` is updated. Parse the body so field order is irrelevant.
        let posts: Vec<_> = transport
            .recorded()
            .into_iter()
            .filter(|request| request.method == Mem0HttpMethod::Post)
            .collect();
        assert_eq!(posts.len(), 1, "exactly one add (the merged write)");
        let written = posts[0].body.as_ref().expect("add body");
        let content = written["messages"][0]["content"]
            .as_str()
            .expect("serialized profile string");
        let merged: serde_json::Value =
            serde_json::from_str(content).expect("merged profile is valid JSON");
        assert_eq!(merged["timezone"], json!("PST"), "supplied field updated");
        assert_eq!(merged["language"], json!("en"), "untouched field preserved");
        // And the write is verbatim so the JSON round-trips for profile_read.
        assert_eq!(written["infer"], json!(false));
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

    #[tokio::test]
    async fn profile_read_returns_newest_profile_by_created_at_not_list_order() {
        // mem0/Qdrant `get_all` order is not chronological, so the newest profile
        // is deliberately placed in the MIDDLE of the list. profile_read must pick
        // the max-`created_at` blob, not the first or last in list order — a
        // `min_by` or a dropped comparator would return a different element.
        let (service, _transport) = service_with(MockMem0Transport::always_ok(json!([
            {
                "memory": "{\"v\":\"middle\"}",
                "created_at": "2026-02-15T00:00:00Z",
                "metadata": { "kind": "profile", "target": "context/profile.json" }
            },
            {
                "memory": "{\"v\":\"newest\"}",
                "created_at": "2026-06-01T00:00:00Z",
                "metadata": { "kind": "profile", "target": "context/profile.json" }
            },
            {
                "memory": "{\"v\":\"oldest\"}",
                "created_at": "2026-01-01T00:00:00Z",
                "metadata": { "kind": "profile", "target": "context/profile.json" }
            }
        ])));
        let read = service
            .profile_read(invocation())
            .await
            .expect("profile_read should succeed");
        assert_eq!(read.document, Some(b"{\"v\":\"newest\"}".to_vec()));
    }

    #[tokio::test]
    async fn profile_set_fails_loud_on_a_corrupt_existing_profile() {
        // A prior profile memory exists but its stored blob is not a JSON object.
        // profile_set must NOT silently treat it as empty (which would drop every
        // previously-stored field); it must surface an Operation error and never
        // issue the merge-write.
        let transport = Arc::new(MockMem0Transport::new(Box::new(|request| {
            match request.method {
                Mem0HttpMethod::Get => Some(Mem0HttpResponse {
                    status: 200,
                    body: json!([{
                        "memory": "this is not json",
                        "metadata": { "kind": "profile", "target": "context/profile.json" }
                    }]),
                }),
                Mem0HttpMethod::Post => Some(Mem0HttpResponse {
                    status: 200,
                    body: json!({ "results": [{ "id": "p-1", "event": "ADD" }] }),
                }),
            }
        })));
        let service = Mem0MemoryService::new(
            Arc::clone(&transport) as Arc<dyn Mem0Transport>,
            Mem0Config::new(),
        );

        let mut fields = Map::new();
        fields.insert("timezone".to_string(), json!("PST"));
        let error = service
            .profile_set(invocation(), MemoryServiceProfileSetRequest { fields })
            .await
            .expect_err("a corrupt existing profile must fail loud, not erase fields");
        assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);

        // The corrupt read aborted before any add: no field-erasing write occurred.
        let posts = transport
            .recorded()
            .into_iter()
            .filter(|request| request.method == Mem0HttpMethod::Post)
            .count();
        assert_eq!(posts, 0, "must not write when the prior profile is corrupt");
    }

    /// Build a service with a specific `app_id` configured (for isolation tests).
    fn service_with_app_id(
        transport: MockMem0Transport,
        app_id: &str,
    ) -> (Mem0MemoryService, Arc<MockMem0Transport>) {
        let transport = Arc::new(transport);
        let service = Mem0MemoryService::new(
            Arc::clone(&transport) as Arc<dyn Mem0Transport>,
            Mem0Config::new().with_app_id(app_id),
        );
        (service, transport)
    }

    #[tokio::test]
    async fn list_query_includes_app_id_when_configured() {
        // When a service has `app_id` set, every GET /memories list request
        // (used by `read`, `tree`, `profile_read`, and the profile_set read-step)
        // must carry `app_id` as a query param — matching how `search` and `write`
        // already scope via the request body `stamp_app_id`. Without this,
        // list-based operations leak across app partitions.
        let (service, transport) = service_with_app_id(
            MockMem0Transport::always_ok(json!([
                { "memory": "isolated", "metadata": { "target": "notes/a.md" } }
            ])),
            "ws-abc123",
        );
        service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/a.md".to_string(),
                },
            )
            .await
            .expect("read should succeed");

        let recorded = transport.recorded();
        assert_eq!(recorded.len(), 1);
        let request = &recorded[0];
        assert_eq!(request.method, Mem0HttpMethod::Get);
        // The query must contain both user_id and app_id.
        let has_user_id = request.query.iter().any(|(key, _)| key == USER_ID_QUERY);
        let app_id_value = request
            .query
            .iter()
            .find(|(key, _)| key == APP_ID_QUERY)
            .map(|(_, value)| value.as_str());
        assert!(has_user_id, "user_id must always be present in list query");
        assert_eq!(
            app_id_value,
            Some("ws-abc123"),
            "app_id must appear in list query when configured"
        );
    }

    #[tokio::test]
    async fn workspace_is_prefixed_into_the_user_id_namespace() {
        // CRITICAL no-leak guard. mem0 OSS enforces filtering by `user_id`, NOT
        // `app_id` (verified empirically), so the workspace partition the
        // composition passes as `app_id` MUST be folded into the `user_id`
        // namespace — that prefix IS the isolation boundary. A regression that
        // drops it silently re-opens cross-workspace leakage (two workspaces would
        // share one `user_id` partition). Every op routes through
        // `scoped_namespace`/`owner_scoped_namespace`, so checking one list op
        // proves the shared `with_workspace_prefix` mechanism.
        let (service, transport) = service_with_app_id(
            MockMem0Transport::always_ok(json!([
                { "memory": "x", "metadata": { "target": "notes/a.md" } }
            ])),
            "ws-deadbeef",
        );
        service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/a.md".to_string(),
                },
            )
            .await
            .expect("read should succeed");

        let recorded = transport.recorded();
        let user_id = recorded[0]
            .query
            .iter()
            .find(|(key, _)| key == USER_ID_QUERY)
            .map(|(_, value)| value.as_str())
            .expect("list query must carry user_id");
        assert!(
            user_id.starts_with("ws-deadbeef/"),
            "workspace MUST prefix the user_id namespace (the enforced isolation \
             boundary), got {user_id:?}"
        );
        assert!(
            user_id.contains("tenant-mem0/user-mem0"),
            "the logical scope must remain in the namespace after the prefix, got {user_id:?}"
        );
    }

    #[tokio::test]
    async fn list_query_omits_app_id_when_not_configured() {
        // When no `app_id` is set (the default / production path where scope is
        // handled only by `user_id`), GET /memories must NOT add an `app_id`
        // query param — consistent with how `stamp_app_id` behaves for POST bodies.
        let (service, transport) = service_with(MockMem0Transport::always_ok(json!([
            { "memory": "m", "metadata": { "target": "notes/a.md" } }
        ])));
        service
            .read(
                invocation(),
                MemoryServiceReadRequest {
                    path: "notes/a.md".to_string(),
                },
            )
            .await
            .expect("read should succeed");

        let recorded = transport.recorded();
        let request = &recorded[0];
        let has_app_id = request.query.iter().any(|(key, _)| key == APP_ID_QUERY);
        assert!(
            !has_app_id,
            "app_id must NOT appear in list query when not configured"
        );
    }

    #[tokio::test]
    async fn list_path_fails_loud_on_an_unrecognized_response_body() {
        // A 2xx body that is neither a bare array nor an object wrapping
        // `results`/`memories`/`data` is a contract violation. The list path
        // (here via `profile_read`) must surface an error rather than silently
        // returning "no memories" (an empty document) — a silent empty would let
        // a later `profile_set` overwrite an existing profile it merely failed to
        // decode.
        let (service, _transport) = service_with(MockMem0Transport::always_ok(
            json!({ "unexpected": "shape" }),
        ));
        let error = service
            .profile_read(invocation())
            .await
            .expect_err("an unrecognized list body must fail loud, not return empty");
        assert_eq!(error.kind(), MemoryServiceErrorKind::Operation);
    }
}
