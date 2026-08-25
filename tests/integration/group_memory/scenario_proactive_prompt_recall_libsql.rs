//! Production-path regression for #7275: a durable native-memory write in one
//! conversation is proactively retrieved into a different conversation's
//! model prompt from the shipping libSQL backend. The reader makes no memory
//! tool call, so only the host-managed prompt lane can satisfy the assertion.
//!
//! The recall must also be SCOPE-ISOLATED: a marker written under a different
//! user on the same libSQL composite reaches neither the canonical user's
//! explicit `memory_search` nor its proactive prompt, while staying
//! retrievable in its own scope.
//!
//! #7294 additions on the same production wiring:
//! - The writer conversation's after-turn transcript (thread-scoped scratch,
//!   proven recorded via the writer thread's own short-term lane) must NOT
//!   surface in the reader conversation's prompt — recall crosses
//!   conversations only through the durable memory the model explicitly
//!   wrote, never through another conversation's raw transcript.
//! - The recalled durable memory reaches the model behind the recall-framing
//!   guidance ("Recalled memory notice:"), so a recollection cannot read as
//!   verified current state.

use std::time::Duration;

use ironclaw_host_api::{
    ids::{CorrelationId, InvocationId, UserId},
    resource::ResourceScope,
};
use ironclaw_host_runtime::{MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use ironclaw_memory::{
    MemoryContextProfileId, MemoryInvocation, MemoryService, MemoryServiceContextRequest,
    MemoryServiceSearchRequest, MemoryServiceWriteRequest, MemoryWriteStatus,
};
use ironclaw_memory_native::NativeMemoryService;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// Written only under the OTHER user scope; must never surface to the
/// canonical user.
const OTHER_SCOPE_MARKER: &str = "rhubarb-77";

/// Present only in the WRITER conversation's user message — never durably
/// written — so after the turn it exists solely in that thread's recorded
/// transcript. It sits adjacent to the reader query's vocabulary ("staging
/// launch code"), so a lane filter that let another conversation's transcript
/// through would surface it in the reader's prompt (#7294).
const TRANSCRIPT_ONLY_MARKER: &str = "quokka-13";

pub async fn run() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools_with_native_memory_libsql().await?;

    let writer = group
        .thread("conv-memory-proactive-writer")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "memory",
                    "content": "the staging launch code is plum-42",
                    "append": false
                }),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await?;
    writer
        .submit_turn(&format!(
            "Please remember the staging launch code {TRANSCRIPT_ONLY_MARKER}."
        ))
        .await?;
    writer
        .assert_tool_invoked(MEMORY_WRITE_CAPABILITY_ID)
        .await?;
    let canonical_binding = writer.binding.clone();
    drop(writer);

    // The after-turn recorder is post-terminal and best-effort: wait until the
    // writer conversation's transcript actually landed in its thread-scoped
    // memory (through the same native provider over the group's libSQL
    // composite) so the reader-side exclusions below cannot pass vacuously.
    let canonical_memory =
        NativeMemoryService::from_filesystem(group.turn_composite().clone(), None);
    let writer_thread_scope = ResourceScope {
        tenant_id: canonical_binding.tenant_id.clone(),
        user_id: canonical_binding.actor_user_id.clone(),
        agent_id: canonical_binding.agent_id.clone(),
        project_id: canonical_binding.project_id.clone(),
        mission_id: None,
        thread_id: Some(canonical_binding.thread_id.clone()),
        invocation_id: InvocationId::new(),
    };
    let mut transcript_recorded = false;
    for _ in 0..100 {
        let snippets = canonical_memory
            .read_short_term(
                MemoryInvocation {
                    scope: writer_thread_scope.clone(),
                    correlation_id: CorrelationId::new(),
                },
                MemoryServiceContextRequest {
                    query: "staging launch code".to_string(),
                    max_snippets: 5,
                    context_profile_id: MemoryContextProfileId::new("default")
                        .map_err(|error| format!("context profile id: {error}"))?,
                },
            )
            .await
            .map_err(|error| format!("writer-thread short-term read: {error}"))?;
        if snippets
            .iter()
            .any(|snippet| snippet.text.contains(TRANSCRIPT_ONLY_MARKER))
        {
            transcript_recorded = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !transcript_recorded {
        return Err(
            "after-turn recorder did not record the writer conversation's transcript".into(),
        );
    }

    // Negative control: the same content lane, a DIFFERENT user, one composite.
    // This group pins capability dispatch to a single fixed user
    // (`core_builtin_tools_over_shared_filesystem`), so a second actor's thread
    // would still write memory under the canonical user — the other scope has
    // to be seeded through the same native provider the group binds, over the
    // group's own libSQL composite. Only the user axis differs.
    let other_user = UserId::new("reborn-memory-other-scope-user")?;
    let other_scope = ResourceScope {
        tenant_id: canonical_binding.tenant_id.clone(),
        user_id: other_user,
        agent_id: canonical_binding.agent_id.clone(),
        project_id: canonical_binding.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let other_scope_memory =
        NativeMemoryService::from_filesystem(group.turn_composite().clone(), None);
    let write = other_scope_memory
        .write(
            MemoryInvocation {
                scope: other_scope.clone(),
                correlation_id: CorrelationId::new(),
            },
            MemoryServiceWriteRequest {
                target: "memory".to_string(),
                // Word-for-word the canonical document apart from the marker, so a
                // scope-blind read of either lane is guaranteed to surface it.
                content: format!("the staging launch code is {OTHER_SCOPE_MARKER}"),
                append: false,
                old_string: None,
                new_string: None,
                replace_all: false,
                metadata: None,
                timezone: None,
            },
        )
        .await
        .map_err(|error| format!("other-scope memory write: {error}"))?;
    if write.status != MemoryWriteStatus::Written {
        return Err(format!("other-scope memory write did not persist: {write:?}").into());
    }
    // The exclusions below only mean isolation if the other scope can read its
    // own marker back through the same backend.
    let other_scope_hits = other_scope_memory
        .search(
            MemoryInvocation {
                scope: other_scope,
                correlation_id: CorrelationId::new(),
            },
            MemoryServiceSearchRequest {
                query: "What is staging AND launch-code?".to_string(),
                limit: 5,
            },
        )
        .await
        .map_err(|error| format!("other-scope memory search: {error}"))?;
    if !other_scope_hits
        .results
        .iter()
        .any(|hit| hit.content.contains(OTHER_SCOPE_MARKER))
    {
        return Err("other-scope memory search did not return its own marker".into());
    }

    let searcher = group
        .thread("conv-memory-explicit-search-libsql")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({
                    "query": "What is staging AND launch-code?",
                    "limit": 5
                }),
            ),
            RebornScriptedReply::text("found"),
        ])
        .build()
        .await?;
    searcher
        .submit_turn("Search my memory for the staging launch code.")
        .await?;
    searcher
        .assert_tool_invoked(MEMORY_SEARCH_CAPABILITY_ID)
        .await?;
    searcher.assert_tool_result_contains("plum-42").await?;
    if searcher
        .assert_tool_result_contains("banana-99")
        .await
        .is_ok()
    {
        return Err("memory search returned an unwritten marker".into());
    }
    // The other scope's document shares this query's "launch code" vocabulary,
    // so a scope-blind FTS filter would surface it here.
    if searcher
        .assert_tool_result_contains(OTHER_SCOPE_MARKER)
        .await
        .is_ok()
    {
        return Err("memory search leaked another user's memory across scopes".into());
    }
    drop(searcher);

    let reader = group
        .thread("conv-memory-proactive-reader")
        .script([RebornScriptedReply::text("answered")])
        .build()
        .await?;
    reader
        .submit_turn("What is the staging launch code?")
        .await?;
    reader.assert_model_request_contains("plum-42").await?;
    reader.assert_model_request_excludes("banana-99").await?;
    reader
        .assert_model_request_excludes(OTHER_SCOPE_MARKER)
        .await?;
    // #7294 leak probe: the writer conversation's transcript (recorded above,
    // sharing this query's vocabulary) must not cross into this conversation's
    // prompt through either retrieval lane.
    reader
        .assert_model_request_excludes(TRANSCRIPT_ONLY_MARKER)
        .await?;
    // #7294 presentation pin: recalled durable memory arrives behind the
    // recall-framing guidance, so the model reads it as a recollection to
    // verify rather than as live state.
    reader
        .assert_model_request_contains("Recalled memory notice:")
        .await?;

    Ok(())
}
