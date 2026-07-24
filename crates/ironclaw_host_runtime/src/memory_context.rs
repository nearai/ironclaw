//! Production [`MemoryPromptContextService`] adapter backed by IronClaw memory.
//!
//! This adapter bridges the memory service into the agent loop context pipeline.
//! It derives the host-resolved memory invocation scope from the request's
//! [`TurnScope`] and [`TurnActor`], then makes two scoped reads — long-term
//! (general) and short-term (this thread) — through [`MemoryService`]. The memory
//! service owns the per-snippet safety (untrusted envelope + size cap + scope
//! check) so it returns prompt-safe snippets; the only host-side step is the
//! loop's prompt-content denylist (a drop-filter) and the model-visible reference,
//! both of which depend on loop-layer types and so stay here.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{CorrelationId, InvocationId, ResourceScope};
use ironclaw_memory::{
    MemoryContextProfileId, MemoryInvocation, MemoryService, MemoryServiceContextSnippet,
    MemoryServiceError, MemoryServiceErrorKind, memory_context_disabled,
};
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopContextSnippet, LoopSafeSummary,
    MemoryPromptContextRequest, MemoryPromptContextService, memory_snippet_display_ref,
};

/// Aggregate model-visible byte budget across all admitted snippets in one turn.
/// The per-snippet byte cap lives with the memory service (it owns sanitization);
/// this combined ceiling is the one budget that must see both lanes, so it stays
/// here where the two reads are concatenated.
const MAX_MEMORY_CONTEXT_TOTAL_BYTES: usize = 4 * 1024;

/// Production adapter that loads memory snippets through IronClaw memory.
pub struct ProductionMemoryPromptContextService {
    memory_service: Arc<dyn MemoryService>,
}

impl ProductionMemoryPromptContextService {
    /// Create a new production adapter wrapping the configured memory service
    /// facade. Native memory remains the default facade adapter in Phase 1.
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
        // memory service keeps an equivalent check as defense in depth).
        if memory_context_disabled(request.context_profile_id.as_str()) {
            return Ok(Vec::new());
        }
        // The host-resolved `ContextProfileId` is already validated, so this
        // construction won't fail in practice — but propagate rather than unwrap.
        let context_profile_id = MemoryContextProfileId::new(request.context_profile_id.as_str())
            .map_err(map_memory_service_error)?;

        // mem0 `on_run_start` shape: two scoped reads, fetched concurrently. The
        // memory service owns the lane scoping AND the per-snippet safety (untrusted
        // envelope + size cap + scope check), so each call returns prompt-safe
        // snippets. The same invocation goes to both: `read_long_term` clears the
        // thread internally (general memory, excludes `threads/`), `read_thread`
        // keeps it (this conversation).
        let invocation = invocation_for_context_request(&request);
        let (long_term, short_term) = tokio::join!(
            self.memory_service.read_long_term(
                invocation.clone(),
                request.query.clone(),
                request.max_snippets,
                context_profile_id.clone(),
            ),
            self.memory_service.read_thread(
                invocation,
                request.query,
                request.max_snippets,
                context_profile_id,
            ),
        );

        // Concatenate short-term before long-term so active-thread memory keeps
        // priority under the shared count + aggregate byte budget. The prompt
        // renderer preserves host order for memory snippets, so this is the lane
        // priority boundary.
        let mut admitted = Vec::new();
        let mut total_bytes = 0usize;
        for snippet in short_term.into_iter().chain(long_term) {
            if admitted.len() >= request.max_snippets {
                break;
            }
            let Some(loop_snippet) = to_loop_context_snippet(snippet) else {
                continue;
            };
            let snippet_bytes = loop_snippet.safe_summary.len();
            if total_bytes.saturating_add(snippet_bytes) > MAX_MEMORY_CONTEXT_TOTAL_BYTES {
                break;
            }
            total_bytes = total_bytes.saturating_add(snippet_bytes);
            admitted.push(loop_snippet);
        }
        Ok(admitted)
    }
}

