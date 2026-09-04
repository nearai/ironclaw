//! The lane → tool mapping.
//!
//! | IronClaw lane | Tool | Mapping fidelity |
//! |---|---|---|
//! | `read_long_term` | [`McpMemoryConfig::search_tool`] | Clean: query + bounded limit in, ranked candidates out. |
//! | `record_interaction` | [`McpMemoryConfig::record_tool`] | Clean: the raw message list plus provenance metadata. |
//! | `read_short_term` | — | Not mapped. The manifest must not declare the hook. |
//! | `profile_read` | — | Not mapped. The manifest must not declare the hook. |
//!
//! The two unmapped lanes fall through to the trait defaults. That is a real
//! shape, not a gap to apologize for: the host only calls the lifecycle hooks a
//! provider's manifest declares, so a provider ships the lanes it actually
//! implements and the rest are never invoked.
//!
//! ## Scope is host-supplied, always
//!
//! Every tool call carries the identity components from the TRUSTED
//! [`ResourceScope`] on the invocation. Nothing in an argument payload is
//! derived from model-generated text. A memory server therefore cannot be
//! steered across tenants by prompt content — the worst a compromised model can
//! do is choose the query string.
//!
//! ## The provider shapes nothing the model sees
//!
//! `read_long_term` returns RAW candidate text. The host owns cross-scope
//! filtering, control-character stripping, truncation, the untrusted-memory
//! envelope, the prompt-safety denylist, and every model-visible byte budget.
//! That division is what makes accepting a third-party memory backend
//! defensible at all, so this crate must never "helpfully" pre-sanitize.

use async_trait::async_trait;
use ironclaw_host_api::resource::ResourceScope;
use ironclaw_memory::{
    MemoryInvocation, MemoryService, MemoryServiceContextRequest, MemoryServiceContextSnippet,
    MemoryServiceError, MemoryServiceRecordRequest, MemoryServiceRecordResponse,
    memory_context_disabled,
};
use serde_json::{Map, Value, json};

use crate::config::McpMemoryConfig;
use crate::transport::{McpMemoryToolCall, McpMemoryTransport, McpMemoryTransportError};

/// Hard ceiling on candidates accepted from one search response, independent of
/// what the host asked for.
///
/// The host applies its own budgets afterwards, but a remote provider is
/// untrusted input: it can answer a `limit: 4` request with ten thousand rows.
/// Bounding here keeps a hostile or broken server from turning one prompt build
/// into an allocation spike before host budgeting ever runs.
const MAX_ACCEPTED_CANDIDATES: usize = 256;

/// Maximum bytes accepted for a single candidate's text, before host
/// truncation. Same rationale as [`MAX_ACCEPTED_CANDIDATES`]: bound the input,
/// then let the host's much smaller model-visible budget do the real shaping.
const MAX_ACCEPTED_CANDIDATE_BYTES: usize = 64 * 1024;

/// A memory provider that speaks the memory-over-MCP tool contract.
pub struct McpMemoryService<T> {
    transport: T,
    config: McpMemoryConfig,
}

impl<T> McpMemoryService<T> {
    pub fn new(transport: T, config: McpMemoryConfig) -> Self {
        Self { transport, config }
    }

    /// The trusted identity every tool call carries.
    fn scope_arguments(scope: &ResourceScope) -> Map<String, Value> {
        let mut arguments = Map::new();
        arguments.insert(
            "tenant_id".to_string(),
            Value::String(scope.tenant_id.as_str().to_string()),
        );
        arguments.insert(
            "user_id".to_string(),
            Value::String(scope.user_id.as_str().to_string()),
        );
        if let Some(agent_id) = scope.agent_id.as_ref() {
            arguments.insert(
                "agent_id".to_string(),
                Value::String(agent_id.as_str().to_string()),
            );
        }
        if let Some(project_id) = scope.project_id.as_ref() {
            arguments.insert(
                "project_id".to_string(),
                Value::String(project_id.as_str().to_string()),
            );
        }
        arguments
    }
}

impl<T: McpMemoryTransport> McpMemoryService<T> {
    async fn call(&self, tool: &str, arguments: Value) -> Result<Value, MemoryServiceError> {
        self.transport
            .call_tool(McpMemoryToolCall {
                tool: tool.to_string(),
                arguments,
            })
            .await
            .map_err(map_transport_error)
    }
}

