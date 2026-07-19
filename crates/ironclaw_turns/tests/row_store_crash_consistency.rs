// arch-exempt: large_file, self-contained crash-consistency chaos suite (fault backend + generator + oracle + regressions), plan #6263
//! Phase-0 crash-consistency property / chaos-monkey suite for the turn-state
//! row store (`FilesystemTurnStateRowStore`).
//!
//! ## Why this file exists (issue #6263, Step 3 prerequisite)
//!
//! The row store is a write-ahead-log durable store: every mutation delegates
//! to the embedded `InMemoryTurnStateStore` engine, diffs the resulting
//! snapshot into a typed delta, appends that delta to a single-writer journal,
//! and (today) awaits the durable ack before returning `Ok` — i.e. it is
//! **write-through**. A background task materializes the journal tail into
//! per-record rows; on restart, `load_snapshot_from_rows` replays the
//! un-materialized tail.
//!
//! Issue #6263 Step 3 will make non-critical transitions **async write-behind**
//! (ack before durable), keeping only gate-park + terminal transitions
//! synchronously durable. That change is only safe with a crash-consistency
//! oracle in place *first*. This suite is that oracle. It MUST be green on the
//! current write-through store and becomes the acceptance gate for Step 3.
//!
//! ## Architecture
//!
//! * [`FaultBackend`] — a `RootFilesystem` wrapping `InMemoryBackend` that can
//!   (a) inject a write failure (simulated disk-full / backend error) on the
//!   Nth mutating write, on the next journal append, or at a path prefix, and
//!   (b) record every applied mutation so [`FaultBackend::fork_durable_bytes`]
//!   can reconstruct an independent byte-identical copy of the durable state
//!   at a chosen moment ("crash at moment T").
//! * **Crash primitive** — dropping the store instance and reopening a fresh
//!   `FilesystemTurnStateRowStore` over the *same* durable backend bytes drops
//!   the in-memory snapshot cache and the in-flight journal, forcing recovery
//!   through `load_snapshot_from_rows` (the exact pattern the sibling contract
//!   suite uses via `strict_row_store`).
//! * **Reference model** — a second, direct `InMemoryTurnStateStore` (the row
//!   store's own engine) driven with the *same* requests. Every op the row
//!   store acked (`Ok`) is applied to the model; ops it rejected (domain error
//!   or injected fault) are not. After every crash+recovery, the recovered
//!   row-store snapshot is projected onto a lease-timing-independent view and
//!   diffed against the model's — the cleanest oracle, since the engine is
//!   shared. Lease-timing invariants (no double-claim, expiry) that a
//!   full-snapshot diff cannot express are checked explicitly.
//!
//! Determinism: every choice is drawn from a seeded `StdRng`. The seed and the
//! structural op log are printed on any failure so a counterexample replays.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rand::{RngExt, SeedableRng, rngs::StdRng};

use ironclaw_filesystem::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, EventRecord, FileStat, FilesystemError,
    FilesystemOperation, InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem, SeqNo,
    VersionedEntry,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, TenantId, ThreadId,
    UserId, VirtualPath,
};
use ironclaw_turns::{
    AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, CancelRunRequest,
    FilesystemTurnStateRowStore, GateRef, GetRunStateRequest, IdempotencyKey,
    InMemoryRunProfileResolver, InMemoryTurnStateStore, InMemoryTurnStateStoreLimits,
    ReplyTargetBindingRef, ResumeTurnPrecondition, ResumeTurnRequest, RunProfileRequest,
    SanitizedCancelReason, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnActor, TurnCheckpointId, TurnError, TurnLeaseToken,
    TurnPersistenceSnapshot, TurnRunId, TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    run_profile::LoopCheckpointStateRef,
    runner::{
        BlockRunRequest, ClaimRunRequest, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecoverExpiredLeasesRequest, TurnRunTransitionPort,
    },
};

// ─────────────────────────────────────────────────────────────────────────────
// Fault-injecting backend
// ─────────────────────────────────────────────────────────────────────────────

/// A mutation recorded for byte-state reconstruction. Only successfully-applied
/// mutations are recorded, so replaying them into a fresh `InMemoryBackend`
/// reproduces a byte-identical durable state (per-path CAS versions and per-log
/// sequence numbers are deterministic in application order).
#[derive(Clone)]
enum RecordedOp {
    Put {
        path: VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    },
    Delete {
        path: VirtualPath,
    },
    DeleteIfVersion {
        path: VirtualPath,
        version: RecordVersion,
    },
    Append {
        path: VirtualPath,
        payload: Vec<u8>,
    },
    AppendBatch {
        path: VirtualPath,
        payloads: Vec<Vec<u8>>,
    },
    ReserveSequence {
        path: VirtualPath,
    },
}

#[derive(Default)]
struct FaultConfig {
    /// Fail the mutating write whose 1-based index equals this value.
    fail_at_write: Option<usize>,
    /// Number of upcoming `append`/`append_batch` calls to fail (one-shot
    /// countdown). Models a crash while flushing the journal batch.
    fail_next_appends: usize,
    /// Fail writes whose path contains this substring.
    fail_path_substr: Option<String>,
}

/// `RootFilesystem` wrapper over `InMemoryBackend` with write-fault injection
/// and mutation recording (for byte-state forks). Concrete over
/// `InMemoryBackend` because that is the only backend under test.
struct FaultBackend {
    inner: InMemoryBackend,
    write_count: AtomicUsize,
    config: StdMutex<FaultConfig>,
    recorded: StdMutex<Vec<RecordedOp>>,
}

impl FaultBackend {
    fn new(inner: InMemoryBackend) -> Self {
        Self {
            inner,
            write_count: AtomicUsize::new(0),
            config: StdMutex::new(FaultConfig::default()),
            recorded: StdMutex::new(Vec::new()),
        }
    }

    fn cfg(&self) -> std::sync::MutexGuard<'_, FaultConfig> {
        self.config.lock().expect("fault config mutex")
    }

    /// Fail the next `n` journal append calls (one-shot).
    fn fail_next_appends(&self, n: usize) {
        self.cfg().fail_next_appends = n;
    }

    /// Fail the mutating write at 1-based index `n` (counting from the current
    /// write count).
    fn fail_at_relative_write(&self, n: usize) {
        let base = self.write_count.load(Ordering::SeqCst);
        self.cfg().fail_at_write = Some(base + n);
    }

    /// Fail writes whose path contains `substr`.
    fn fail_path_substr(&self, substr: impl Into<String>) {
        self.cfg().fail_path_substr = Some(substr.into());
    }

    /// Clear all armed faults.
    fn disarm(&self) {
        *self.cfg() = FaultConfig::default();
    }

    /// Decide whether a mutating write to `path` should be faulted, consuming
    /// one-shot counters. Returns the injected error when it fires.
    fn maybe_fault(&self, path: &VirtualPath, is_append: bool) -> Option<FilesystemError> {
        let index = self.write_count.fetch_add(1, Ordering::SeqCst) + 1;
        let mut cfg = self.cfg();
        if is_append && cfg.fail_next_appends > 0 {
            cfg.fail_next_appends -= 1;
            return Some(injected_error(path, FilesystemOperation::Append));
        }
        if cfg.fail_at_write == Some(index) {
            cfg.fail_at_write = None;
            return Some(injected_error(path, FilesystemOperation::WriteFile));
        }
        if let Some(substr) = &cfg.fail_path_substr
            && path.as_str().contains(substr.as_str())
        {
            return Some(injected_error(path, FilesystemOperation::WriteFile));
        }
        None
    }

    fn record(&self, op: RecordedOp) {
        self.recorded.lock().expect("recorded mutex").push(op);
    }

    fn recorded_ops(&self) -> Vec<RecordedOp> {
        self.recorded.lock().expect("recorded mutex").clone()
    }

    /// Reconstruct an independent, byte-identical copy of the durable state as
    /// of the last recorded (successfully-applied) mutation, and wrap it in a
    /// fresh scoped filesystem a new store can be opened over. This is the
    /// "restore the durable bytes to their state at moment T" primitive.
    async fn fork_durable_bytes(&self) -> Arc<ScopedFilesystem<FaultBackend>> {
        let fresh = InMemoryBackend::new();
        for op in self.recorded_ops() {
            replay_into(&fresh, &op).await;
        }
        fault_scoped(Arc::new(FaultBackend::new(fresh)))
    }
}

fn injected_error(path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::Backend {
        path: path.clone(),
        operation,
        reason: "injected fault (simulated crash / disk-full)".to_string(),
    }
}

