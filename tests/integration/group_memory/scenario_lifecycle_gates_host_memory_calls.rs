//! F3/F8 end-to-end: a lifecycle hook the bound provider's manifest does not
//! declare is NEVER called across a full real turn — no retrieval query, no
//! after-turn record, no profile read — while a full declaration drives every
//! hook through the SAME production consumers
//! (`ironclaw_composition::memory_lifecycle_consumers`, the derivation
//! `build_reborn_runtime` wires).
//!
//! The counterpart is pinned here too: `read_document` is NOT a lifecycle hook,
//! so the host-composed always-on curated lane reads the standing document even
//! under an empty declaration. Gating it would put a user's standing facts
//! behind a manifest opt-in, which is the failure #7185 exists to fix.
//!
//! Builds its own groups (not the shared `builtin_tools` group): the memory
//! provider + declared lifecycle are per-group construction inputs.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use ironclaw_extension_contracts::memory::{MemoryDescriptor, MemoryLifecycleHook};
use ironclaw_memory::{
    MemoryInvocation, MemoryService, MemoryServiceContextRequest, MemoryServiceContextSnippet,
    MemoryServiceError, MemoryServiceProfileReadResponse, MemoryServiceReadRequest,
    MemoryServiceReadResponse, MemoryServiceRecordRequest, MemoryServiceRecordResponse,
};

use super::reborn_support::group::{HarnessResult, RebornIntegrationGroup};
use super::reborn_support::reply::RebornScriptedReply;

/// Counts every host-initiated lifecycle call; returns inert successes so the
/// turn's behavior never depends on memory content.
#[derive(Default)]
struct RecordingLifecycleMemoryService {
    long_term_reads: AtomicUsize,
    short_term_reads: AtomicUsize,
    interaction_records: AtomicUsize,
    profile_reads: AtomicUsize,
    /// NOT a lifecycle hook: the ordinary document read the host composes the
    /// always-on curated lane out of. Counted separately because it is
    /// deliberately NOT gated by `[memory].lifecycle` — see Arm 1.
    document_reads: AtomicUsize,
}