#[async_trait]
impl<T: McpMemoryTransport> MemoryService for McpMemoryService<T> {
    async fn read_long_term(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        if request.max_snippets == 0 || memory_context_disabled(request.context_profile_id.as_str())
        {
            return Ok(Vec::new());
        }
        let scope = &invocation.scope;
        let mut arguments = Self::scope_arguments(scope);
        arguments.insert("query".to_string(), Value::String(request.query.clone()));
        arguments.insert("limit".to_string(), json!(request.max_snippets));
        // The long-term lane must exclude per-thread scratch even when the
        // invocation carries a thread, so the two lanes stay disjoint when the
        // host concatenates them. Say so explicitly rather than relying on the
        // absence of `thread_id` to mean it.
        arguments.insert("scope".to_string(), Value::String("long_term".to_string()));

        let response = self
            .call(&self.config.search_tool, Value::Object(arguments))
            .await?;

        let candidates = response_candidates(&response);
        Ok(candidates
            .into_iter()
            .take(request.max_snippets.min(MAX_ACCEPTED_CANDIDATES))
            .filter_map(|candidate| {
                let text = candidate_text(candidate)?;
                if text.len() > MAX_ACCEPTED_CANDIDATE_BYTES {
                    tracing::debug!(
                        bytes = text.len(),
                        "dropping oversized mcp memory candidate before host budgeting"
                    );
                    return None;
                }
                Some(MemoryServiceContextSnippet {
                    tenant_id: scope.tenant_id.as_str().to_string(),
                    user_id: scope.user_id.as_str().to_string(),
                    agent_id: scope.agent_id.as_ref().map(|id| id.as_str().to_string()),
                    project_id: scope.project_id.as_ref().map(|id| id.as_str().to_string()),
                    relative_path: candidate_path(candidate),
                    text: text.to_string(),
                })
            })
            .collect())
    }

    async fn record_interaction(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceRecordRequest,
    ) -> Result<MemoryServiceRecordResponse, MemoryServiceError> {
        let Some(record_tool) = self.config.record_tool.as_ref() else {
            return Ok(MemoryServiceRecordResponse { recorded: false });
        };
        if request.messages.is_empty() {
            return Ok(MemoryServiceRecordResponse { recorded: false });
        }
        let scope = &invocation.scope;
        let mut arguments = Self::scope_arguments(scope);
        // The active conversation, when there is one: a server that partitions
        // sessions needs it, and it is the only thread identity it may use.
        if let Some(thread_id) = scope.thread_id.as_ref() {
            arguments.insert(
                "thread_id".to_string(),
                Value::String(thread_id.as_str().to_string()),
            );
        }
        arguments.insert(
            "messages".to_string(),
            serde_json::to_value(&request.messages).map_err(MemoryServiceError::operation_from)?,
        );
        arguments.insert("metadata".to_string(), request.metadata.clone());
        if let Some(turn_run_id) = request.turn_run_id.as_ref() {
            arguments.insert(
                "turn_run_id".to_string(),
                Value::String(turn_run_id.clone()),
            );
        }
        // Correlation doubles as the idempotency key: a retried turn must not
        // record the same interaction twice on the provider side.
        arguments.insert(
            "idempotency_key".to_string(),
            Value::String(invocation.correlation_id.to_string()),
        );

        let response = self.call(record_tool, Value::Object(arguments)).await?;
        Ok(MemoryServiceRecordResponse {
            recorded: recorded_flag(&response),
        })
    }
}

/// Every transport failure degrades the lane rather than failing the turn.
///
/// The host turns this into a typed retrieval degradation and one
/// operator-visible note, so a down memory server is diagnosable instead of
/// looking exactly like a user with nothing stored.
fn map_transport_error(error: McpMemoryTransportError) -> MemoryServiceError {
    MemoryServiceError::unavailable_from(error)
}

/// Accept the shapes a memory server may reasonably use for a result list:
/// a bare array, or a `results` / `memories` / `content` envelope.
///
/// Tolerant on purpose — this is a cross-vendor contract, and rejecting a
/// response because it wrapped its rows differently would make the contract
/// harder to satisfy without making it safer.
fn response_candidates(response: &Value) -> Vec<&Value> {
    for key in ["results", "memories", "content", "data"] {
        if let Some(Value::Array(items)) = response.get(key) {
            return items.iter().collect();
        }
    }
    match response {
        Value::Array(items) => items.iter().collect(),
        _ => Vec::new(),
    }
}