async fn replay_into(inner: &InMemoryBackend, op: &RecordedOp) {
    let result = match op {
        RecordedOp::Put { path, entry, cas } => {
            inner.put(path, entry.clone(), *cas).await.map(|_| ())
        }
        RecordedOp::Delete { path } => inner.delete(path).await,
        RecordedOp::DeleteIfVersion { path, version } => {
            inner.delete_if_version(path, *version).await
        }
        RecordedOp::Append { path, payload } => {
            inner.append(path, payload.clone()).await.map(|_| ())
        }
        RecordedOp::AppendBatch { path, payloads } => {
            inner.append_batch(path, payloads.clone()).await.map(|_| ())
        }
        RecordedOp::ReserveSequence { path } => inner.reserve_sequence(path).await.map(|_| ()),
    };
    result.expect("replaying a recorded durable mutation must succeed");
}

#[async_trait]
impl RootFilesystem for FaultBackend {
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if let Some(error) = self.maybe_fault(path, false) {
            return Err(error);
        }
        let version = self.inner.put(path, entry.clone(), cas).await?;
        self.record(RecordedOp::Put {
            path: path.clone(),
            entry,
            cas,
        });
        Ok(version)
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if let Some(error) = self.maybe_fault(path, false) {
            return Err(error);
        }
        self.inner.delete(path).await?;
        self.record(RecordedOp::Delete { path: path.clone() });
        Ok(())
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        if let Some(error) = self.maybe_fault(path, false) {
            return Err(error);
        }
        self.inner.delete_if_version(path, expected_version).await?;
        self.record(RecordedOp::DeleteIfVersion {
            path: path.clone(),
            version: expected_version,
        });
        Ok(())
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        if let Some(error) = self.maybe_fault(path, true) {
            return Err(error);
        }
        let seq = self.inner.append(path, payload.clone()).await?;
        self.record(RecordedOp::Append {
            path: path.clone(),
            payload,
        });
        Ok(seq)
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        if let Some(error) = self.maybe_fault(path, true) {
            return Err(error);
        }
        let seqs = self.inner.append_batch(path, payloads.clone()).await?;
        self.record(RecordedOp::AppendBatch {
            path: path.clone(),
            payloads,
        });
        Ok(seqs)
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.inner.tail(path, from).await
    }

    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.inner.tail_bounded(path, from, max_records).await
    }

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        self.inner.head_seq(path, from).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        if let Some(error) = self.maybe_fault(path, false) {
            return Err(error);
        }
        let seq = self.inner.reserve_sequence(path).await?;
        self.record(RecordedOp::ReserveSequence { path: path.clone() });
        Ok(seq)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Store wiring
// ─────────────────────────────────────────────────────────────────────────────

fn fault_scoped(backend: Arc<FaultBackend>) -> Arc<ScopedFilesystem<FaultBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("turns mount alias"),
        VirtualPath::new("/turns").expect("turns virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("turns mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}

fn limits() -> InMemoryTurnStateStoreLimits {
    // Default limits keep retention/eviction out of play so the row store's
    // durable projection and the direct-engine model evict identically (i.e.
    // not at all) across a short chaos run. Eviction parity is a separate
    // (#6263 gap-5) concern.
    InMemoryTurnStateStoreLimits::default()
}

/// Open a fresh row store in the strict cross-store reservation mode the
/// crash/recovery contract tests use. Reopening over the same `scoped` (same
/// durable bytes) after dropping the previous instance is the crash primitive.
fn open_row_store(
    scoped: Arc<ScopedFilesystem<FaultBackend>>,
) -> FilesystemTurnStateRowStore<FaultBackend> {
    FilesystemTurnStateRowStore::new(scoped)
        .with_limits(limits())
        .with_preappend_row_reservations()
}

/// Lenient opener (no pre-append row reservations): the hosted single-tenant
/// production shape, where run rows are written only by the background
/// materializer. Used to isolate a *materialization* fault from the strict
/// mode's pre-append reservation writes.
fn open_row_store_lenient(
    scoped: Arc<ScopedFilesystem<FaultBackend>>,
) -> FilesystemTurnStateRowStore<FaultBackend> {
    FilesystemTurnStateRowStore::new(scoped).with_limits(limits())
}

fn model_store() -> InMemoryTurnStateStore {
    InMemoryTurnStateStore::with_limits(limits())
}

// ─────────────────────────────────────────────────────────────────────────────
// Request builders (mirroring the sibling contract suite's vocabulary)
// ─────────────────────────────────────────────────────────────────────────────

const RECEIVED_AT_YMD: (i32, u32, u32, u32, u32, u32) = (2026, 5, 17, 12, 0, 0);

fn scopes() -> Vec<TurnScope> {
    // 2 tenants × 2 threads = 4 canonical scopes.
    let mut out = Vec::new();
    for tenant in ["tenant-a", "tenant-b"] {
        for thread in ["thread-0", "thread-1"] {
            out.push(TurnScope::new(
                TenantId::new(tenant).unwrap(),
                Some(AgentId::new("agent1").unwrap()),
                Some(ProjectId::new("project1").unwrap()),
                ThreadId::new(thread).unwrap(),
            ));
        }
    }
    out
}

fn turn_actor() -> TurnActor {
    TurnActor::new(UserId::new("user1").unwrap())
}

fn submit_request(scope: TurnScope, run_id: TurnRunId, idem: &str) -> SubmitTurnRequest {
    let (y, mo, d, h, mi, s) = RECEIVED_AT_YMD;
    SubmitTurnRequest {
        requested_model: None,
        scope,
        actor: turn_actor(),
        accepted_message_ref: AcceptedMessageRef::new(format!("message-{idem}")).unwrap(),
        source_binding_ref: SourceBindingRef::new("source-web").unwrap(),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply-web").unwrap(),
        requested_run_profile: Some(RunProfileRequest::new("default").unwrap()),
        idempotency_key: IdempotencyKey::new(idem).unwrap(),
        received_at: Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap(),
        requested_run_id: Some(run_id),
        parent_run_id: None,
        subagent_depth: 0,
        spawn_tree_root_run_id: None,
        product_context: None,
    }
}

fn gate_ref(tag: &str) -> GateRef {
    GateRef::new(format!("gate-{tag}")).unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// Operation plan + application
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RunHandle {
    run_id: TurnRunId,
    scope_idx: usize,
    /// The submit idempotency key this run was created under, so the generator
    /// can emit an idempotent-replay submit (a duplicate key → `Ok` with an
    /// empty targeted delta — one of the no-op input classes #6263 fixed).
    idem: String,
    runner_id: Option<TurnRunnerId>,
    lease_token: Option<TurnLeaseToken>,
    gate: Option<GateRef>,
}

/// A fully-materialized operation. Built once from the seeded RNG, then applied
/// verbatim to both the row store and the reference model so they stay in
/// lockstep.
#[derive(Clone)]
enum Plan {
    Submit {
        scope_idx: usize,
        run_id: TurnRunId,
        idem: String,
    },
    Claim {
        runner_id: TurnRunnerId,
        lease_token: TurnLeaseToken,
        scope_idx: Option<usize>,
    },
    Heartbeat {
        run_idx: usize,
    },
    Block {
        run_idx: usize,
        checkpoint_id: TurnCheckpointId,
        gate: GateRef,
        auth: bool,
    },
    Resume {
        run_idx: usize,
        idem: String,
    },
    Complete {
        run_idx: usize,
    },
    Fail {
        run_idx: usize,
    },
    Cancel {
        run_idx: usize,
        idem: String,
    },
    Recover {
        expire_all: bool,
    },
}

impl Plan {
    /// A stable, seed-reproducible one-line description for the failure log.
    fn describe(&self) -> String {
        match self {
            Plan::Submit {
                scope_idx, idem, ..
            } => format!("submit(scope={scope_idx}, idem={idem})"),
            Plan::Claim { scope_idx, .. } => format!("claim(scope={scope_idx:?})"),
            Plan::Heartbeat { run_idx } => format!("heartbeat(run#{run_idx})"),
            Plan::Block { run_idx, auth, .. } => format!("block(run#{run_idx}, auth={auth})"),
            Plan::Resume { run_idx, .. } => format!("resume(run#{run_idx})"),
            Plan::Complete { run_idx } => format!("complete(run#{run_idx})"),
            Plan::Fail { run_idx } => format!("fail(run#{run_idx})"),
            Plan::Cancel { run_idx, .. } => format!("cancel(run#{run_idx})"),
            Plan::Recover { expire_all } => format!("recover(expire_all={expire_all})"),
        }
    }
}

/// The observable effect of applying a plan, used for lockstep cross-checks.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Submitted,
    Claimed(Option<TurnRunId>),
    Transitioned,
    Recovered,
}

