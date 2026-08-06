//! Production-path regression for #7275: a durable native-memory write in one
//! conversation is proactively retrieved into a different conversation's
//! model prompt from the shipping libSQL backend. The reader makes no memory
//! tool call, so only the host-managed prompt lane can satisfy the assertion.

use ironclaw_host_runtime::{MEMORY_SEARCH_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID};
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

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
    drop(writer);

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

    Ok(())
}