/// Map a memory-service safe snippet onto a loop context snippet, or drop it.
///
/// The memory service already sanitized the `text` (control-stripped, size-capped,
/// untrusted-enveloped) and scope-checked the snippet. The host adds the two steps
/// that depend on loop-layer types: it builds the model-visible `memory-snippet:*`
/// reference from the scope/path components, and runs the loop's prompt-content
/// denylist ([`LoopSafeSummary`]) as a DROP-filter — a prompt-layer policy applied
/// to all model context — so a memory doc carrying a denylisted secret/path is
/// skipped here rather than failing the instruction bundle at render time.
fn to_loop_context_snippet(snippet: MemoryServiceContextSnippet) -> Option<LoopContextSnippet> {
    let snippet_ref = memory_snippet_display_ref([
        snippet.tenant_id.as_str(),
        snippet.user_id.as_str(),
        snippet.agent_id.as_deref().unwrap_or(""),
        snippet.project_id.as_deref().unwrap_or(""),
        snippet.relative_path.as_str(),
    ]);
    let safe = LoopSafeSummary::new(snippet.text)
        .ok()?
        .as_str()
        .to_string();
    Some(LoopContextSnippet {
        snippet_ref,
        safe_summary: safe.clone(),
        model_content: safe,
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

/// Map a provider error onto the agent-loop host error surface.
///
/// Only the `context_profile_id` construction can surface an error on this path
/// now (the lane reads degrade internally to empty), so this maps that validation
/// failure; `Operation`/`Unavailable` are retained for completeness.
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

#[cfg(test)]
mod tests {
    //! Host-side `to_loop_context_snippet` regression tests: the loop's prompt
    //! denylist drop-filter + the model-visible reference. The per-snippet
    //! sanitization (control-char/truncate/envelope) and scope-check live with the
    //! memory service now and are tested there; end-to-end admission coverage lives
    //! in `tests/memory_prompt_context.rs`.

    use super::*;

    fn snippet(text: &str) -> MemoryServiceContextSnippet {
        MemoryServiceContextSnippet {
            tenant_id: "tenant-a".to_string(),
            user_id: "user-x".to_string(),
            agent_id: None,
            project_id: None,
            relative_path: "notes/alpha.md".to_string(),
            text: text.to_string(),
        }
    }

    /// Benign content is mapped onto a loop snippet with a stable `memory-snippet:*`
    /// reference and identical safe-summary / model-content.
    #[test]
    fn maps_benign_snippet_with_reference() {
        let mapped =
            to_loop_context_snippet(snippet("Untrusted memory content: ordinary planning note"))
                .expect("benign snippet must map");
        assert!(mapped.snippet_ref.starts_with("memory-snippet:"));
        assert_eq!(
            mapped.snippet_ref,
            memory_snippet_display_ref(["tenant-a", "user-x", "", "", "notes/alpha.md"])
        );
        assert_eq!(mapped.safe_summary, mapped.model_content);
        assert!(mapped.safe_summary.contains("ordinary planning note"));
    }

    /// A snippet carrying a filesystem path is dropped by the loop denylist
    /// (rather than erroring the bundle later at render time).
    #[test]
    fn drops_snippet_with_path_delimiters() {
        assert!(to_loop_context_snippet(snippet("/etc/passwd")).is_none());
    }

    /// A snippet mentioning a secret marker is dropped by the loop denylist.
    #[test]
    fn drops_snippet_with_sensitive_marker() {
        assert!(to_loop_context_snippet(snippet("the api key is exposed")).is_none());
    }

    /// The denylist must not false-positive on benign substrings ("impact"
    /// contains "pa" but is not "passwd").
    #[test]
    fn keeps_snippet_with_benign_marker_substring() {
        assert!(to_loop_context_snippet(snippet("impact assessment notes")).is_some());
    }
}
