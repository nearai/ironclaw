//! #7185 / #7275 observability: a memory RETRIEVAL FAILURE is distinguishable
//! from "no matching memory", through the real composition.
//!
//! Both cases used to look identical from outside: a lane error was logged at
//! `debug!` and returned as an empty snippet list, so a memory backend that was
//! down produced exactly the same prompt — and exactly the same test evidence —
//! as a user with nothing relevant stored. Operators had no signal at all.
//!
//! The two arms below are byte-identical except for what the bound memory
//! provider returns from its retrieval lanes: `Err(unavailable)` versus
//! `Ok(vec![])`. Only the failing arm emits the operator-visible driver note.
//!
//! The note rides the milestone sink, not a log level: `info!`/`warn!` output
//! is rendered by the REPL and corrupts the terminal UI, and this fires from a
//! background prompt build.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::memory::{MemoryDescriptor, MemoryLifecycleHook};
use ironclaw_memory::{
    MemoryInvocation, MemoryService, MemoryServiceContextRequest, MemoryServiceContextSnippet,
    MemoryServiceError, MemoryServiceProfileReadResponse, MemoryServiceRecordRequest,
    MemoryServiceRecordResponse,
};

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// A bound memory provider whose retrieval lanes either fail or answer empty.
struct RetrievalOutcomeMemoryService {
    lanes_fail: bool,
}

#[async_trait]
impl MemoryService for RetrievalOutcomeMemoryService {
    async fn read_long_term(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        if self.lanes_fail {
            return Err(MemoryServiceError::unavailable());
        }
        Ok(Vec::new())
    }

    async fn read_short_term(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        if self.lanes_fail {
            return Err(MemoryServiceError::unavailable());
        }
        Ok(Vec::new())
    }

    async fn record_interaction(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceRecordRequest,
    ) -> Result<MemoryServiceRecordResponse, MemoryServiceError> {
        Ok(MemoryServiceRecordResponse { recorded: true })
    }

    async fn profile_read(
        &self,
        _invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        Ok(MemoryServiceProfileReadResponse { document: None })
    }
}

async fn group_with_lanes(lanes_fail: bool) -> HarnessResult<RebornIntegrationGroup> {
    RebornIntegrationGroup::builder()
        .with_bound_memory_provider(
            Arc::new(RetrievalOutcomeMemoryService { lanes_fail }) as Arc<dyn MemoryService>,
            MemoryDescriptor {
                lifecycle: MemoryLifecycleHook::ALL.to_vec(),
                ..MemoryDescriptor::default()
            },
        )
        .builtin_tools()
        .await
}

pub async fn run() -> HarnessResult<()> {
    // ── Arm 1: both retrieval lanes fail. The turn still completes (memory is
    // best-effort and must never break a run) AND the failure is surfaced. ───
    let failing = group_with_lanes(true).await?;
    let harness = failing
        .thread("conv-memory-retrieval-failed")
        .script([RebornScriptedReply::text("answered anyway")])
        .build()
        .await?;
    harness.submit_turn("what do you remember?").await?;
    harness.assert_reply_contains("answered anyway").await?;
    harness.assert_memory_retrieval_degraded_note().await?;
    drop(harness);

    // ── Arm 2: healthy lanes with nothing to return. Identical turn, no note —
    // this is what makes arm 1 evidence rather than noise. ───────────────────
    let healthy = group_with_lanes(false).await?;
    let harness = healthy
        .thread("conv-memory-retrieval-empty")
        .script([RebornScriptedReply::text("answered anyway")])
        .build()
        .await?;
    harness.submit_turn("what do you remember?").await?;
    harness.assert_reply_contains("answered anyway").await?;
    harness.assert_no_memory_retrieval_degraded_note().await?;

    Ok(())
}