/// The candidate's raw body, under any of the field names the contract accepts.
fn candidate_text(candidate: &Value) -> Option<&str> {
    for key in ["text", "memory", "content", "value"] {
        if let Some(Value::String(text)) = candidate.get(key) {
            return Some(text.as_str());
        }
    }
    candidate.as_str()
}

/// A stable display path for the candidate. Falls back to the record id, then to
/// a constant, so the host always has something to hash into its snippet
/// reference.
fn candidate_path(candidate: &Value) -> String {
    for key in ["path", "relative_path", "source", "id"] {
        if let Some(Value::String(path)) = candidate.get(key) {
            return path.clone();
        }
    }
    "memory".to_string()
}

/// Whether the server says it durably recorded the interaction.
///
/// Absent means "yes": a server that returns a bare success payload with no
/// explicit flag has still accepted the write. Only an explicit `false`
/// (or an explicit `recorded: false`) reports a no-op.
fn recorded_flag(response: &Value) -> bool {
    for key in ["recorded", "ok", "success"] {
        if let Some(Value::Bool(flag)) = response.get(key) {
            return *flag;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::ids::{CorrelationId, TenantId, UserId};
    use ironclaw_memory::{
        MemoryContextProfileId, MemoryInteractionMessage, MemoryInteractionRole,
    };

    use super::*;
    use crate::transport::MockMcpMemoryTransport;

    fn scope() -> ResourceScope {
        let mut scope = ResourceScope::local_default(
            UserId::new("user-a").expect("user id"),
            ironclaw_host_api::ids::InvocationId::new(),
        )
        .expect("local default scope");
        scope.tenant_id = TenantId::new("tenant-a").expect("tenant id");
        scope
    }

    fn invocation() -> MemoryInvocation {
        MemoryInvocation {
            scope: scope(),
            correlation_id: CorrelationId::new(),
        }
    }

    fn context_request(max_snippets: usize) -> MemoryServiceContextRequest {
        MemoryServiceContextRequest {
            query: "when is the standup".to_string(),
            max_snippets,
            context_profile_id: MemoryContextProfileId::new("default").expect("profile id"),
        }
    }

    /// The identity a memory server sees comes from the trusted scope, and the
    /// query is the ONLY model-influenced field. A server that could read a
    /// tenant out of prompt text would be a cross-tenant steering primitive.
    #[tokio::test]
    async fn tool_arguments_carry_trusted_scope_and_bounded_limit() {
        let transport = MockMcpMemoryTransport::always_ok(json!({ "results": [] }));
        let service = McpMemoryService::new(transport, McpMemoryConfig::new());

        service
            .read_long_term(invocation(), context_request(4))
            .await
            .expect("retrieval succeeds");

        let calls = service.transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, DEFAULT_SEARCH_TOOL_FOR_TEST);
        assert_eq!(calls[0].arguments["tenant_id"], json!("tenant-a"));
        assert_eq!(calls[0].arguments["user_id"], json!("user-a"));
        assert_eq!(calls[0].arguments["limit"], json!(4));
        assert_eq!(calls[0].arguments["query"], json!("when is the standup"));
        assert_eq!(calls[0].arguments["scope"], json!("long_term"));
    }

    const DEFAULT_SEARCH_TOOL_FOR_TEST: &str = crate::config::DEFAULT_SEARCH_TOOL;

    /// A vendor that names its tools differently binds by config, not by code.
    /// This is the property the whole crate exists for.
    #[tokio::test]
    async fn tool_names_come_from_configuration() {
        let transport = MockMcpMemoryTransport::always_ok(json!({ "results": [] }));
        let service = McpMemoryService::new(
            transport,
            McpMemoryConfig::new().with_search_tool("recall_v2"),
        );

        service
            .read_long_term(invocation(), context_request(2))
            .await
            .expect("retrieval succeeds");

        assert_eq!(service.transport.calls()[0].tool, "recall_v2");
    }

    /// Cross-vendor tolerance: rows may arrive bare or under any of the
    /// accepted envelopes, and the body under any of the accepted field names.
    #[tokio::test]
    async fn response_envelopes_and_field_names_are_tolerated() {
        for body in [
            json!({ "results": [{ "text": "standup is thursday", "path": "notes/a.md" }] }),
            json!({ "memories": [{ "memory": "standup is thursday", "id": "notes/a.md" }] }),
            json!([{ "content": "standup is thursday", "source": "notes/a.md" }]),
        ] {
            let service = McpMemoryService::new(
                MockMcpMemoryTransport::always_ok(body.clone()),
                McpMemoryConfig::new(),
            );
            let snippets = service
                .read_long_term(invocation(), context_request(4))
                .await
                .expect("retrieval succeeds");
            assert_eq!(snippets.len(), 1, "envelope not tolerated: {body}");
            assert_eq!(snippets[0].text, "standup is thursday");
            assert_eq!(snippets[0].relative_path, "notes/a.md");
            assert_eq!(snippets[0].tenant_id, "tenant-a");
        }
    }

    /// A remote provider is untrusted input: it may answer a bounded request
    /// with far more rows than were asked for.
    #[tokio::test]
    async fn oversized_result_sets_are_bounded_to_the_request() {
        let rows: Vec<Value> = (0..1000)
            .map(|index| json!({ "text": format!("row {index}") }))
            .collect();
        let service = McpMemoryService::new(
            MockMcpMemoryTransport::always_ok(json!({ "results": rows })),
            McpMemoryConfig::new(),
        );

        let snippets = service
            .read_long_term(invocation(), context_request(4))
            .await
            .expect("retrieval succeeds");

        assert_eq!(snippets.len(), 4);
    }

    /// A down memory server degrades the lane; it never fails the turn. The
    /// host turns `Unavailable` into the operator-visible degradation note, so
    /// this variant is what keeps "broken" distinguishable from "empty".
    #[tokio::test]
    async fn transport_failure_degrades_rather_than_failing_the_turn() {
        let service = McpMemoryService::new(
            MockMcpMemoryTransport::always_err(McpMemoryTransportError::transport(
                "connect timeout",
            )),
            McpMemoryConfig::new(),
        );

        let error = service
            .read_long_term(invocation(), context_request(4))
            .await
            .expect_err("a down server must surface as an error, not as empty");

        assert_eq!(
            error.kind(),
            ironclaw_memory::MemoryServiceErrorKind::Unavailable
        );
    }

    /// A disabled memory-context profile must not reach the network at all.
    #[tokio::test]
    async fn disabled_context_profile_makes_no_call() {
        let service = McpMemoryService::new(
            MockMcpMemoryTransport::always_ok(json!({ "results": [] })),
            McpMemoryConfig::new(),
        );

        let snippets = service
            .read_long_term(
                invocation(),
                MemoryServiceContextRequest {
                    query: "anything".to_string(),
                    max_snippets: 4,
                    context_profile_id: MemoryContextProfileId::new("memory_disabled")
                        .expect("profile id"),
                },
            )
            .await
            .expect("disabled profile yields empty");

        assert!(snippets.is_empty());
        assert!(service.transport.calls().is_empty());
    }

    /// A retried turn must not record the same interaction twice on the
    /// provider side, so the correlation id rides along as the idempotency key.
    #[tokio::test]
    async fn record_interaction_sends_an_idempotency_key() {
        let service = McpMemoryService::new(
            MockMcpMemoryTransport::always_ok(json!({ "recorded": true })),
            McpMemoryConfig::new(),
        );
        let invocation = invocation();
        let expected_key = invocation.correlation_id.to_string();

        let response = service
            .record_interaction(
                invocation,
                MemoryServiceRecordRequest {
                    messages: vec![MemoryInteractionMessage {
                        role: MemoryInteractionRole::User,
                        content: "remember the launch window".to_string(),
                        name: None,
                    }],
                    turn_run_id: Some("run-1".to_string()),
                    metadata: json!({ "source": "test" }),
                },
            )
            .await
            .expect("record succeeds");

        assert!(response.recorded);
        let calls = service.transport.calls();
        assert_eq!(calls[0].arguments["idempotency_key"], json!(expected_key));
        assert_eq!(calls[0].arguments["turn_run_id"], json!("run-1"));
    }

    /// A provider that declares no record tool reports a no-op rather than
    /// inventing a call.
    #[tokio::test]
    async fn without_a_record_tool_nothing_is_recorded() {
        let service = McpMemoryService::new(
            MockMcpMemoryTransport::always_ok(json!({})),
            McpMemoryConfig::new().without_record_tool(),
        );

        let response = service
            .record_interaction(
                invocation(),
                MemoryServiceRecordRequest {
                    messages: vec![MemoryInteractionMessage {
                        role: MemoryInteractionRole::User,
                        content: "anything".to_string(),
                        name: None,
                    }],
                    turn_run_id: None,
                    metadata: Value::Null,
                },
            )
            .await
            .expect("no-op record succeeds");

        assert!(!response.recorded);
        assert!(service.transport.calls().is_empty());
    }
}
