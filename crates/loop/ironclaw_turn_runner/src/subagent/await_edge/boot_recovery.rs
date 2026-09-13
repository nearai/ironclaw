//! Recovery projection for unresolved process dependencies.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use ironclaw_host_api::turn::TurnRunId;
use ironclaw_loop_contracts::AgentLoopHostError;
use ironclaw_loop_host::{AwaitEdgeWriter, ResolveReport};
use ironclaw_processes::ProcessDependencyState;
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnScope;

use super::{AwaitEdge, AwaitEdgeState, resolver::AwaitEdgeResolver, store::AwaitEdgeStore};

pub(super) async fn recover_scope<S>(
    resolver: &AwaitEdgeResolver<S>,
    store: &AwaitEdgeStore,
    scope: &TurnScope,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized,
{
    let mut report = ResolveReport::default();
    let unclosed = match store.list_unclosed_for_scope(scope).await {
        Ok(edges) => edges,
        Err(error) => {
            tracing::debug!(error = %error, "process dependency recovery query failed");
            report.record_failed();
            return report;
        }
    };
    for (parent_run_id, child_run_id, edge) in unclosed {
        let outcome = match edge.state {
            super::AwaitEdgeState::Open => continue,
            // Blocking-mode `Settled` keeps its pre-existing group drain
            // (resumes the blocked parent once every group member has
            // settled). Background-mode `Settled` never reaches
            // `drain_settled_group` — `resume_parent` is exclusive to the
            // blocking dependent-run gate, which a background parent never
            // parks on (`deliver_background`'s own doc comment) — so it takes
            // the same `deliver_background` re-drive path as the other
            // background delivery substates below.
            super::AwaitEdgeState::Settled
                if edge.mode == ironclaw_loop_host::SpawnSubagentMode::Background =>
            {
                resolver
                    .deliver_background(&edge, parent_run_id, child_run_id, false)
                    .await
            }
            super::AwaitEdgeState::Settled => {
                resolver
                    .drain_settled_group(scope, parent_run_id, child_run_id)
                    .await
            }
            // Crash-recovery re-drive of a half-delivered background result
            // (System provenance — boot recovery is not a human/permitted
            // start): `ResultAppended` re-attends (append is a no-op
            // replay); `AttentionScheduled` closes only, both through
            // `deliver_background`'s existing idempotent re-drive contract.
            super::AwaitEdgeState::ResultAppended | super::AwaitEdgeState::AttentionScheduled => {
                resolver
                    .deliver_background(&edge, parent_run_id, child_run_id, false)
                    .await
            }
            // A streak-capped edge stays parked at boot — boot recovery is
            // never a human-initiated start, so it must not drain this
            // forward; a later permitted/human run-start sweep does.
            super::AwaitEdgeState::AttentionDeferredStreakCap => continue,
            super::AwaitEdgeState::Drained | super::AwaitEdgeState::Abandoned => continue,
        };
        match outcome {
            Ok(outcome) => report.record(outcome),
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    %parent_run_id,
                    %child_run_id,
                    "process dependency recovery drain failed"
                );
                report.record_failed();
            }
        }
    }
    report
}

/// Total edges one boot/periodic sweep pass will *drive through delivery*
/// (`AwaitEdgeResolver::deliver_background`), across every scope, before
/// returning. `8 * MAX_QUEUED_INPUTS_PER_RUN`: the run-start sweep
/// (`sweep_thread_on_run_start`) caps itself at `MAX_QUEUED_INPUTS_PER_RUN`
/// (32) edges for *one thread*; a boot pass fans out across every
/// thread/scope in the deployment instead of one thread, so its budget is a
/// multiple of that single-thread cap rather than equal to it — high enough
/// to make real progress on a cold boot with several backlogged scopes,
/// bounded so one pass cannot run unbounded.
const MAX_BOOT_SWEEP_EDGES: usize = ironclaw_loop_host::MAX_QUEUED_INPUTS_PER_RUN * 8;

