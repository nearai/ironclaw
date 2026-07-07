//! Memory-recall search query derivation and the memory-snippets step of
//! [`LoopContextPort::load_loop_context`](ironclaw_turns::run_profile::LoopContextPort::load_loop_context).
//!
//! The query text handed to [`MemoryPromptContextService`] is bounded but
//! otherwise raw: it carries no backend query syntax. Recall against it is a
//! literal-phrase match, not a free-form search — the backend, not this
//! crate, owns how "match this text literally" is expressed in its native
//! query dialect.

use ironclaw_threads::{ContextMessage, MessageKind};
use ironclaw_turns::{
    TurnActor,
    run_profile::{
        LoopContextSnippet, LoopRunContext, MemoryPromptContextRequest, MemoryPromptContextService,
    },
};

/// Upper bound on memory snippets admitted into one loop context bundle.
/// Not yet caller-configurable — no composition site has needed a different
/// value; revisit if one does.
pub(crate) const DEFAULT_MEMORY_CONTEXT_MAX_SNIPPETS: usize = 5;
/// Upper bound (in `char`s) admitted into a memory-recall search query
/// derived from a user message, before backend dispatch.
pub(crate) const MEMORY_QUERY_MAX_CHARS: usize = 512;

/// Load the memory snippets for one `load_loop_context` call: derive the
/// search query from the latest user message, resolve the actor memory
/// recall must key off, and query the memory backend.
///
/// Best-effort: unlike skill/identity context, memory recall is prompt
/// enrichment, not required context. A backend hiccup (contention, transient
/// unavailability) degrades to no snippets rather than failing the turn.
pub(crate) async fn load_memory_snippets_for_run(
    messages: &[ContextMessage],
    run_context: &LoopRunContext,
    memory_context_source: &(dyn MemoryPromptContextService + Send + Sync),
) -> Vec<LoopContextSnippet> {
    let Some(query) = latest_user_message_query(messages) else {
        return Vec::new();
    };
    // Explicit thread owner wins over the submitting actor: shared
    // conversation routes intentionally let actor/owner diverge
    // (`validate_thread_scope_for_run`), and memory must recall under the
    // thread's owner, not whichever actor is currently posting into it.
    let actor = run_context
        .scope
        .explicit_owner_user_id()
        .cloned()
        .map(TurnActor::new)
        .or_else(|| run_context.actor().cloned())
        .unwrap_or_else(|| TurnActor::new(run_context.scope.to_resource_scope().user_id));

    match memory_context_source
        .load_memory_snippets(MemoryPromptContextRequest {
            scope: run_context.scope.clone(),
            actor,
            query,
            max_snippets: DEFAULT_MEMORY_CONTEXT_MAX_SNIPPETS,
            context_profile_id: run_context.resolved_run_profile.context_profile_id.clone(),
        })
        .await
    {
        Ok(snippets) => snippets,
        Err(error) => {
            tracing::debug!(
                ?error,
                "memory context recall failed; continuing without snippets"
            );
            Vec::new()
        }
    }
}

/// Search query for memory recall: the most recent user-authored message in
/// the loaded context window, or `None` if the window has no user message
/// (nothing to search for — the caller must skip the memory lookup
/// entirely). The raw message is attacker-controlled, so it is bounded
/// before it ever reaches the memory backend; the backend is responsible for
/// treating it as literal content rather than query syntax.
fn latest_user_message_query(messages: &[ContextMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.kind == MessageKind::User)
        .map(|message| message.content.trim())
        .filter(|content| !content.is_empty())
        .map(memory_search_query_from_message)
}

/// Bounds a raw, potentially attacker-controlled message to
/// [`MEMORY_QUERY_MAX_CHARS`] for use as a memory-backend search query.
/// Bounding only — the text is passed through as-is, with no query-syntax
/// quoting or escaping. Literal-phrase intent is carried by the request
/// type the backend receives, not encoded into the text here.
fn memory_search_query_from_message(content: &str) -> String {
    content.chars().take(MEMORY_QUERY_MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_search_query_from_message_passes_through_raw_text() {
        // The bounding helper no longer quotes or escapes: query-syntax
        // metacharacters (hyphens, colons, commas) pass through unchanged.
        // Literal-phrase handling is the backend's responsibility, driven by
        // the request's literal-phrase intent, not by text mangling here.
        let query = memory_search_query_from_message("write the stale-ref file, please: now");

        assert_eq!(query, "write the stale-ref file, please: now");
    }

    #[test]
    fn memory_search_query_from_message_does_not_escape_embedded_quotes() {
        let query = memory_search_query_from_message(r#"say "hello" now"#);

        assert_eq!(query, r#"say "hello" now"#);
    }

    #[test]
    fn memory_search_query_from_message_bounds_length() {
        let long_message = "a".repeat(MEMORY_QUERY_MAX_CHARS + 100);

        let query = memory_search_query_from_message(&long_message);

        assert_eq!(query.chars().count(), MEMORY_QUERY_MAX_CHARS);
    }
}
