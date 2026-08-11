//! Production-path regression for #7185: a durable fact the model saves in one
//! conversation reaches a LATER conversation whose opening message shares no
//! content word with it.
//!
//! This is the case full-text retrieval cannot cover. Both proactive lanes used
//! to be nothing but search over the current turn's query, so recall depended on
//! the reader's first message happening to reuse the writer's vocabulary. Here
//! the reader asks something deliberately unrelated, makes NO memory tool call,
//! and the fact must still be in its system prompt — which only the standing
//! `MEMORY.md` the native provider serves at the head of its long-term lane can
//! do.
//!
//! Deliberately blind to HOW that happens: this scenario asserts the user-facing
//! property through real composition, so it is the behavior-neutrality proof
//! across reworks of the mechanism (the curated read moved from a host-composed
//! third lane into the provider's own `read_long_term` without touching a line
//! of it).
//!
//! Two further properties ride along, because both are ways this could "pass"
//! while being wrong: the writer saves in APPEND mode without the user asking
//! it to remember anything (the prompt protocol, not a user command, is what
//! makes it save), and another user's `MEMORY.md` on the same libSQL composite
//! never reaches this user's prompt.

use ironclaw_host_api::{
    ids::{CorrelationId, InvocationId, UserId},
    resource::ResourceScope,
};
use ironclaw_host_runtime::MEMORY_WRITE_CAPABILITY_ID;
use ironclaw_memory::{MemoryInvocation, MemoryServiceWriteRequest, MemoryWriteStatus};
use ironclaw_memory_native::NativeMemoryService;
use serde_json::json;

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// The durable fact. Its content words ("kayak", "quokka") appear nowhere in
/// the reader's opening message, so no full-text lane can match it.
const DURABLE_FACT: &str = "the user races a kayak named quokka every autumn";

/// Written only under the OTHER user's scope; must never surface here.
const OTHER_SCOPE_FACT: &str = "the other user races a kayak named narwhal every autumn";

pub async fn run() -> HarnessResult<()> {
    let group = RebornIntegrationGroup::builtin_tools_with_native_memory_libsql().await?;

    // Conversation A. The user states a preference in passing; nothing in the
    // message asks the agent to remember it. The scripted model saves it the
    // way the memory protocol instructs: target `memory`, append mode.
    let writer = group
        .thread("conv-always-on-writer")
        .script([
            RebornScriptedReply::tool_call(
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "target": "memory",
                    "content": DURABLE_FACT,
                    "append": true
                }),
            ),
            RebornScriptedReply::text("noted"),
        ])
        .build()
        .await?;
    writer
        .submit_turn("By the way, I race a kayak named quokka every autumn.")
        .await?;
    writer
        .assert_tool_invoked(MEMORY_WRITE_CAPABILITY_ID)
        .await?;
    let canonical_binding = writer.binding.clone();
    drop(writer);

    // Negative control: the SAME curated document path under a different user
    // on the one libSQL composite. The standing document is read on every run
    // regardless of query, so a scope-blind read would surface this.
    let other_user = UserId::new("reborn-always-on-other-scope-user")?;
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
                scope: other_scope,
                correlation_id: CorrelationId::new(),
            },
            MemoryServiceWriteRequest {
                target: "memory".to_string(),
                content: OTHER_SCOPE_FACT.to_string(),
                append: false,
                old_string: None,
                new_string: None,
                replace_all: false,
                metadata: None,
                timezone: None,
            },
        )
        .await
        .map_err(|error| format!("other-scope curated write: {error}"))?;
    if write.status != MemoryWriteStatus::Written {
        return Err(format!("other-scope curated write did not persist: {write:?}").into());
    }

    // Conversation B. The opening message shares no content word with the
    // saved fact, and the script makes no memory tool call — so full-text
    // retrieval cannot be what puts the fact in the prompt.
    let reader = group
        .thread("conv-always-on-reader")
        .script([RebornScriptedReply::text("answered")])
        .build()
        .await?;
    reader
        .submit_turn("Can you help me draft a short thank-you note?")
        .await?;
    reader.assert_system_prompt_contains("quokka").await?;
    reader.assert_system_prompt_excludes("narwhal").await?;

    Ok(())
}