/// Apply a plan to any store implementing the turn-state surface. `handles`
/// supplies the tracked run identities/leases the plan indexes into.
async fn apply<S>(
    store: &S,
    plan: &Plan,
    scope_list: &[TurnScope],
    handles: &[RunHandle],
) -> Result<Effect, TurnError>
where
    S: TurnStateStore + TurnRunTransitionPort + Sync,
{
    match plan {
        Plan::Submit {
            scope_idx,
            run_id,
            idem,
        } => {
            let request = submit_request(scope_list[*scope_idx].clone(), *run_id, idem);
            store
                .submit_turn(
                    request,
                    &AllowAllTurnAdmissionPolicy,
                    &InMemoryRunProfileResolver::default(),
                )
                .await
                .map(|_| Effect::Submitted)
        }
        Plan::Claim {
            runner_id,
            lease_token,
            scope_idx,
        } => {
            let request = ClaimRunRequest {
                runner_id: *runner_id,
                lease_token: *lease_token,
                scope_filter: scope_idx.map(|i| scope_list[i].clone()),
            };
            let claimed = store.claim_next_run(request).await?;
            Ok(Effect::Claimed(claimed.map(|c| c.state.run_id)))
        }
        Plan::Heartbeat { run_idx } => {
            let h = &handles[*run_idx];
            store
                .heartbeat(HeartbeatRequest {
                    run_id: h.run_id,
                    runner_id: h.runner_id.unwrap_or_default(),
                    lease_token: h.lease_token.unwrap_or_default(),
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Block {
            run_idx,
            checkpoint_id,
            gate,
            auth,
        } => {
            let h = &handles[*run_idx];
            let reason = if *auth {
                BlockedReason::Auth {
                    gate_ref: gate.clone(),
                    credential_requirements: Vec::new(),
                }
            } else {
                BlockedReason::Approval {
                    gate_ref: gate.clone(),
                }
            };
            store
                .block_run(BlockRunRequest {
                    run_id: h.run_id,
                    runner_id: h.runner_id.unwrap_or_default(),
                    lease_token: h.lease_token.unwrap_or_default(),
                    checkpoint_id: *checkpoint_id,
                    state_ref: LoopCheckpointStateRef::new("checkpoint:crash-block").unwrap(),
                    reason,
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Resume { run_idx, idem } => {
            let h = &handles[*run_idx];
            let gate = h
                .gate
                .clone()
                .unwrap_or_else(|| gate_ref("resume-fallback"));
            store
                .resume_turn(ResumeTurnRequest {
                    scope: scope_list[h.scope_idx].clone(),
                    actor: turn_actor(),
                    run_id: h.run_id,
                    gate_resolution_ref: gate,
                    source_binding_ref: SourceBindingRef::new("source-resume").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply-resume").unwrap(),
                    idempotency_key: IdempotencyKey::new(idem).unwrap(),
                    precondition: ResumeTurnPrecondition::AnyBlockedGate,
                    resume_disposition: None,
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Complete { run_idx } => {
            let h = &handles[*run_idx];
            store
                .complete_run(CompleteRunRequest {
                    run_id: h.run_id,
                    runner_id: h.runner_id.unwrap_or_default(),
                    lease_token: h.lease_token.unwrap_or_default(),
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Fail { run_idx } => {
            let h = &handles[*run_idx];
            store
                .fail_run(FailRunRequest {
                    run_id: h.run_id,
                    runner_id: h.runner_id.unwrap_or_default(),
                    lease_token: h.lease_token.unwrap_or_default(),
                    failure: SanitizedFailure::new("chaos_failure").unwrap(),
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Cancel { run_idx, idem } => {
            let h = &handles[*run_idx];
            store
                .request_cancel(CancelRunRequest {
                    scope: scope_list[h.scope_idx].clone(),
                    actor: turn_actor(),
                    run_id: h.run_id,
                    reason: SanitizedCancelReason::UserRequested,
                    idempotency_key: IdempotencyKey::new(idem).unwrap(),
                })
                .await
                .map(|_| Effect::Transitioned)
        }
        Plan::Recover { expire_all } => {
            let now = if *expire_all {
                Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap()
            } else {
                Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap()
            };
            store
                .recover_expired_leases(RecoverExpiredLeasesRequest {
                    now,
                    scope_filter: None,
                })
                .await
                .map(|_| Effect::Recovered)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Normalized projection + invariant checks (the oracle)
// ─────────────────────────────────────────────────────────────────────────────

/// A lease-timing-independent view of the durable state. Every volatile field
/// (lease token, all timestamps, heartbeat/expiry) is excluded, so a recovered
/// row store and the direct-engine model that received the same acked ops must
/// project identically. This is the crash-consistency contract Step 3 must not
/// break.
#[derive(Debug, PartialEq, Eq)]
struct Projection {
    // Turn *count*, not the turn ids: `TurnId` is minted internally by each
    // engine instance on submit, so the row store's embedded engine and the
    // separate reference-model engine legitimately assign different turn ids
    // for the same submit. Cross-store equality is keyed on the caller-supplied
    // run ids; per-snapshot run→turn integrity is checked separately in
    // `check_internal_invariants`.
    turn_count: usize,
    runs: BTreeMap<String, RunProjection>,
    active_locks: BTreeMap<String, (String, TurnStatus)>,
    idempotency: BTreeMap<String, (Option<String>, String)>,
    // Per scope, the ordered sequence of `(kind, run_id)` — ordered BY cursor
    // but with the cursor *value* dropped. Absolute cursor numbers are not
    // compared cross-store: when a `recover_expired_leases` pass fails several
    // runs at once, the engine assigns their event cursors in HashMap-iteration
    // order, so two independent engine instances number the same events
    // differently. Each scope has ≤1 run (active-run exclusivity), so the
    // per-scope order is deterministic; cursor *monotonicity* within a store is
    // asserted separately in `check_internal_invariants`.
    events: BTreeMap<String, Vec<(String, String)>>,
    checkpoints: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq, Eq)]
struct RunProjection {
    scope_key: String,
    status: TurnStatus,
    gate_present: bool,
    checkpoint_present: bool,
    parent: Option<String>,
    failure_present: bool,
}

fn scope_key(scope: &TurnScope) -> String {
    serde_json::to_string(scope).expect("serialize scope")
}

fn project(snapshot: &TurnPersistenceSnapshot) -> Projection {
    let turn_count = snapshot.turns.len();

    let runs = snapshot
        .runs
        .iter()
        .map(|run| {
            (
                run.run_id.to_string(),
                RunProjection {
                    scope_key: scope_key(&run.scope),
                    status: run.status,
                    gate_present: run.gate_ref.is_some(),
                    checkpoint_present: run.checkpoint_id.is_some(),
                    parent: run.parent_run_id.map(|id| id.to_string()),
                    failure_present: run.failure.is_some(),
                },
            )
        })
        .collect();

    let active_locks = snapshot
        .active_locks
        .iter()
        .map(|lock| {
            (
                serde_json::to_string(&lock.key).expect("serialize lock key"),
                (lock.run_id.to_string(), lock.status),
            )
        })
        .collect();

    let idempotency = snapshot
        .idempotency_records
        .iter()
        .map(|record| {
            let key = format!(
                "{}|{:?}|{}",
                scope_key(&record.scope),
                record.operation,
                serde_json::to_string(&record.key).expect("serialize idem key"),
            );
            (
                key,
                (
                    record.run_id.map(|id| id.to_string()),
                    format!("{:?}", record.outcome),
                ),
            )
        })
        .collect();

    let mut cursored: BTreeMap<String, Vec<(u64, String, String)>> = BTreeMap::new();
    for event in &snapshot.events {
        cursored.entry(scope_key(&event.scope)).or_default().push((
            event.cursor.0,
            format!("{:?}", event.kind),
            event.run_id.to_string(),
        ));
    }
    let events = cursored
        .into_iter()
        .map(|(scope, mut series)| {
            series.sort_by_key(|(cursor, _, _)| *cursor);
            let ordered = series
                .into_iter()
                .map(|(_, kind, run)| (kind, run))
                .collect();
            (scope, ordered)
        })
        .collect();

    let checkpoints = snapshot
        .checkpoints
        .iter()
        .map(|cp| {
            (
                cp.checkpoint_id.as_uuid().to_string(),
                cp.run_id.to_string(),
            )
        })
        .collect();

    Projection {
        turn_count,
        runs,
        active_locks,
        idempotency,
        events,
        checkpoints,
    }
}

/// Structural invariants that must hold on any recovered snapshot on its own,
/// independent of the reference model.
fn check_internal_invariants(snapshot: &TurnPersistenceSnapshot) -> Result<(), String> {
    let run_ids: BTreeSet<TurnRunId> = snapshot.runs.iter().map(|r| r.run_id).collect();
    let turn_ids: BTreeSet<String> = snapshot
        .turns
        .iter()
        .map(|t| t.turn_id.to_string())
        .collect();

    // No active-lock row without a matching run, and never a lock on a run that
    // has reached a terminal status (a terminal transition releases the lock).
    for lock in &snapshot.active_locks {
        let Some(run) = snapshot.runs.iter().find(|r| r.run_id == lock.run_id) else {
            return Err(format!(
                "active lock {:?} references missing run {}",
                lock.key, lock.run_id
            ));
        };
        if run.status.is_terminal() {
            return Err(format!(
                "active lock held for terminal run {} (status {:?})",
                run.run_id, run.status
            ));
        }
    }

    // Every run resolves to a durable turn row (turn-ref integrity).
    for run in &snapshot.runs {
        if !turn_ids.contains(&run.turn_id.to_string()) {
            return Err(format!(
                "run {} references missing turn row {}",
                run.run_id, run.turn_id
            ));
        }
    }

    // Turn-checkpoint refs resolve to a live run.
    for cp in &snapshot.checkpoints {
        if !run_ids.contains(&cp.run_id) {
            return Err(format!(
                "checkpoint {} references missing run {}",
                cp.checkpoint_id.as_uuid(),
                cp.run_id
            ));
        }
    }

    // Loop-checkpoint refs resolve to a live run.
    for cp in &snapshot.loop_checkpoints {
        if !run_ids.contains(&cp.run_id) {
            return Err(format!(
                "loop checkpoint references missing run {}",
                cp.run_id
            ));
        }
    }

    // Idempotency records that name a run must resolve to a durable run.
    for record in &snapshot.idempotency_records {
        if let Some(run_id) = record.run_id
            && !run_ids.contains(&run_id)
        {
            return Err(format!(
                "idempotency record ({:?}) references missing run {}",
                record.operation, run_id
            ));
        }
    }

    // Event cursor is strictly monotonic (no dupes / no rewind) within a scope.
    let mut per_scope: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for event in &snapshot.events {
        per_scope
            .entry(scope_key(&event.scope))
            .or_default()
            .push(event.cursor.0);
    }
    for (scope, cursors) in &per_scope {
        let mut sorted = cursors.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != cursors.len() {
            return Err(format!("duplicate event cursor within scope {scope}"));
        }
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Chaos harness
// ─────────────────────────────────────────────────────────────────────────────

struct Harness {
    rng: StdRng,
    seed: u64,
    scope_list: Vec<TurnScope>,
    handles: Vec<RunHandle>,
    unique: u64,
    log: Vec<String>,
}

impl Harness {
    fn new(seed: u64) -> Self {
        Self {
            rng: StdRng::seed_from_u64(seed),
            seed,
            scope_list: scopes(),
            handles: Vec::new(),
            unique: 0,
            log: Vec::new(),
        }
    }

    fn next_idem(&mut self) -> String {
        self.unique += 1;
        format!("idem-{}-{}", self.seed, self.unique)
    }

    /// A fresh submit plan (new run id + unique idempotency key). Used both by
    /// the generator and as the fallback when a claim/recover would be a no-op.
    fn fresh_submit(&mut self) -> Plan {
        let scope_idx = self.rng.random_range(0..self.scope_list.len());
        Plan::Submit {
            scope_idx,
            run_id: TurnRunId::new(),
            idem: self.next_idem(),
        }
    }

    /// A replay of an existing run's submit (same scope + run id + idempotency
    /// key), so the store sees an idempotent-replay submit — `Ok` with an empty
    /// targeted delta, one of the no-op input classes #6263 fixed.
    fn replay_submit(&self, run_idx: usize) -> Plan {
        let handle = &self.handles[run_idx];
        Plan::Submit {
            scope_idx: handle.scope_idx,
            run_id: handle.run_id,
            idem: handle.idem.clone(),
        }
    }

    /// Build the next operation from the seeded RNG.
    ///
    /// Both fresh submits (unique key) and idempotent-replay submits (a duplicate
    /// key) are generated. An idempotent replay returns `Ok` with an empty
    /// targeted delta — a no-op input the store must handle without desyncing the
    /// journal reservation sequence (#6263, pinned by
    /// `noop_claim_does_not_leak_active_lock_across_crash`). Idempotency across
    /// crashes is covered by `idempotency_key_replays_same_run_after_crash`.
    fn plan_next(&mut self) -> Plan {
        let scope_count = self.scope_list.len();
        // With no runs yet, the only meaningful op is a submit.
        if self.handles.is_empty() {
            return self.fresh_submit();
        }
        let roll = self.rng.random_range(0..100u32);
        let run_idx = self.rng.random_range(0..self.handles.len());
        match roll {
            0..=24 => self.fresh_submit(),
            25..=27 => self.replay_submit(run_idx),
            28..=47 => {
                // Always claim WITH a scope filter. Active-run exclusivity keeps
                // at most one Queued run per scope, so a scope-filtered claim
                // selects a unique, deterministic run — whereas an unfiltered
                // claim pops the FIFO queue front, whose order is not preserved
                // across a crash (the queue is rebuilt from row files in
                // backend-map order). That ordering difference is a fairness
                // property, not a durability one, so we avoid depending on it.
                let scope_idx = Some(self.rng.random_range(0..scope_count));
                Plan::Claim {
                    runner_id: TurnRunnerId::new(),
                    lease_token: TurnLeaseToken::new(),
                    scope_idx,
                }
            }
            48..=61 => {
                let auth = self.rng.random_bool(0.5);
                let tag = self.next_idem();
                Plan::Block {
                    run_idx,
                    checkpoint_id: TurnCheckpointId::new(),
                    gate: gate_ref(&tag),
                    auth,
                }
            }
            62..=71 => Plan::Resume {
                run_idx,
                idem: self.next_idem(),
            },
            72..=81 => Plan::Complete { run_idx },
            82..=88 => Plan::Fail { run_idx },
            89..=94 => Plan::Cancel {
                run_idx,
                idem: self.next_idem(),
            },
            95..=97 => Plan::Heartbeat { run_idx },
            _ => {
                let expire_all = self.rng.random_bool(0.5);
                Plan::Recover { expire_all }
            }
        }
    }
}

/// Compare the recovered row store against the model, and assert structural
/// invariants. Panics with the seed + op log on any violation.
async fn assert_recovered_matches_model(
    recovered: &FilesystemTurnStateRowStore<FaultBackend>,
    model: &InMemoryTurnStateStore,
    seed: u64,
    log: &[String],
) {
    let recovered_snapshot = recovered
        .persistence_snapshot()
        .await
        .expect("recovered snapshot");
    let model_snapshot = model.persistence_snapshot();

    if let Err(violation) = check_internal_invariants(&recovered_snapshot) {
        panic!(
            "internal invariant violated after recovery: {violation}\nseed={seed}\nops:\n  {}",
            log.join("\n  ")
        );
    }

    // The recoverability-critical set (gate-park + terminal) must survive
    // EVERY crash once acked — this is the boundary #6263 Step 3 keeps
    // synchronously durable. Assert it explicitly and separately *before* the
    // full projection diff, so that when Step 3 relaxes "acked ⇒ durable" to
    // non-critical transitions only, exactly this named check stays and the
    // full-projection diff below is the one scoped down.
    assert_recoverability_critical_survives(&recovered_snapshot, &model_snapshot, seed, log);

    let recovered_projection = project(&recovered_snapshot);
    let model_projection = project(&model_snapshot);
    assert!(
        recovered_projection == model_projection,
        "recovered row store diverged from the acked reference model.\n\
         seed={seed}\nops:\n  {}\n\nRECOVERED:\n{recovered_projection:#?}\n\nMODEL:\n{model_projection:#?}",
        log.join("\n  ")
    );
}

/// #6263 Step 3 boundary (aligned with the #6284 error-recoverability
/// contract). A transition is **recoverability-critical** — and therefore must
/// stay synchronously durable even after async write-behind lands — when its
/// resulting status is a gate-park (Blocked*) or a terminal (Completed / Failed
/// / Cancelled / RecoveryRequired):
///
/// * losing a gate-park strands a run away from the human who must act on it;
/// * losing a Completed re-runs an already-performed side effect;
/// * losing a Failed loses the sanitized cause the model must see to explain
///   or retry.
///
/// Step 3 may move only NON-critical transitions off the synchronous ack; this
/// predicate is the single boundary it flips.
fn is_recoverability_critical(status: TurnStatus) -> bool {
    status.is_blocked() || status.is_terminal()
}

/// Assert that every run the model holds in a recoverability-critical status is
/// present in the recovered snapshot with the *same* status, and that the
/// model-visible failure cause / gate metadata survived. Named separately from
/// the full projection diff so Step 3 can relax the latter without weakening
/// this always-durable guarantee.
fn assert_recoverability_critical_survives(
    recovered: &TurnPersistenceSnapshot,
    model: &TurnPersistenceSnapshot,
    seed: u64,
    log: &[String],
) {
    for model_run in &model.runs {
        if !is_recoverability_critical(model_run.status) {
            continue;
        }
        let recovered_run = recovered
            .runs
            .iter()
            .find(|run| run.run_id == model_run.run_id)
            .unwrap_or_else(|| {
                panic!(
                    "recoverability-critical run {} ({:?}) was LOST across recovery\nseed={seed}\nops:\n  {}",
                    model_run.run_id,
                    model_run.status,
                    log.join("\n  ")
                )
            });
        assert!(
            recovered_run.status == model_run.status,
            "recoverability-critical run {} changed status across recovery: model {:?} vs recovered {:?}\nseed={seed}\nops:\n  {}",
            model_run.run_id,
            model_run.status,
            recovered_run.status,
            log.join("\n  ")
        );
        if model_run.status.is_blocked() {
            assert!(
                recovered_run.gate_ref.is_some() == model_run.gate_ref.is_some(),
                "gate-parked run {} lost its gate ref across recovery\nseed={seed}",
                model_run.run_id
            );
        }
        if model_run.status == TurnStatus::Failed {
            // #6284 (b)(c): the sanitized, model-visible failure cause must
            // survive so the run can be explained/retried, never silently lost.
            assert!(
                recovered_run.failure.is_some() == model_run.failure.is_some(),
                "Failed run {} lost its failure record across recovery\nseed={seed}",
                model_run.run_id
            );
            assert!(
                recovered_run
                    .failure
                    .as_ref()
                    .and_then(|f| f.detail().map(str::to_owned))
                    == model_run
                        .failure
                        .as_ref()
                        .and_then(|f| f.detail().map(str::to_owned)),
                "Failed run {} lost its model-visible failure detail across recovery\nseed={seed}",
                model_run.run_id
            );
        }
    }
}

/// Drive `ops` operations against a fresh store, crashing+recovering every
/// `crash_every` ops, optionally arming a write fault before some ops. Green iff
/// the write-through store never loses an acked op and never violates an
/// invariant across crashes.
async fn run_chaos(seed: u64, ops: usize, crash_every: usize, inject_faults: bool) {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let mut store = open_row_store(Arc::clone(&scoped));
    let model = model_store();

    let mut h = Harness::new(seed);

    for op_index in 0..ops {
        // Periodically drive a fault through the op, expecting the caller to
        // observe an error (write-through: not durably applied) and the model
        // to stay put — a crash right after must recover to the model.
        let fault_this_op = inject_faults && op_index % 9 == 4;
        if fault_this_op {
            // Alternate the two "disk-full / backend error" fault shapes: a
            // faulted journal append (mid-flush) and a faulted Nth mutating
            // write (which may bite a strict-mode pre-append row reservation).
            if op_index % 2 == 0 {
                backend.fail_next_appends(1);
            } else {
                backend.fail_at_relative_write(1);
            }
        }

        // Generate the next op verbatim — including no-op inputs (a
        // `claim_next_run` that matches nothing → Ok(None), a
        // `recover_expired_leases` that expires nothing, an idempotent-replay
        // submit, an idempotent `request_cancel`). Each of those goes through the
        // commit path with an empty durable delta; #6263's fix makes that a true
        // no-op that does not advance the reservation sequence, so the suite
        // exercises them directly instead of steering the generator away (the
        // former steering existed only to dodge the desync bug now fixed and
        // pinned by `noop_claim_does_not_leak_active_lock_across_crash`). The
        // oracle/invariants below stay strict.
        let plan = h.plan_next();
        h.log.push(format!("#{op_index} {}", plan.describe()));

        let scope_list = h.scope_list.clone();
        let handles = h.handles.clone();
        let rs_result = apply(&store, &plan, &scope_list, &handles).await;

        if fault_this_op {
            backend.disarm();
        }

        // Advance the reference model only for ops the row store acked. A
        // heartbeat is deliberately excluded: the row store keeps lease
        // liveness in a process-local overlay that bypasses the journal
        // (#5452), so a heartbeat emits no durable event and does not advance
        // the durable event cursor — it must not advance the model's either, or
        // their event streams would diverge purely on a non-durable ping.
        if rs_result.is_ok() && !matches!(plan, Plan::Heartbeat { .. }) {
            apply_to_model_and_bookkeep(&mut h, &model, &plan, &rs_result).await;
        }

        if (op_index + 1) % crash_every == 0 {
            // Crash: drop the store, reopen over the same durable bytes.
            drop(store);
            store = open_row_store(Arc::clone(&scoped));
            assert_recovered_matches_model(&store, &model, seed, &h.log).await;
        }
    }

    // Final crash + full check.
    drop(store);
    let store = open_row_store(Arc::clone(&scoped));
    assert_recovered_matches_model(&store, &model, seed, &h.log).await;
}

/// Apply the acked op to the reference model, assert the model agrees, and
/// update the tracked run handles.
async fn apply_to_model_and_bookkeep(
    h: &mut Harness,
    model: &InMemoryTurnStateStore,
    plan: &Plan,
    rs_result: &Result<Effect, TurnError>,
) {
    let scope_list = h.scope_list.clone();
    let handles = h.handles.clone();
    let model_result = apply(model, plan, &scope_list, &handles).await;

    // The row store accepted this op; the model (same engine, same acked
    // history) must accept it too. A divergence here is a real finding.
    let model_effect = match model_result {
        Ok(effect) => effect,
        Err(error) => panic!(
            "reference model rejected an op the row store acked: {error:?}\nseed={}\nops:\n  {}",
            h.seed,
            h.log.join("\n  ")
        ),
    };
    let rs_effect = rs_result.as_ref().expect("caller guarantees Ok");

    match (plan, rs_effect, &model_effect) {
        (
            Plan::Submit {
                scope_idx,
                run_id,
                idem,
            },
            Effect::Submitted,
            Effect::Submitted,
        ) => {
            // Track the run so later transitions can target it. (A duplicate
            // idempotency key replays to an existing run_id; harmlessly adds a
            // second handle pointing at the requested id — transitions on it
            // simply no-op/err and are skipped, which is fine.)
            h.handles.push(RunHandle {
                run_id: *run_id,
                scope_idx: *scope_idx,
                idem: idem.clone(),
                runner_id: None,
                lease_token: None,
                gate: None,
            });
        }
        (
            Plan::Claim {
                runner_id,
                lease_token,
                ..
            },
            Effect::Claimed(rs_run),
            Effect::Claimed(model_run),
        ) => {
            assert!(
                rs_run == model_run,
                "claim picked different runs (row store {rs_run:?} vs model {model_run:?})\nseed={}\nops:\n  {}",
                h.seed,
                h.log.join("\n  ")
            );
            if let Some(run_id) = rs_run
                && let Some(handle) = h.handles.iter_mut().find(|handle| handle.run_id == *run_id)
            {
                handle.runner_id = Some(*runner_id);
                handle.lease_token = Some(*lease_token);
            }
        }
        (Plan::Block { run_idx, gate, .. }, _, _) => {
            if let Some(handle) = h.handles.get_mut(*run_idx) {
                handle.gate = Some(gate.clone());
            }
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Property entry points (bounded, deterministic, CI-friendly)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn row_store_crash_consistency_property_no_faults() {
    // 10 seeds × 64 ops, crashing every 6 ops — pure crash/recovery consistency
    // against the acked reference model, no injected write faults.
    for seed in [1, 7, 42, 101, 777, 2718, 8191, 31337, 65521, 999983] {
        run_chaos(seed, 64, 6, false).await;
    }
}

#[tokio::test]
async fn row_store_crash_consistency_property_with_faults() {
    // Same shape, but every ~9th op runs with a write fault armed (alternating
    // faulted journal append / faulted Nth write), exercising
    // crash-immediately-after-a-rolled-back-write. The recovered state must
    // still equal the acked model (the faulted op left no durable trace).
    for seed in [3, 13, 99, 500, 4093, 50021, 1234567, 88888] {
        run_chaos(seed, 56, 5, true).await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Targeted regression tests for specific crash points
// ─────────────────────────────────────────────────────────────────────────────

async fn submit_one(
    store: &FilesystemTurnStateRowStore<FaultBackend>,
    scope: &TurnScope,
    idem: &str,
) -> TurnRunId {
    let run_id = TurnRunId::new();
    let request = submit_request(scope.clone(), run_id, idem);
    store
        .submit_turn(
            request,
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect("submit accepted");
    run_id
}

fn only_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-r").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-regression").unwrap(),
    )
}

/// Crash between the durable journal append and background materialization:
/// after an acked submit, drop the store before the materializer has written
/// rows; recovery must replay the un-materialized journal tail.
#[tokio::test]
async fn crash_between_journal_append_and_materialize_recovers_submit() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let run_id = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-append-vs-materialize").await;
        // Drop synchronously — no await between the acked submit and the drop,
        // so the background materializer task cannot have run.
        drop(store);
        run_id
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("acked submit must survive crash before materialization");
    assert_eq!(state.run_id, run_id);
    assert_eq!(state.status, TurnStatus::Queued);
    check_internal_invariants(&recovered.persistence_snapshot().await.unwrap()).unwrap();
}

/// #6284 (II.a) — Durable-write failure is recoverable + atomic (journal
/// append). A committed run A must stay intact and re-drivable; a run B whose
/// journal append is faulted must fail with a *retryable* `Unavailable`, and
/// recovery must show B atomically absent — never a run row without its turn,
/// never an orphan active lock.
#[tokio::test]
async fn crash_mid_flush_batch_leaves_no_durable_trace() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope_a = only_scope();
    let scope_b = TurnScope::new(
        TenantId::new("tenant-r").unwrap(),
        Some(AgentId::new("agent1").unwrap()),
        Some(ProjectId::new("project1").unwrap()),
        ThreadId::new("thread-regression-b").unwrap(),
    );

    // Run A commits durably (no fault).
    let run_a = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_a = submit_one(&store, &scope_a, "idem-mid-flush-a").await;
        drop(store);
        run_a
    };

    // Run B's journal append is faulted.
    let run_b = TurnRunId::new();
    let request = submit_request(scope_b.clone(), run_b, "idem-mid-flush-b");
    backend.fail_next_appends(1);
    let error = {
        let store = open_row_store(Arc::clone(&scoped));
        let result = store
            .submit_turn(
                request,
                &AllowAllTurnAdmissionPolicy,
                &InMemoryRunProfileResolver::default(),
            )
            .await;
        drop(store);
        result.expect_err("faulted journal append must fail the write-through submit")
    };
    assert!(
        matches!(error, TurnError::Unavailable { .. }),
        "a durable-append failure must be a RETRYABLE error the runner can re-drive, got {error:?}"
    );
    backend.disarm();

    let recovered = open_row_store(Arc::clone(&scoped));

    // B left no durable trace.
    let hidden = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope_b.clone(),
            run_id: run_b,
        })
        .await;
    assert!(
        matches!(hidden, Err(TurnError::ScopeNotFound)),
        "a failed durable append must not leave a recoverable run, got {hidden:?}"
    );

    // A survived and is still re-drivable (claimable).
    let a_state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope_a.clone(),
            run_id: run_a,
        })
        .await
        .expect("pre-failure committed run must survive the later faulted write");
    assert_eq!(a_state.status, TurnStatus::Queued);

    let snapshot = recovered.persistence_snapshot().await.unwrap();
    assert_eq!(
        snapshot.runs.len(),
        1,
        "only run A is durable: {snapshot:#?}"
    );
    assert!(
        snapshot.runs.iter().all(|run| run.run_id == run_a),
        "B must be atomically absent"
    );
    // Atomicity: no half-applied B (no turn/lock left behind).
    check_internal_invariants(&snapshot).unwrap();

    let claimed = recovered
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope_a),
        })
        .await
        .unwrap()
        .expect("surviving run A must be re-drivable after the faulted write");
    assert_eq!(claimed.state.run_id, run_a);
}

/// #6284 (II.b) — Durable-write failure is recoverable (mid-materialize). A
/// fault during row materialization must surface a retryable error, leave the
/// durable journal intact (no partial corruption), and a retry must recover the
/// run cleanly and re-drivable.
///
/// Deterministic by construction: a *persistent* fault on every run-row write
/// (`/runs/`) never touches the journal-append (delta-log) path, so the submit
/// is durably acked while the run row can never materialize — no dependence on
/// the 25 ms background-materializer timing. A lenient (non-strict) store is
/// used so the only run-row writes are materializations, not pre-append
/// reservations.
#[tokio::test]
async fn crash_mid_materialize_failure_is_retryable_and_consistent() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();
    let run_id = TurnRunId::new();

    // Persistently fault every run-row materialization write.
    backend.fail_path_substr("/runs/");
    {
        let store = open_row_store_lenient(Arc::clone(&scoped));
        let request = submit_request(scope.clone(), run_id, "idem-mid-materialize");
        store
            .submit_turn(
                request,
                &AllowAllTurnAdmissionPolicy,
                &InMemoryRunProfileResolver::default(),
            )
            .await
            .expect("append-durable submit succeeds even while run-row materialization is faulted");
        drop(store);
    }

    // Reopen with the fault still armed: recovery must materialize the run row,
    // hit the fault, and surface a retryable error rather than corrupt state.
    let faulted = open_row_store_lenient(Arc::clone(&scoped));
    let failed = faulted
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await;
    assert!(
        matches!(failed, Err(TurnError::Unavailable { .. })),
        "a faulted materialization must surface a RETRYABLE error, got {failed:?}"
    );
    drop(faulted);

    // Disarm and retry: the journal was never corrupted, so materialization now
    // completes and the run recovers cleanly and re-drivable.
    backend.disarm();
    let retried = open_row_store_lenient(Arc::clone(&scoped));
    let state = retried
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("run must materialize cleanly on retry after a faulted materialize");
    assert_eq!(state.status, TurnStatus::Queued);
    check_internal_invariants(&retried.persistence_snapshot().await.unwrap()).unwrap();

    let claimed = retried
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope),
        })
        .await
        .unwrap()
        .expect("recovered run must be re-drivable");
    assert_eq!(claimed.state.run_id, run_id);
}

/// Crash immediately after an acked gate-park (block): the blocked turn must
/// survive recovery still blocked, keeping its active lock.
#[tokio::test]
async fn crash_after_acked_gate_park_survives_blocked() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let (run_id, checkpoint_id) = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-gate-park").await;
        let claimed = store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("claim the queued run");
        let checkpoint_id = TurnCheckpointId::new();
        store
            .block_run(BlockRunRequest {
                run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                checkpoint_id,
                state_ref: LoopCheckpointStateRef::new("checkpoint:gate-park").unwrap(),
                reason: BlockedReason::Approval {
                    gate_ref: gate_ref("park"),
                },
            })
            .await
            .expect("gate-park the run");
        drop(store);
        (run_id, checkpoint_id)
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("gate-parked run must survive crash");
    assert_eq!(state.status, TurnStatus::BlockedApproval);

    let snapshot = recovered.persistence_snapshot().await.unwrap();
    assert!(
        snapshot
            .active_locks
            .iter()
            .any(|lock| lock.run_id == run_id),
        "gate-parked run must retain its active lock across recovery"
    );
    assert!(
        snapshot
            .checkpoints
            .iter()
            .any(|cp| cp.checkpoint_id == checkpoint_id),
        "gate-park checkpoint must be durable"
    );
    check_internal_invariants(&snapshot).unwrap();
}

/// No double-claim across a crash: a claimed, lease-valid run is never
/// re-claimable by another runner after recovery. The only escape from the
/// lease is a genuine expiry through `recover_expired_leases`. Per #6284 a
/// checkpoint-less abandoned run (crashed before its first checkpoint = before
/// any side effect) is RE-QUEUED to a claimable state so it re-drives, rather
/// than stranded terminal `Failed`. That is not a double-claim: the prior lease
/// genuinely expired before the re-queue, so two runners never hold the same run
/// at once.
#[tokio::test]
async fn crash_preserves_single_claim_and_lease_expiry_requeues_abandoned_run() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let run_id = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-no-double-claim").await;
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("first claim");
        drop(store);
        run_id
    };

    let recovered = open_row_store(Arc::clone(&scoped));

    // A different runner must NOT be able to re-claim the still-Running run —
    // the crash did not release the lease.
    let stolen = recovered
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert!(
        stolen.is_none(),
        "a Running run must not be re-claimable after crash without lease expiry (got {stolen:?})"
    );

    // Genuine lease expiry resolves the abandoned lease. Per #6284 a
    // checkpoint-less run (crashed before any side effect) is re-queued to a
    // claimable state — re-drivable, NOT stranded terminal — and it is not
    // silently re-handed to another runner while the lease was still valid.
    recovered
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
            scope_filter: None,
        })
        .await
        .expect("recover expired leases");
    let post_recover = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("run still resolvable after lease expiry");
    assert_eq!(
        post_recover.status,
        TurnStatus::Queued,
        "an abandoned checkpoint-less lease re-queues to a re-drivable state (#6284)"
    );
    // A fresh claim now re-drives the re-queued run — this is not a double-claim
    // because the prior lease already expired.
    let after = recovered
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(
        after.map(|claimed| claimed.state.run_id),
        Some(run_id),
        "a re-queued checkpoint-less run must be re-claimable so it re-drives (#6284)"
    );
}

/// Idempotency survives crashes: re-submitting an acked idempotency key after
/// recovery replays the same run id rather than minting a new run.
#[tokio::test]
async fn idempotency_key_replays_same_run_after_crash() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let original = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-crash-replay").await;
        drop(store);
        run_id
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    // Same scope + same idempotency key, different requested run id: must replay.
    let replay = submit_request(scope.clone(), TurnRunId::new(), "idem-crash-replay");
    let response = recovered
        .submit_turn(
            replay,
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect("idempotent re-submit after crash");
    let SubmitTurnResponse::Accepted { run_id, .. } = response;
    assert_eq!(
        run_id, original,
        "re-submitting an acked idempotency key after crash must return the original run id"
    );
}

/// The byte-state fork primitive: an independent copy of the durable bytes as
/// of moment T recovers to the same projection as the model at T, while the
/// live store keeps running.
#[tokio::test]
async fn byte_state_fork_recovers_to_model_snapshot() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let live = open_row_store(Arc::clone(&scoped));
    let model = model_store();
    let scope = only_scope();

    // Apply a few acked ops to both the live store and the model.
    let run_id = TurnRunId::new();
    let submit = submit_request(scope.clone(), run_id, "idem-fork");
    live.submit_turn(
        submit.clone(),
        &AllowAllTurnAdmissionPolicy,
        &InMemoryRunProfileResolver::default(),
    )
    .await
    .unwrap();
    model
        .submit_turn(
            submit,
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();

    for store_kind in 0..2 {
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let request = ClaimRunRequest {
            runner_id,
            lease_token,
            scope_filter: None,
        };
        if store_kind == 0 {
            live.claim_next_run(request).await.unwrap();
        } else {
            model.claim_next_run(request).await.unwrap();
        }
    }

    // Fork the durable bytes as of now and open an independent store over them.
    let forked_scoped = backend.fork_durable_bytes().await;
    let forked = open_row_store(forked_scoped);

    let forked_projection = project(&forked.persistence_snapshot().await.unwrap());
    let model_projection = project(&model.persistence_snapshot());
    assert_eq!(
        forked_projection, model_projection,
        "independent byte-state fork must recover to the acked model projection"
    );
}

/// #6284 (I) — A crash mid-run must not LOSE the run, and recovery must resolve
/// it deterministically (identically to the never-crashed reference model),
/// with any cause preserved.
///
/// NOTE on the literal coordinator ask ("recover_expired_leases returns the run
/// to a *claimable* state; never Failed"): the shared turn engine does NOT
/// re-queue an expired lease — `InMemoryTurnStateStore::recover_expired_leases`
/// terminates the abandoned run as `Failed(lease_expired)` (a
/// resumable-checkpointed run keeps its checkpoint and is retryable; a
/// checkpoint-less one does not — see the ignored reproducer below). That is
/// pre-existing engine semantics, identical in the direct authority and the row
/// store, and is unaffected by whether a crash occurred. This suite's charge is
/// row-store *crash-consistency*, so the assertion here is the defensible one:
/// the crash neither loses the run nor diverges its recovered outcome from the
/// model. The lifecycle question (should lease expiry be terminal at all under
/// #6284?) is captured, reproducibly, in the ignored test below.
#[tokio::test]
async fn crash_mid_run_recovers_identically_to_model_and_preserves_cause() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let model = model_store();
    let scope = only_scope();

    // Drive submit + claim on BOTH the row store and the model (same requests).
    let run_id = TurnRunId::new();
    let submit = submit_request(scope.clone(), run_id, "idem-crash-mid-run");
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();
    let claim = ClaimRunRequest {
        runner_id,
        lease_token,
        scope_filter: None,
    };
    {
        let store = open_row_store(Arc::clone(&scoped));
        store
            .submit_turn(
                submit.clone(),
                &AllowAllTurnAdmissionPolicy,
                &InMemoryRunProfileResolver::default(),
            )
            .await
            .unwrap();
        store.claim_next_run(claim.clone()).await.unwrap().unwrap();
        // Crash mid-run — never completed.
        drop(store);
    }
    model
        .submit_turn(
            submit,
            &AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .unwrap();
    model.claim_next_run(claim).await.unwrap().unwrap();

    let recovered = open_row_store(Arc::clone(&scoped));

    // The crashed-mid-run survived (not lost) and is Running — not silently
    // failed by the crash itself.
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("crashed-mid-run must survive recovery, never be lost");
    assert_eq!(
        state.status,
        TurnStatus::Running,
        "the crash itself must not change a mid-run's status"
    );

    // Recovery resolves identically to the never-crashed model.
    assert_recovered_matches_model(&recovered, &model, 0, &["crash mid-run".to_string()]).await;
}

/// #6284 — lease expiry of a checkpoint-less run is RE-DRIVABLE, not a terminal
/// dead-end.
///
/// #6284's error-recoverability contract states a crash / abandoned run must
/// stay re-drivable and that terminal failure is reserved for *genuine*
/// invariants (cancellation, budget, DriverBug) — never a bare crash. A run that
/// was only *claimed* (no loop checkpoint yet) crashed BEFORE its first
/// checkpoint = before BeforeModel = before any side effect, so it is always
/// safe to re-drive. `recover_expired_leases` therefore re-queues it to a
/// claimable state instead of stranding it `Failed(lease_expired)`. (A run that
/// DID reach a resumable checkpoint keeps today's `Failed(lease_expired)` +
/// checkpoint behavior — asserted separately in `retry_failed_turn_store_contract`.)
#[tokio::test]
async fn lease_expiry_requeues_checkpointless_run_as_redrivable() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let run_id = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-strand-repro").await;
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("claim (no checkpoint reached)");
        drop(store);
        run_id
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    recovered
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
            scope_filter: None,
        })
        .await
        .expect("lease recovery");
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("run resolvable");
    // #6284: re-drivable, not a terminal dead-end.
    assert!(
        !state.status.is_terminal(),
        "#6284: a crashed checkpoint-less run must remain re-drivable, got {:?}",
        state.status
    );
    assert_eq!(
        state.status,
        TurnStatus::Queued,
        "a re-drivable expired lease is re-queued to a claimable state"
    );

    // Re-claimable: the scheduler re-drives it. This is NOT a double-claim — the
    // prior lease genuinely expired before the re-queue.
    let reclaimed = recovered
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap();
    assert_eq!(
        reclaimed.map(|claimed| claimed.state.run_id),
        Some(run_id),
        "a re-queued checkpoint-less run must be re-claimable so it re-drives"
    );
}