#[async_trait]
impl MemoryService for RecordingLifecycleMemoryService {
    async fn read_long_term(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        self.long_term_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    async fn read_short_term(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        self.short_term_reads.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    async fn read_document(
        &self,
        _invocation: MemoryInvocation,
        request: MemoryServiceReadRequest,
    ) -> Result<MemoryServiceReadResponse, MemoryServiceError> {
        self.document_reads.fetch_add(1, Ordering::SeqCst);
        // Absent, the way both bundled providers report a path naming nothing.
        let _ = request;
        Err(MemoryServiceError::input())
    }

    async fn record_interaction(
        &self,
        _invocation: MemoryInvocation,
        _request: MemoryServiceRecordRequest,
    ) -> Result<MemoryServiceRecordResponse, MemoryServiceError> {
        self.interaction_records.fetch_add(1, Ordering::SeqCst);
        Ok(MemoryServiceRecordResponse { recorded: true })
    }

    async fn profile_read(
        &self,
        _invocation: MemoryInvocation,
    ) -> Result<MemoryServiceProfileReadResponse, MemoryServiceError> {
        self.profile_reads.fetch_add(1, Ordering::SeqCst);
        Ok(MemoryServiceProfileReadResponse { document: None })
    }
}

fn expect_count(label: &str, actual: usize, expected_zero: bool) -> HarnessResult<()> {
    match (expected_zero, actual) {
        (true, 0) => Ok(()),
        (false, n) if n > 0 => Ok(()),
        (true, n) => Err(format!("{label}: expected ZERO host calls, saw {n}").into()),
        (false, _) => Err(format!("{label}: expected at least one host call, saw none").into()),
    }
}

pub async fn run() -> HarnessResult<()> {
    // ── Arm 1: an EMPTY lifecycle — the host issues ZERO memory calls. ──────
    let silent = Arc::new(RecordingLifecycleMemoryService::default());
    let group = RebornIntegrationGroup::builder()
        .with_bound_memory_provider(
            Arc::clone(&silent) as Arc<dyn MemoryService>,
            MemoryDescriptor::default(),
        )
        .builtin_tools()
        .await?;
    let harness = group
        .thread("conv-memory-lifecycle-empty")
        .script([RebornScriptedReply::text("nothing remembered")])
        .build()
        .await?;
    harness.submit_turn("anything worth remembering?").await?;
    harness.assert_reply_contains("nothing remembered").await?;
    // `submit_turn` returns at `Completed`, but a MIS-wired after-turn record
    // would run just after the status flip — inline on the scheduler worker
    // (`turn_run_executor.rs`), not on a detached queue that could defer it
    // arbitrarily. Poll-negative over a bounded window so such a stray call
    // FAILS the zero assertions below instead of landing after them.
    let mut waited = Duration::ZERO;
    while waited < Duration::from_secs(1) {
        let fired = silent.long_term_reads.load(Ordering::SeqCst)
            + silent.short_term_reads.load(Ordering::SeqCst)
            + silent.interaction_records.load(Ordering::SeqCst)
            + silent.profile_reads.load(Ordering::SeqCst);
        if fired > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        waited += Duration::from_millis(25);
    }
    expect_count(
        "empty lifecycle / read_long_term",
        silent.long_term_reads.load(Ordering::SeqCst),
        true,
    )?;
    expect_count(
        "empty lifecycle / read_short_term",
        silent.short_term_reads.load(Ordering::SeqCst),
        true,
    )?;
    expect_count(
        "empty lifecycle / record_interaction",
        silent.interaction_records.load(Ordering::SeqCst),
        true,
    )?;
    expect_count(
        "empty lifecycle / profile_read",
        silent.profile_reads.load(Ordering::SeqCst),
        true,
    )?;
    // …but the always-on curated lane is NOT a lifecycle hook. It is composed
    // host-side out of the ordinary document read, so it runs against any bound
    // provider without a declaration — the whole point of #7185 is that a user's
    // standing facts do not depend on a manifest opting in.
    expect_count(
        "empty lifecycle / read_document (host-composed curated lane)",
        silent.document_reads.load(Ordering::SeqCst),
        false,
    )?;

    // ── Arm 2: the full declaration — every hook fires through the same
    // consumers (guards Arm 1 against passing vacuously). ────────────────────
    let observed = Arc::new(RecordingLifecycleMemoryService::default());
    let group = RebornIntegrationGroup::builder()
        .with_bound_memory_provider(
            Arc::clone(&observed) as Arc<dyn MemoryService>,
            MemoryDescriptor {
                lifecycle: MemoryLifecycleHook::ALL.to_vec(),
            },
        )
        .builtin_tools()
        .await?;
    let harness = group
        .thread("conv-memory-lifecycle-full")
        .script([RebornScriptedReply::text("remembered")])
        .build()
        .await?;
    harness.submit_turn("anything worth remembering?").await?;
    harness.assert_reply_contains("remembered").await?;

    // The after-turn record runs post-terminal (best-effort, after `Completed`
    // is observable), so poll briefly instead of racing it.
    let mut waited = Duration::ZERO;
    while observed.interaction_records.load(Ordering::SeqCst) == 0
        && waited < Duration::from_secs(5)
    {
        tokio::time::sleep(Duration::from_millis(25)).await;
        waited += Duration::from_millis(25);
    }

    expect_count(
        "full lifecycle / read_long_term",
        observed.long_term_reads.load(Ordering::SeqCst),
        false,
    )?;
    expect_count(
        "full lifecycle / read_short_term",
        observed.short_term_reads.load(Ordering::SeqCst),
        false,
    )?;
    expect_count(
        "full lifecycle / record_interaction",
        observed.interaction_records.load(Ordering::SeqCst),
        false,
    )?;
    expect_count(
        "full lifecycle / profile_read",
        observed.profile_reads.load(Ordering::SeqCst),
        false,
    )?;
    Ok(())
}
