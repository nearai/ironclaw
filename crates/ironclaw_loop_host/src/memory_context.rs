use ironclaw_threads::{ContextMessage, MessageKind, SessionThreadService};
use ironclaw_turns::run_profile::{LoopContextSnippet, MemoryPromptContextRequest};

use crate::ThreadBackedLoopContextPort;

/// Upper bound on memory snippets requested per lane. The host's admission
/// budget (4 KiB aggregate / 512 B per snippet) admits at most ~8 snippets, so a
/// small per-lane request fills the budget without over-fetching the provider.
const MEMORY_PROMPT_CONTEXT_MAX_SNIPPETS: usize = 8;

impl<S> ThreadBackedLoopContextPort<S>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    /// Fetch proactive memory snippets ONCE per run, caching the result.
    ///
    /// The first prompt build of the run seeds the query from the latest user
    /// message and fetches both lanes through the wired
    /// [`ironclaw_turns::run_profile::MemoryPromptContextService`]; subsequent
    /// per-iteration calls reuse the cached snippets (the "fetch once per run"
    /// guarantee). When no service is wired, or there is no actor / user message
    /// to scope a query to, this returns empty. A fetch failure degrades to empty
    /// and never fails the turn.
    pub(super) async fn load_memory_snippets_once(
        &self,
        context_messages: &[ContextMessage],
    ) -> Vec<LoopContextSnippet> {
        let Some(service) = self.memory_context_service.as_deref() else {
            return Vec::new();
        };
        // Build the request BEFORE touching the cache. When there is no actor or no
        // user message yet, there is nothing to query: return empty WITHOUT seeding
        // the `OnceCell`, so a later prompt build that DOES carry a user message can
        // still fetch (M1 regression - seeding the cell with an empty vec here froze
        // memory to empty for the rest of the run). Only seed the cell once a real
        // request exists.
        let Some(request) = self.build_memory_prompt_context_request(context_messages) else {
            return Vec::new();
        };
        // Fetch exactly once per run and CACHE the outcome - including an empty vec
        // on failure. A down or slow memory service must not be re-hit on every
        // model step of the run: the prior `get_or_try_init` left the cell
        // uninitialized on error, so each iteration retried and could stack
        // timeouts into latency spikes. A retrieval failure degrades to empty memory
        // for the rest of the run rather than failing the turn; the per-run cache
        // makes that decision exactly once.
        let snippets = self
            .memory_snippets_cache
            .get_or_init(|| async {
                match service.load_memory_snippets(request).await {
                    Ok(snippets) => snippets,
                    Err(error) => {
                        tracing::debug!(
                            kind = ?error.kind,
                            "memory context fetch failed; degrading to empty memory for this run"
                        );
                        Vec::new()
                    }
                }
            })
            .await;
        snippets.clone()
    }

    /// Build the memory request from the run context. Returns `None` (no memory
    /// fetch) when there is no actor to scope to, or no user message to derive a
    /// query from; both degrade to empty rather than failing the turn.
    fn build_memory_prompt_context_request(
        &self,
        context_messages: &[ContextMessage],
    ) -> Option<MemoryPromptContextRequest> {
        // Memory is keyed to the human user; without an actor there is no user to
        // scope to.
        let actor = self.run_context.actor()?.clone();
        // The query is the latest user message - the first prompt build of the
        // run carries the real user turn, which the per-run cache then freezes.
        let query = latest_user_message_text(context_messages)?;
        Some(MemoryPromptContextRequest {
            scope: self.run_context.scope.clone(),
            actor,
            query,
            max_snippets: MEMORY_PROMPT_CONTEXT_MAX_SNIPPETS,
            context_profile_id: self
                .run_context
                .resolved_run_profile
                .context_profile_id
                .clone(),
        })
    }
}

/// The text of the latest user message in the context window, used as the memory
/// retrieval query. Returns `None` when there is no (non-blank) user message yet.
/// Messages arrive ordered ascending by sequence, so the last `User` message is
/// the most recent.
pub(crate) fn latest_user_message_text(messages: &[ContextMessage]) -> Option<String> {
    // The latest NON-BLANK user message: skip blank trailing user rows and keep
    // looking back, so a whitespace-only newest user turn doesn't drop memory for
    // the run when an earlier user turn carries real content.
    messages.iter().rev().find_map(|message| {
        (message.kind == MessageKind::User && !message.content.trim().is_empty())
            .then(|| message.content.clone())
    })
}