/// #6284 — the checkpoint-less re-drive loop is BOUNDED by `claim_count`. A run
/// that keeps crashing before its first checkpoint cannot re-drive forever: once
/// `claim_count` reaches `max_crash_recovery_reclaims`, lease expiry terminal-
/// fails it with the genuine-invariant reason `crash_retry_exhausted` (NOT
/// `lease_expired`), so the failure is model-visible and honest.
#[tokio::test]
async fn lease_expiry_crash_retry_bound_fails_with_crash_retry_exhausted() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();
    // Bound of 1: the first claim (claim_count = 1) already reaches the bound, so
    // the next lease expiry terminal-fails instead of re-queuing.
    let open = |scoped: Arc<ScopedFilesystem<FaultBackend>>| {
        FilesystemTurnStateRowStore::new(scoped)
            .with_limits(limits().set_max_crash_recovery_reclaims(1))
            .with_preappend_row_reservations()
    };

    let run_id = {
        let store = open(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-crash-retry-bound").await;
        store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("claim (no checkpoint reached)");
        drop(store);
        run_id
    };

    let recovered = open(Arc::clone(&scoped));
    recovered
        .recover_expired_leases(RecoverExpiredLeasesRequest {
            now: Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap(),
            scope_filter: None,
        })
        .await
        .expect("lease recovery");
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("run resolvable");
    assert_eq!(
        state.status,
        TurnStatus::Failed,
        "at the crash-retry bound a checkpoint-less run terminal-fails"
    );

    let snapshot = recovered.persistence_snapshot().await.unwrap();
    let run = snapshot
        .runs
        .iter()
        .find(|record| record.run_id == run_id)
        .expect("failed run present");
    let failure = run
        .failure
        .as_ref()
        .expect("crash-retry-exhausted run records its genuine-invariant failure");
    assert_eq!(
        failure.category(),
        "crash_retry_exhausted",
        "checkpoint-less crash-retry exhaustion is a genuine invariant, never lease_expired"
    );

    // Terminal → not re-claimable (the bound really did stop the re-drive loop).
    let after = recovered
        .claim_next_run(ClaimRunRequest {
            runner_id: TurnRunnerId::new(),
            lease_token: TurnLeaseToken::new(),
            scope_filter: Some(scope.clone()),
        })
        .await
        .unwrap();
    assert!(
        after.is_none(),
        "a crash-retry-exhausted terminal run must not be claimable, got {after:?}"
    );
}

