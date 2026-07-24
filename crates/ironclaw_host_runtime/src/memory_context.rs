//! Production [`MemoryPromptContextService`] adapter backed by IronClaw memory.
//!
//! This adapter bridges the Reborn memory service service into the agent loop
//! context pipeline. It derives the host-resolved IronClaw memory invocation
//! scope from the request's [`TurnScope`] and [`TurnActor`], then delegates
//! retrieval to [`MemoryService`]. The loop-facing adapter still owns final
//! model-context admission so future extension-backed memory cannot bypass
//! host prompt safety by returning already-shaped snippets.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{
    MemoryContextProfileId, MemoryInvocation, MemoryService, MemoryServiceContextRequest,
    MemoryServiceError, MemoryServiceErrorKind, memory_context_disabled,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopContextSnippet, LoopSafeSummary,
    MemoryPromptContextRequest, MemoryPromptContextService,
};

const MAX_MEMORY_CONTEXT_SNIPPET_BYTES: usize = 512;
const MAX_MEMORY_CONTEXT_TOTAL_BYTES: usize = 4 * 1024;
const MEMORY_CONTEXT_REF_PREFIX: &str = "memory-snippet:";
const MEMORY_CONTEXT_UNTRUSTED_PREFIX: &str = "Untrusted memory content:";

/// Production adapter that loads memory snippets through IronClaw memory.
pub struct ProductionMemoryPromptContextService {
    memory_service: Arc<dyn MemoryService>,
}

impl ProductionMemoryPromptContextService {
    /// Create a new production adapter wrapping the configured memory service
    /// service. Native memory remains the default service adapter in Phase 1.
    pub fn new(memory_service: Arc<dyn MemoryService>) -> Self {
        Self { memory_service }
    }
}

#[async_trait]
impl MemoryPromptContextService for ProductionMemoryPromptContextService {
    async fn load_memory_snippets(
        &self,
        request: MemoryPromptContextRequest,
    ) -> Result<Vec<LoopContextSnippet>, AgentLoopHostError> {
        if request.max_snippets == 0 {
            return Ok(Vec::new());
        }

        // Fail closed at the host before any provider call: a memory-disabled
        // profile returns no snippets without touching the memory service (the
        // native provider keeps an equivalent check as defense in depth).
        if memory_context_disabled(request.context_profile_id.as_str()) {
            return Ok(Vec::new());
        }
        let invocation = invocation_for_context_request(&request);
        // The host-resolved `ContextProfileId` is already validated, so this
        // construction won't fail in practice — but propagate rather than unwrap.
        let context_profile_id = MemoryContextProfileId::new(request.context_profile_id.as_str())
            .map_err(map_memory_service_error)?;
        let snippets = self
            .memory_service
            .retrieve_context(
                invocation,
                MemoryServiceContextRequest {
                    query: request.query,
                    max_snippets: request.max_snippets,
                    context_profile_id,
                },
            )
            .await
            .map_err(map_memory_service_error)?;

        let mut admitted = Vec::new();
        let mut total_bytes = 0usize;
        for snippet in snippets {
            if admitted.len() >= request.max_snippets {
                break;
            }
            let Some(snippet) = admit_memory_context_snippet(snippet, &mut total_bytes) else {
                continue;
            };
            admitted.push(snippet);
        }
        Ok(admitted)
    }
}

fn admit_memory_context_snippet(
    snippet: ironclaw_memory::MemoryServiceContextSnippet,
    total_bytes: &mut usize,
) -> Option<LoopContextSnippet> {
    if !snippet.snippet_ref.starts_with(MEMORY_CONTEXT_REF_PREFIX)
        || snippet.snippet_ref.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '{' | '}' | '[' | ']' | '`' | '<' | '>' | '/' | '\\'
                )
        })
    {
        tracing::debug!("dropping memory context snippet with invalid ref");
        return None;
    }
    if snippet.model_content != snippet.safe_summary
        || !snippet
            .model_content
            .starts_with(MEMORY_CONTEXT_UNTRUSTED_PREFIX)
    {
        tracing::debug!("dropping memory context snippet without host-accepted wrapper");
        return None;
    }
    if snippet.model_content.len() > MAX_MEMORY_CONTEXT_SNIPPET_BYTES {
        tracing::debug!("dropping oversized memory context snippet");
        return None;
    }
    let safe_summary = match LoopSafeSummary::new(snippet.safe_summary.clone()) {
        Ok(summary) => summary,
        Err(error) => {
            tracing::debug!(
                ?error,
                "dropping memory context snippet: safe_summary failed LoopSafeSummary validation"
            );
            return None;
        }
    };
    let model_content = match LoopSafeSummary::new(snippet.model_content) {
        Ok(content) => content,
        Err(error) => {
            tracing::debug!(
                ?error,
                "dropping memory context snippet: model_content failed LoopSafeSummary validation"
            );
            return None;
        }
    };
    let next_total = total_bytes.saturating_add(model_content.as_str().len());
    if next_total > MAX_MEMORY_CONTEXT_TOTAL_BYTES {
        tracing::debug!("dropping memory context snippet over aggregate budget");
        return None;
    }
    *total_bytes = next_total;
    Some(LoopContextSnippet {
        snippet_ref: snippet.snippet_ref,
        safe_summary: safe_summary.as_str().to_string(),
        model_content: model_content.as_str().to_string(),
        metadata: None,
    })
}

fn invocation_for_context_request(request: &MemoryPromptContextRequest) -> MemoryInvocation {
    MemoryInvocation {
        scope: ResourceScope {
            tenant_id: request.scope.tenant_id.clone(),
            user_id: request.actor.user_id.clone(),
            agent_id: request.scope.agent_id.clone(),
            project_id: request.scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(request.scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        },
        correlation_id: CorrelationId::new(),
    }
}

fn map_memory_service_error(error: MemoryServiceError) -> AgentLoopHostError {
    match error.kind() {
        MemoryServiceErrorKind::Input => AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "memory search query is invalid",
        ),
        MemoryServiceErrorKind::Operation | MemoryServiceErrorKind::Unavailable => {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "memory context unavailable",
            )
        }
    }
}
