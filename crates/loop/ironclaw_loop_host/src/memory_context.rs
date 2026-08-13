use std::sync::Arc;
use std::sync::atomic::Ordering;

use ironclaw_loop_contracts::{
    LoopContextSnippet, LoopDriverNoteKind, LoopHostMilestoneEmitter, LoopSafeSummary,
    MemoryPromptContextLoad, MemoryPromptContextRequest, MemoryRetrievalDegradation,
    MemoryRetrievalFailureKind, MemoryRetrievalLane,
};
use ironclaw_threads::{ContextMessage, MessageKind, SessionThreadService};

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
    /// [`ironclaw_loop_contracts::MemoryPromptContextService`]; subsequent
    /// per-iteration calls reuse the cached snippets (the "fetch once per run"
    /// guarantee). When no service is wired, or there is no actor / user message
    /// to scope a query to, this returns empty. A fetch failure degrades to empty
    /// and never fails the turn — but it is recorded, not laundered: see
    /// [`Self::load_memory_context_once`].
    pub(super) async fn load_memory_snippets_once(
        &self,
        context_messages: &[ContextMessage],
    ) -> Vec<LoopContextSnippet> {
        self.load_memory_context_once(context_messages)
            .await
            .snippets
    }

    /// The full memory load for this run: the admitted snippets plus every lane
    /// that FAILED rather than simply matching nothing.
    ///
    /// The distinction matters because both used to look identical downstream.
    /// A memory backend that is down produced the same empty prompt section as
    /// a user with nothing relevant stored, so "it forgot" and "retrieval broke"
    /// were indistinguishable in tests and in operator diagnostics.
    pub(super) async fn load_memory_context_once(
        &self,
        context_messages: &[ContextMessage],
    ) -> MemoryPromptContextLoad {
        let Some(service) = self.memory_context_service.as_deref() else {
            return MemoryPromptContextLoad::default();
        };
        // Build the request BEFORE touching the cache. When there is no actor or no
        // user message yet, there is nothing to query: return empty WITHOUT seeding
        // the `OnceCell`, so a later prompt build that DOES carry a user message can
        // still fetch (M1 regression - seeding the cell with an empty vec here froze
        // memory to empty for the rest of the run). Only seed the cell once a real
        // request exists.
        let Some(request) = self.build_memory_prompt_context_request(context_messages) else {
            return MemoryPromptContextLoad::default();
        };
        // Fetch exactly once per run and CACHE the outcome. A down or slow memory
        // service must not be re-hit on every model step of the run: the prior
        // `get_or_try_init` left the cell uninitialized on error, so each iteration
        // retried and could stack timeouts into latency spikes.
        //
        // The cache therefore still holds a failed fetch for the rest of the run —
        // but it caches the whole `MemoryPromptContextLoad`, so the cached value
        // RECORDS that it failed instead of being indistinguishable from "no
        // matching memory". A hard error from the service is folded into the same
        // shape with an `Unavailable` degradation covering both lanes.
        let load = self
            .memory_snippets_cache
            .get_or_init(|| async {
                match service.load_memory_snippets(request).await {
                    Ok(load) => load,
                    Err(error) => {
                        tracing::debug!(
                            kind = ?error.kind,
                            "memory context fetch failed; degrading to empty memory for this run"
                        );
                        MemoryPromptContextLoad {
                            snippets: Vec::new(),
                            degradations: vec![
                                MemoryRetrievalDegradation::new(
                                    MemoryRetrievalLane::ShortTerm,
                                    MemoryRetrievalFailureKind::Unavailable,
                                ),
                                MemoryRetrievalDegradation::new(
                                    MemoryRetrievalLane::LongTerm,
                                    MemoryRetrievalFailureKind::Unavailable,
                                ),
                            ],
                        }
                    }
                }
            })
            .await
            .clone();
        self.publish_memory_retrieval_degraded(&load.degradations);
        load
    }

    /// Surface a degraded memory retrieval to the operator as a driver note.
    ///
    /// The note is the operator-visible half of the typed degradation: it rides
    /// the milestone sink this port already holds and reaches the live work
    /// summary, the same route `publish_personal_context_admitted` uses and the
    /// same rationale as `EventSubscriptionTerminated` — a subsystem that
    /// stopped contributing must not be silently invisible.
    ///
    /// Deliberately NOT `warn!`/`info!`: those levels render in the REPL and
    /// corrupt the terminal UI, and this fires from a background prompt build.
    /// The summary carries only closed-vocabulary lane and failure labels, never
    /// a backend message, a query, or a path.
    ///
    /// The per-run `OnceCell` above means the fetch happens once, but this is
    /// called on every prompt build that reads the cache, so it is guarded to
    /// stay at one note per run.
    ///
    /// That guard is claimed in two steps, matching
    /// `publish_personal_context_admitted`: an in-flight flag suppresses
    /// duplicates while a publish is outstanding, and the `OnceCell` is set
    /// only once a publish SUCCEEDS. Marking the note emitted up front would
    /// mean a single transient sink error silently suppressed it for the rest
    /// of the run — putting the operator back in exactly the state this change
    /// exists to fix, unable to tell broken retrieval from empty memory.
    fn publish_memory_retrieval_degraded(&self, degradations: &[MemoryRetrievalDegradation]) {
        if degradations.is_empty() {
            return;
        }
        let Some(milestone_sink) = self.milestone_sink.as_ref() else {
            return;
        };
        if self.memory_degradation_note_emitted.get().is_some() {
            return;
        }
        if self
            .memory_degradation_note_in_flight
            .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let lanes = degradations
            .iter()
            .map(|degradation| {
                format!(
                    "{}:{}",
                    degradation.lane.as_str(),
                    degradation.kind.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let summary = match LoopSafeSummary::new(format!("memory retrieval degraded ({lanes})")) {
            Ok(summary) => summary,
            Err(error) => {
                self.memory_degradation_note_in_flight
                    .store(false, Ordering::Release);
                tracing::debug!("failed to build memory degradation milestone: {error}");
                return;
            }
        };
        let context = self.run_context.clone();
        let milestone_sink = Arc::clone(milestone_sink);
        let emitted = Arc::clone(&self.memory_degradation_note_emitted);
        let in_flight = Arc::clone(&self.memory_degradation_note_in_flight);
        tokio::spawn(async move {
            let publish_result = LoopHostMilestoneEmitter::new(context, milestone_sink)
                .driver_note(LoopDriverNoteKind::Context, summary)
                .await;
            match publish_result {
                Ok(()) => {
                    let _ = emitted.set(());
                }
                Err(error) => {
                    tracing::debug!("failed to emit memory degradation milestone: {error}");
                }
            }
            in_flight.store(false, Ordering::Release);
        });
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