/// Total edges one boot/periodic sweep pass will *scan and accumulate in
/// memory* before bucketing for round-robin delivery — deliberately larger
/// than [`MAX_BOOT_SWEEP_EDGES`], and a distinct constant from it. Scanning
/// is a cheap, side-effect-free read; delivery is the expensive/dangerous
/// part (thread-service writes, coordinator activation calls), which is why
/// only delivery is capped at the smaller number. The gap between the two
/// caps is what makes round-robin fairness actually fair rather than
/// order-dependent: if the scan itself stopped at `MAX_BOOT_SWEEP_EDGES`,
/// one large scope's backlog could — depending on the arbitrary
/// `(dependent_process_id, dependency_process_id)` scan order — fill the
/// entire accumulated set before a smaller scope's edges are ever read,
/// which would silently starve that scope regardless of the round-robin
/// delivery loop below. Scanning generously first, then rationing only the
/// expensive delivery step, guarantees every backlogged scope is represented
/// in the round-robin pool whenever its total across all scopes fits under
/// this cap. `4 * MAX_BOOT_SWEEP_EDGES`: enough headroom for round-robin to
/// find every scope's edges even when the single largest scope's backlog
/// alone exceeds the delivery cap, while still bounded so a pathological
/// multi-million-edge backlog cannot make one sweep pass load unbounded
/// memory.
const BOOT_SWEEP_SCAN_CAP: usize = MAX_BOOT_SWEEP_EDGES * 4;

/// Dependency-delivery states a boot/periodic pass acts on: the same
/// non-deferred set `sweep_thread_on_run_start` acts on for an autonomous
/// (non-human-initiated) start. `AttentionDeferred` (the kernel's
/// domain-neutral name for `AttentionDeferredStreakCap`) is excluded on
/// purpose — a streak-capped edge must wait for a permitted or
/// human-initiated start, and a boot pass is neither; `recover_scope`
/// already makes the same call via `retry_deferred: false`.
fn boot_sweep_states() -> Vec<ProcessDependencyState> {
    vec![
        ProcessDependencyState::Settled,
        ProcessDependencyState::ResultAppended,
        ProcessDependencyState::AttentionScheduled,
    ]
}

