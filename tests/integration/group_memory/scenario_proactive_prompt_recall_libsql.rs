//! Production-path regression for #7275: a durable native-memory write in one
//! conversation is proactively retrieved into a different conversation's
//! model prompt from the shipping libSQL backend. The reader makes no memory
//! tool call, so only the host-managed prompt lane can satisfy the assertion.
//!
//! The recall must also be SCOPE-ISOLATED: a marker written under a different
//! user on the same libSQL composite reaches neither the canonical user's
//! explicit `memory_search` nor its proactive prompt, while staying
//! retrievable in its own scope.

use ironclaw_host_api::{
    ids::{CorrelationId, InvocationId, UserId},
    resource::ResourceScope,
};
use ironclaw_host_runtime::{MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use ironclaw_memory::{
    MemoryInvocation, MemoryServiceSearchRequest, MemoryServiceWriteRequest, MemoryWriteStatus,
};
use ironclaw_memory_native::NativeMemoryService;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// Written only under the OTHER user scope; must never surface to the
/// canonical user.
const OTHER_SCOPE_MARKER: &str = "rhubarb-77";

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
        .submit_turn("Please remember the staging launch code.")
        .await?;
    writer
        .assert_tool_invoked(MEMORY_WRITE_CAPABILITY_ID)
        .await?;
    let canonical_binding = writer.binding.clone();
    drop(writer);

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
    reader.assert_system_prompt_contains("plum-42").await?;
    reader.assert_system_prompt_excludes("banana-99").await?;
    reader
        .assert_system_prompt_excludes(OTHER_SCOPE_MARKER)
        .await?;

    Ok(())
}