/// #6284 (IV) — When a run reaches terminal Failed via a genuine invariant, its
/// Failed record AND its model-visible failure detail must survive a crash.
#[tokio::test]
async fn acked_failure_detail_survives_crash() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let run_id = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-failure-detail").await;
        let claimed = store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("claim before fail");
        store
            .fail_run(FailRunRequest {
                run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                failure: SanitizedFailure::new("driver_bug")
                    .unwrap()
                    .with_detail("loop driver produced an invalid transition"),
            })
            .await
            .expect("fail the run");
        drop(store);
        run_id
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    let snapshot = recovered.persistence_snapshot().await.unwrap();
    let run = snapshot
        .runs
        .iter()
        .find(|run| run.run_id == run_id)
        .expect("Failed run must survive crash");
    assert_eq!(run.status, TurnStatus::Failed);
    let failure = run
        .failure
        .as_ref()
        .expect("Failed run must retain its failure record across crash");
    assert_eq!(failure.category(), "driver_bug");
    assert_eq!(
        failure.detail(),
        Some("loop driver produced an invalid transition"),
        "the model-visible failure cause must survive a crash (#6284 b/c)"
    );
    check_internal_invariants(&snapshot).unwrap();
}

/// #6284 (IV, converse) — A crash BEFORE the fail is durable must leave the run
/// re-drivable (so it can fail again and record its cause), never a silent loss
/// of both the run and its failure.
#[tokio::test]
async fn fail_before_durable_leaves_run_redrivable() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope = only_scope();

    let (run_id, runner_id, lease_token) = {
        let store = open_row_store(Arc::clone(&scoped));
        let run_id = submit_one(&store, &scope, "idem-fail-not-durable").await;
        let claimed = store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: None,
            })
            .await
            .unwrap()
            .expect("claim before fail");

        // Fault the fail's durable append — under write-through it must error
        // and NOT durably fail the run.
        backend.fail_next_appends(1);
        let result = store
            .fail_run(FailRunRequest {
                run_id,
                runner_id: claimed.runner_id,
                lease_token: claimed.lease_token,
                failure: SanitizedFailure::new("transient_boom").unwrap(),
            })
            .await;
        backend.disarm();
        assert!(
            result.is_err(),
            "a faulted fail append must not report success, got {result:?}"
        );
        drop(store);
        (run_id, claimed.runner_id, claimed.lease_token)
    };

    let recovered = open_row_store(Arc::clone(&scoped));
    let state = recovered
        .get_run_state(GetRunStateRequest {
            scope: scope.clone(),
            run_id,
        })
        .await
        .expect("run must survive a non-durable fail");
    assert_ne!(
        state.status,
        TurnStatus::Failed,
        "a fail that never became durable must not appear Failed after crash"
    );

    // Re-drivable: the run can be failed AGAIN with the (durably-recorded)
    // lease and this time records its cause — the failure was not silently lost.
    let refailed = recovered
        .fail_run(FailRunRequest {
            run_id,
            runner_id,
            lease_token,
            failure: SanitizedFailure::new("transient_boom")
                .unwrap()
                .with_detail("second attempt records the cause"),
        })
        .await
        .expect("run must be re-drivable (re-failable) after a non-durable fail");
    assert_eq!(refailed.status, TurnStatus::Failed);

    // And the cause is now durable across a further crash.
    drop(recovered);
    let reopened = open_row_store(Arc::clone(&scoped));
    let snapshot = reopened.persistence_snapshot().await.unwrap();
    let run = snapshot
        .runs
        .iter()
        .find(|run| run.run_id == run_id)
        .expect("re-failed run present");
    assert_eq!(run.status, TurnStatus::Failed);
    assert_eq!(
        run.failure.as_ref().and_then(|f| f.detail()),
        Some("second attempt records the cause"),
        "the re-drive's recorded cause must be durable"
    );
    check_internal_invariants(&snapshot).unwrap();
}