/// Boot/periodic sweep (§4.2, §6 row R4) — the third healing trigger,
/// covering background await-edges whose thread may never start another run.
/// Unlike `recover_scope` (one scope, driven lazily on admission) and
/// `AwaitEdgeResolver::sweep_thread_on_run_start` (one thread, driven at run
/// start), this walks every unclosed background edge in the deployment via
/// `ProcessDependencyPort::scan_unclosed_process_dependencies`
/// (`AwaitEdgeStore::scan_unclosed_background`, `group_ref_prefix: "bg:"` —
/// the exact tag `finish_spawn` writes for `SpawnSubagentMode::Background`,
/// so every scanned record is background-mode by construction; no
/// `edge.mode` check needed).
///
/// **Fairness.** Every page is accumulated first (bounded by
/// `BOOT_SWEEP_SCAN_CAP` total, or scan exhaustion), then bucketed by each
/// record's own `group_ref` (`"bg:{parent_thread_id}"` — the parent backlog
/// identity, *not* `record.scope`, which is the child's own scope and would
/// put every edge in a singleton bucket) in first-seen order, then drained
/// round-robin — capped separately at `MAX_BOOT_SWEEP_EDGES` delivered: one
/// edge from parent A, one from parent B, one from parent C, repeat — never
/// all of parent A's backlog before B gets a turn. A backlog-by-backlog
/// drain (finish every A edge, then every B edge) would starve B whenever A
/// alone exceeds the delivery cap; the round-robin loop below is what
/// prevents that
/// (`sweep_is_fair_across_scopes_when_one_scope_exceeds_the_cap` in
/// `boot_recovery/tests.rs` fails without it).
///
/// Delivery re-drive is `AwaitEdgeResolver::deliver_background` — the same
/// idempotent path `recover_scope` and `sweep_thread_on_run_start` call, no
/// second delivery implementation. One edge's failure is logged and counted;
/// it never aborts the rest of the pass, mirroring `sweep_thread_on_run_start`.
///
/// **One read in the common case.** The underlying
/// `rows::unresolved_dependencies` read the scan is built on takes no
/// `limit` — every `scan_unclosed_process_dependencies` call re-reads the
/// *whole* unclosed-dependency index, regardless of the page size requested
/// (see the ponytail on that method). Requesting the scan cap as a single
/// page (below) means one sweep pass costs one full index read whenever the
/// backlog fits under `BOOT_SWEEP_SCAN_CAP`, instead of paying that cost once
/// per page. The keyset cursor still runs — and is still exercised, by
/// `sweep_scan_pages_through_a_backlog_larger_than_one_page` in
/// `boot_recovery/tests.rs`, which calls the `_bounded` entry point with an
/// explicit small page size — for the day the underlying read is bounded and
/// paging stops being a multiplier.
pub async fn sweep_unclosed_background_edges<S>(
    resolver: &AwaitEdgeResolver<S>,
    store: &AwaitEdgeStore,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized,
{
    let page_size = u32::try_from(BOOT_SWEEP_SCAN_CAP).unwrap_or(u32::MAX);
    sweep_unclosed_background_edges_bounded(
        resolver,
        store,
        BOOT_SWEEP_SCAN_CAP,
        MAX_BOOT_SWEEP_EDGES,
        page_size,
    )
    .await
}

/// `sweep_unclosed_background_edges` with the scan cap, delivery cap, and
/// scan page size all as parameters instead of the fixed
/// [`BOOT_SWEEP_SCAN_CAP`]/[`MAX_BOOT_SWEEP_EDGES`] (and, for page size, the
/// scan cap itself) — exists so `boot_recovery/tests.rs` can prove the
/// round-robin fairness contract with a handful of edges instead of needing
/// thousands of real journal rows to force the caps, and can independently
/// force multi-page scanning by passing a `page_size` smaller than
/// `scan_cap` without paying that cost in production. The public function
/// above is the only production entry point.
async fn sweep_unclosed_background_edges_bounded<S>(
    resolver: &AwaitEdgeResolver<S>,
    store: &AwaitEdgeStore,
    scan_cap: usize,
    max_delivered: usize,
    page_size: u32,
) -> ResolveReport
where
    S: SessionThreadService + ?Sized,
{
    let mut report = ResolveReport::default();

    // Accumulate every scanned record up front (bounded by `scan_cap`),
    // before any delivery — round-robin fairness needs the whole
    // accumulated set bucketed by scope, not a running window over one page
    // at a time (see `BOOT_SWEEP_SCAN_CAP`'s doc comment for why this cap is
    // deliberately larger than the delivery cap below).
    let mut scanned: Vec<(TurnRunId, TurnRunId, Option<String>, AwaitEdge)> = Vec::new();
    let mut after = None;
    loop {
        let remaining = scan_cap.saturating_sub(scanned.len());
        if remaining == 0 {
            break;
        }
        let page_limit = u32::try_from(remaining.min(page_size as usize)).unwrap_or(page_size);
        let page = match store
            .scan_unclosed_background(boot_sweep_states(), page_limit, after)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::debug!(error = %error, "boot sweep scan failed");
                report.record_failed();
                break;
            }
        };
        let (edges, next_after) = page;
        let page_was_empty = edges.is_empty();
        scanned.extend(edges);
        after = next_after;
        if after.is_none() || page_was_empty {
            break;
        }
    }

    // Defensive fallback for a record with no `group_ref` at all — shouldn't
    // occur given the `"bg:"` prefix filter above, but a missing key must
    // still land in a well-defined bucket rather than panic.
    let mut order: Vec<String> = Vec::new();
    let mut buckets: HashMap<String, VecDeque<(TurnRunId, TurnRunId, AwaitEdge)>> = HashMap::new();
    for (parent_run_id, child_run_id, group_ref, edge) in scanned {
        let key = group_ref.unwrap_or_default();
        buckets
            .entry(key.clone())
            .or_insert_with(|| {
                order.push(key.clone());
                VecDeque::new()
            })
            .push_back((parent_run_id, child_run_id, edge));
    }

    let mut delivered_count = 0usize;
    'rounds: loop {
        let mut delivered_any = false;
        for key in &order {
            if delivered_count >= max_delivered {
                break 'rounds;
            }
            let Some(queue) = buckets.get_mut(key) else {
                continue;
            };
            let Some((parent_run_id, child_run_id, edge)) = queue.pop_front() else {
                continue;
            };
            delivered_any = true;
            delivered_count += 1;
            let outcome = match edge.state {
                // The scan's `states` filter already excludes these; kept as
                // a defensive no-op rather than an unreachable panic if the
                // filter ever widens.
                AwaitEdgeState::Open
                | AwaitEdgeState::Drained
                | AwaitEdgeState::Abandoned
                | AwaitEdgeState::AttentionDeferredStreakCap => continue,
                AwaitEdgeState::Settled
                | AwaitEdgeState::ResultAppended
                | AwaitEdgeState::AttentionScheduled => {
                    resolver
                        .deliver_background(&edge, parent_run_id, child_run_id, false)
                        .await
                }
            };
            match outcome {
                Ok(outcome) => report.record(outcome),
                Err(error) => {
                    tracing::debug!(
                        error = %error,
                        %parent_run_id,
                        %child_run_id,
                        "boot sweep delivery failed for one edge"
                    );
                    report.record_failed();
                }
            }
        }
        if !delivered_any {
            break;
        }
    }

    tracing::debug!(
        resumed = report.resumed,
        drained = report.drained,
        abandoned = report.abandoned,
        already_closed = report.already_closed,
        failed = report.failed,
        "boot sweep pass complete"
    );
    report
}

/// Spawn-side compatibility adapter. Recovery no longer needs a roster or an
/// admission cache: unresolved dependencies are authoritative journal
/// projections and every close transition is idempotent.
pub struct ScopeRecoveryDriver<S: SessionThreadService + ?Sized> {
    resolver: Arc<AwaitEdgeResolver<S>>,
    store: Arc<AwaitEdgeStore>,
}

impl<S> ScopeRecoveryDriver<S>
where
    S: SessionThreadService + ?Sized,
{
    pub fn new(resolver: Arc<AwaitEdgeResolver<S>>, store: Arc<AwaitEdgeStore>) -> Self {
        Self { resolver, store }
    }
}

#[async_trait::async_trait]
impl<S> AwaitEdgeWriter for ScopeRecoveryDriver<S>
where
    S: SessionThreadService + ?Sized + 'static,
{
    async fn check_scope_recovered(
        &self,
        scope: &TurnScope,
    ) -> Result<(), ironclaw_loop_host::ScopeRecoveryInProgress> {
        let report = recover_scope(&self.resolver, &self.store, scope).await;
        if report.failed > 0 {
            return Err(ironclaw_loop_host::ScopeRecoveryInProgress {
                retry_after_hint: std::time::Duration::from_millis(50),
            });
        }
        Ok(())
    }

    async fn abandon_awaited_child(
        &self,
        child_scope: &TurnScope,
        parent_run_id: ironclaw_turns::TurnRunId,
        child_run_id: ironclaw_turns::TurnRunId,
    ) -> Result<(), AgentLoopHostError> {
        self.store
            .abandon_awaited_child(child_scope, parent_run_id, child_run_id)
            .await
    }
}

#[cfg(test)]
mod tests;