/// REGRESSION (#6263) — a no-op `claim_next_run` must not leak a completed run's
/// active lock across a crash.
///
/// Minimal reduction (from chaos seed 101): a `claim_next_run` that matches no
/// queued run — i.e. returns `Ok(None)` — runs through the row store's
/// targeted-delta commit path with an empty durable delta. Before the fix that
/// still advanced the hot-cache journal reservation sequence without a matching
/// backend append, desyncing it by +1: a subsequent `complete_run`'s active-lock
/// DELETE (materialized at the real append seq) then collided with the run's
/// active-lock row (reserved at the desynced, higher seq) and was skipped. The
/// LIVE hot cache correctly showed the lock released (0 locks), but after a crash
/// + recovery the terminal (`Completed`) run still held its active lock (1 lock).
///
/// Impact: active-run exclusivity is keyed on the thread's active lock, so a
/// leaked lock on a terminal run permanently blocks every new turn on that
/// thread after a crash (submits return `ThreadBusy` forever).
///
/// The fix makes an empty durable delta a true no-op that does not advance the
/// reservation sequence (`apply` / `apply_with_targeted_delta` in
/// `crates/ironclaw_turns/src/filesystem_store/row_store`). The assertion below
/// holds after the fix.
#[tokio::test]
async fn noop_claim_does_not_leak_active_lock_across_crash() {
    let backend = Arc::new(FaultBackend::new(InMemoryBackend::new()));
    let scoped = fault_scoped(Arc::clone(&backend));
    let scope_with_run = scopes()[3].clone();
    let empty_scope = scopes()[2].clone();
    let run_id = TurnRunId::new();
    let runner_id = TurnRunnerId::new();
    let lease_token = TurnLeaseToken::new();

    let live_locks = {
        let store = open_row_store(Arc::clone(&scoped));
        store
            .submit_turn(
                submit_request(scope_with_run.clone(), run_id, "idem-noop-claim-repro"),
                &AllowAllTurnAdmissionPolicy,
                &InMemoryRunProfileResolver::default(),
            )
            .await
            .unwrap();
        // The trigger: a claim that matches nothing (empty scope) → Ok(None).
        let nothing = store
            .claim_next_run(ClaimRunRequest {
                runner_id: TurnRunnerId::new(),
                lease_token: TurnLeaseToken::new(),
                scope_filter: Some(empty_scope),
            })
            .await
            .unwrap();
        assert!(
            nothing.is_none(),
            "the empty-scope claim must match nothing"
        );
        // Now genuinely claim then complete the real run.
        store
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: Some(scope_with_run.clone()),
            })
            .await
            .unwrap()
            .expect("claim the queued run");
        store
            .complete_run(CompleteRunRequest {
                run_id,
                runner_id,
                lease_token,
            })
            .await
            .expect("complete the run");
        let live = store.persistence_snapshot().await.unwrap();
        drop(store);
        live.active_locks.len()
    };
    assert_eq!(live_locks, 0, "live hot cache correctly released the lock");

    let recovered = open_row_store(Arc::clone(&scoped));
    let snapshot = recovered.persistence_snapshot().await.unwrap();
    // SHOULD hold; fails today because the completed run's lock leaked durably.
    assert!(
        snapshot.active_locks.is_empty(),
        "completed run must not retain a durable active lock across crash, but found: {:?}",
        snapshot
            .active_locks
            .iter()
            .map(|lock| (lock.run_id.to_string(), lock.status))
            .collect::<Vec<_>>()
    );
}
